//! Interim workholding audit while camlib jobs still start from fresh stock (#121).
//! Cutting envelopes can include preserved material: this warns, never certifies a grip.
use crate::trace::{Trace, because};
use serde_json::{Value, json};

type Point = [f64; 3];
type Bounds = [Point; 2];
const RULE: &str = "machine.clamping";
const SCOPE: &str = "checks stock-face contact bands from stock bottom to jaw top against earlier tooled operations' conservative cutting envelopes, transformed through setup rotation and origin_model; roughing/contour envelopes include preserved material and grouped holes include the space between them; not a stock simulation or proof of face retention, contact area or holding strength; missing frames or geometry are unchecked";

struct Frame { rotation: [[f64; 3]; 3], origin: Point }
impl Frame {
    fn read(setup: &Value) -> Option<Self> {
        Some(Self { rotation: serde_json::from_value(setup["rotation"].clone()).ok()?, origin: serde_json::from_value(setup["origin_model"].clone()).ok()? })
    }
    fn transform(&self, point: Point, to: &Self) -> Point {
        let model: Point = std::array::from_fn(|j| self.origin[j] + (0..3).map(|i| self.rotation[i][j] * point[i]).sum::<f64>());
        std::array::from_fn(|i| (0..3).map(|j| to.rotation[i][j] * (model[j] - to.origin[j])).sum())
    }
}
fn bounds(points: impl IntoIterator<Item = Point>) -> Option<Bounds> {
    let mut points = points.into_iter();
    let first = points.next()?;
    let mut b = [first; 2];
    for p in points {
        for (i, value) in p.into_iter().enumerate() { b[0][i] = b[0][i].min(value); b[1][i] = b[1][i].max(value); }
    }
    Some(b)
}
fn transformed(b: Bounds, from: &Frame, to: &Frame) -> Bounds {
    bounds((0..8).map(|mask| from.transform(std::array::from_fn(|i| b[(mask >> i) & 1][i]), to))).unwrap()
}
fn overlaps(face: Bounds, cut: Bounds) -> bool {
    (0..3).all(|i| {
        if (face[1][i] - face[0][i]).abs() < 1e-6 {
            face[0][i] >= cut[0][i] - 1e-6 && face[0][i] <= cut[1][i] + 1e-6
        } else { face[1][i].min(cut[1][i]) - face[0][i].max(cut[0][i]) > 1e-6 }
    })
}
fn stock(setup: &Value) -> Option<Bounds> {
    Some([serde_json::from_value(setup["stock"]["min"].clone()).ok()?, serde_json::from_value(setup["stock"]["max"].clone()).ok()?])
}
fn xy_points(points: &[Value], pad: f64, low: f64, high: f64) -> Option<Bounds> {
    let mut corners = vec![];
    for p in points {
        let (x, y) = (p[0].as_f64()?, p[1].as_f64()?);
        corners.extend([[x - pad, y - pad, low], [x + pad, y + pad, high]]);
    }
    bounds(corners)
}

/// Bounding envelopes of potentially removed stock, not cutter or holder collisions.
/// No assumed mesh/curve topology: outlines use the whole stock XY bounds.
fn envelope(setup: &Value, op: &Value, resources: &Value) -> Option<Bounds> {
    let mut b = stock(setup)?;
    let tool = resources["tools"].as_array()?.iter().find(|t| t["id"] == op["tool"])?;
    let breakthrough = resources["breakthrough_mm"].as_f64().unwrap_or(1.);
    let floor = |name: &str| op[name].as_f64();
    match op["kind"].as_str()? {
        "facing" | "roughing" | "wall_finish" => b[0][2] = floor("floor_z")?,
        "flats" => b[0][2] = op["levels"].as_array()?.iter().map(Value::as_f64).collect::<Option<Vec<_>>>()?.into_iter().min_by(f64::total_cmp)?,
        "holes" | "counterbore" | "thread" => {
            let kind = op["kind"].as_str()?;
            let bottom = floor(if kind == "counterbore" { "floor_z" } else { "bottom_z" })?;
            let tip = if tool["type"] == "drill" { crate::resources::drill_tip_length(tool).ok()? } else { 0. };
            let low = (bottom - breakthrough - tip).max(b[0][2]);
            return xy_points(op["centers"].as_array()?, op[if kind == "thread" { "major" } else { "diameter" }].as_f64()? / 2., low, b[1][2]);
        }
        "slot" | "notch" | "v_channel" => {
            let kind = op["kind"].as_str()?;
            let low = floor(if kind == "v_channel" { "root_z" } else { "floor_z" })?;
            let low = if op["bottom"] == "through" { (low - breakthrough).max(b[0][2]) } else { low };
            if kind == "notch" { return xy_points(op["outer"].as_array()?, 0., low, b[1][2]); }
            return xy_points(&[op["a"].clone(), op["b"].clone()], op["width"].as_f64()? / 2., low, b[1][2]);
        }
        "edge_chamfer" => {
            let half = (tool["angle"].as_f64()? / 2.).to_radians().tan();
            b[0][2] = floor("top_z")? - (op["width"].as_f64()? + crate::resources::TIP_CLEARANCE) / half;
        }
        _ => return None,
    }
    b[0][2] = b[0][2].max(setup["stock"]["min"][2].as_f64()?);
    Some(b)
}

fn bands(setup: &Value) -> Option<Vec<Bounds>> {
    let stock = stock(setup)?;
    let c = &setup["clamping"];
    let axis = match c["faces"][0].as_str()? { "stock.x.min" => 0, "stock.y.min" => 1, _ => return None };
    let along = 1 - axis;
    let mut bands = vec![];
    for (n, jaw) in c["jaws"].as_array()?.iter().enumerate() {
        let mut b = stock;
        b[0][axis] = stock[n][axis];
        b[1][axis] = stock[n][axis];
        b[0][along] = b[0][along].max(jaw["min"][along].as_f64()?);
        b[1][along] = b[1][along].min(jaw["max"][along].as_f64()?);
        if let Some(z) = c["contact_min_z"].as_f64() { b[0][2] = z; }
        if let Some(a) = c["contact_along_min"].as_f64() { b[0][along] = b[0][along].max(a); }
        if let Some(a) = c["contact_along_max"].as_f64() { b[1][along] = b[1][along].min(a); }
        b[1][2] = c["jaw_top_z"].as_f64()?;
        bands.push(b);
    }
    Some(bands)
}

/// Check the actual retained contact polygons against finished contour segments.
/// A notch can divide a side into several segments; no segment need span air.
fn prepared_contacts(setup: &Value, earlier: &Value) -> bool {
    let seat=&setup["flip_seating"];
    let axis=seat["axis"].as_u64().unwrap() as usize;
    let along=1-axis;
    let eps=crate::rules::workholding()["flip_frame_tolerance_mm"].as_f64().unwrap();
    let Some(frame)=Frame::read(earlier) else {return false;};
    let Ok(sides)=serde_json::from_value::<Vec<Vec<Vec<Point>>>>(seat["contact_patches_model"].clone()) else {return false;};
    sides.iter().all(|patches| !patches.is_empty() && patches.iter().all(|patch| {
        let points: Vec<Point>=patch.iter().map(|p| std::array::from_fn(|i|(0..3).map(|j|frame.rotation[i][j]*(p[j]-frame.origin[j])).sum())).collect();
        let low=points.iter().map(|p|p[2]).fold(f64::INFINITY,f64::min);
        let side=points[0][axis];
        let mut intervals=vec![];
        for op in earlier["operations"].as_array().into_iter().flatten().filter(|o|o["kind"]=="wall_finish" && o["tool"].is_string() && o["floor_z"].as_f64().is_some_and(|z|z<=low+eps)) {
            if let Some(outer)=op["outer"].as_array() {for i in 0..outer.len() {
                let a=&outer[i];let b=&outer[(i+1)%outer.len()];
                if a[2].as_f64()==Some(0.) && a[axis].as_f64().is_some_and(|v|(v-side).abs()<=eps) && b[axis].as_f64().is_some_and(|v|(v-side).abs()<=eps)
                    && let Some((a,b))=a[along].as_f64().zip(b[along].as_f64()) {intervals.push([a.min(b)-eps,a.max(b)+eps]);}
            }}
        }
        intervals.sort_by(|a,b|a[0].total_cmp(&b[0]));
        let mut merged: Vec<[f64;2]>=vec![];
        for span in intervals {if let Some(last)=merged.last_mut().filter(|last|last[1]>=span[0]) {last[1]=last[1].max(span[1]);} else {merged.push(span);}}
        let poly: Vec<_>=points.iter().map(|p|[p[along],p[2]]).collect();
        let want=crate::flip::area(&poly);
        let covered: f64=merged.iter().map(|span|crate::flip::area(&crate::flip::clipped(poly.clone(),[[span[0],f64::NEG_INFINITY],[span[1],f64::INFINITY]]))).sum();
        want>0. && covered+eps>=want
    }))
}

/// Positive evidence for the actual support plane, independent of cutting
/// envelopes: absence of cuts is never evidence of prepared seating or faces.
fn established_seat(setup: &Value, earlier: &Value) -> bool {
    let support=setup["flip_seating"]["seat_z_model"].as_f64()
        .zip(earlier["origin_model"][2].as_f64()).map(|(seat,origin)|seat-origin);
    let eps=crate::rules::workholding()["flip_frame_tolerance_mm"].as_f64().unwrap();
    earlier["operations"].as_array().into_iter().flatten().any(|op|
        op["tool"].is_string() && support.is_some_and(|support|
            (op["kind"]=="facing" && op["floor_z"].as_f64().is_some_and(|z|(z-support).abs()<eps))
            || (op["kind"]=="flats" && op["levels"].as_array().into_iter().flatten()
                .any(|z|z.as_f64().is_some_and(|z|(z-support).abs()<eps)))))
}

fn spot_envelope(op: &Value) -> Option<Bounds> {
    let corners: Option<Vec<_>> = op["spots"].as_array()?.iter().map(|s| {
        let x=s["center"][0].as_f64()?;
        let y=s["center"][1].as_f64()?;
        let z=s["top_z"].as_f64()?;
        let radius=s["diameter"].as_f64()?/2.;
        let depth=s["depth"].as_f64()?;
        Some([[x-radius,y-radius,z-depth],[x+radius,y+radius,z]])
    }).collect();
    bounds(corners?.into_iter().flatten())
}

/// Called after settlement, exclusions and tool resolution, so absent/untooled operations
/// do not invent earlier cuts. The final process retry recomputes this audit after refusals.
pub fn check(plan: &mut Value, resources: &Value, trace: &mut Trace) {
    let setups = plan["setups"].as_array().cloned().unwrap_or_default();
    for (index, setup) in setups.iter().enumerate().skip(1).filter(|(_, s)| s["clamping"].is_object()) {
        let id = setup["id"].as_str().unwrap_or("");
        let mut hits = vec![];
        let mut unchecked = vec![];
        let flip=setup["flip_seating"].is_object();
        let top=setups[..index].iter().find(|s|s["id"]=="top");
        let prepared=flip && top.is_some_and(|s|prepared_contacts(setup,s));
        let seated=flip && top.is_some_and(|s|established_seat(setup,s));
        // Check these even when top has no retained cuts, or only cuts that
        // never touch the bands. A clean overlap audit cannot prepare a grip.
        if flip && !prepared {
            unchecked.push(json!({"setup":"top","reason":"finished grip faces are not established by retained tooled wall finishing"}));
        }
        if flip && !seated {
            unchecked.push(json!({"setup":"top","reason":"finished edge seating is not established by retained facing/flats"}));
        }
        for earlier in &setups[..index] {
            let cuts: Vec<_> = earlier["operations"].as_array().into_iter().flatten().enumerate().filter(|(_, o)| o["tool"].is_string()).collect();
            if cuts.is_empty() { continue; }
            let (Some(from), Some(to), Some(bands)) = (Frame::read(setup), Frame::read(earlier), bands(setup)) else {
                unchecked.push(json!({"setup": earlier["id"], "reason": "missing setup frame or grip band geometry"}));
                continue;
            };
            let bands: Vec<_> = bands.into_iter().map(|b| transformed(b, &from, &to)).collect();
            // Only this option's mesh-proven contact faces can treat retained
            // outside finishing as preparation, rather than removed material.
            // Still check holes/notches/slots and all other setups normally.
            let flip_top = setup["flip_seating"].is_object() && earlier["id"] == "top";
            for (n, op) in cuts {
                if flip_top && prepared && (matches!(op["kind"].as_str(),Some("roughing" | "flats" | "wall_finish")) || crate::flip::modeled_cut(&setup["flip_seating"],op)) { continue; }
                let operation = format!("{}-{}-{}", earlier["id"].as_str().unwrap_or(""), n + 1, op["kind"].as_str().unwrap_or(""));
                let cut = if setup["flip_seating"].is_object() && op["kind"] == "spot_drill" {
                    spot_envelope(op)
                } else { envelope(earlier, op, resources) };
                let Some(cut) = cut else {
                    unchecked.push(json!({"operation": operation, "reason": "missing or unsupported cutting geometry"}));
                    continue;
                };
                let faces: Vec<_> = bands.iter().enumerate().filter(|(_, band)| overlaps(**band, cut)).map(|(n, _)| setup["clamping"]["faces"][n].clone()).collect();
                if !faces.is_empty() { hits.push(json!({"operation": operation, "faces": faces})); }
            }
        }
        let status = if !hits.is_empty() { "possible_overlap" } else if !unchecked.is_empty() { "unchecked" } else { "no_overlap" };
        let mut detail = json!({"status": status, "scope": SCOPE, "overlaps": hits, "unchecked": unchecked});
        if flip {
            detail["prepared_contacts"]=json!(prepared);
            detail["established_seat"]=json!(seated);
        }
        let says = match status {
            "possible_overlap" => format!("setup {id}: earlier cuts may have removed material in the later grip band: {}; {SCOPE}", detail["overlaps"]),
            "unchecked" => format!("setup {id}: later grip band could not be checked completely against earlier cuts: {}; {SCOPE}", detail["unchecked"]),
            _ => format!("setup {id}: no overlap found between later grip band and checked earlier cuts; {SCOPE}"),
        };
        // Store the audit in the actual given/chosen clamping decision as well as its warning.
        let primary = setup["clamping"]["because"][0].as_str().unwrap_or("");
        if let Some(d) = trace.decisions.iter_mut().find(|d| d["id"] == primary) {
            d["chosen"]["prior_setups"] = detail.clone();
            if flip {
                let ready=prepared && seated && status=="no_overlap";
                let conclusion=if ready {"finished grip faces and edge seating confirmed by retained top operations"}
                    else {"grip preparation not established; flip clamping is unavailable"};
                d["says"]=json!(format!("{}; {conclusion}",d["says"].as_str().unwrap_or("")));
                if !ready {d["outcome"]=json!("none");}
            }
        }
        let decision = if status == "no_overlap" {
            trace.decide(RULE, id, "chosen", &says, json!({"chosen": detail}))
        } else { trace.warn(RULE, id, says) };
        because(&mut plan["setups"][index]["clamping"], [decision]);
        if flip && (!prepared || !seated || status != "no_overlap") {
            let b=setup["stock"]["max"][2].as_f64().unwrap_or(0.);
            let reason = format!("bottom flip setup withheld: retained Op 1 operations do not establish checked finished grip faces and edge seating; the part remains on {b} mm of sacrificial bottom stock, with the outline and through features stopping in it; face it off by hand or another setup if not replanned");
            trace.warn(RULE, id, reason.clone());
            for feature in setup["operations"].as_array().into_iter().flatten()
                .flat_map(|o| o["features"].as_array().into_iter().flatten()).filter_map(Value::as_str) {
                trace.skip(RULE, feature, reason.clone());
            }
            plan["setups"][index]["operations"] = json!([]);
        }
    }
}
