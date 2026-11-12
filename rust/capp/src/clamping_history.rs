//! Certain finished-part contact, with a conservative prior-cut audit for uncovered evidence.
use crate::kind::OpKind;
use crate::model::{Cut, Operation, Plan, Point2, Setup, SpotDrill};
use crate::trace::{Trace, because};
use serde_json::{Value, json};

type Point = [f64; 3];
type Bounds = [Point; 2];
const RULE: &str = "machine.clamping";
const SCOPE: &str = "checks stock-face contact bands from stock bottom to jaw top against earlier tooled operations' conservative cutting envelopes, transformed through setup rotation and origin_model; roughing/contour envelopes include preserved material and grouped holes include the space between them; not a stock simulation or proof of face retention, contact area or holding strength; missing frames or geometry are unchecked";

struct Frame { rotation: [[f64; 3]; 3], origin: Point }
impl Frame {
    fn read(setup: &Setup) -> Option<Self> {
        Some(Self { rotation: setup.rotation?.map(|row| row.map(f64::from)), origin: setup.origin_model? })
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
fn stock(setup: &Setup) -> Bounds {
    [setup.stock.min, setup.stock.max]
}
fn xy_points(points: impl IntoIterator<Item = Point2>, pad: f64, low: f64, high: f64) -> Option<Bounds> {
    let mut corners = vec![];
    for [x, y] in points {
        corners.extend([[x - pad, y - pad, low], [x + pad, y + pad, high]]);
    }
    bounds(corners)
}

/// Bounding envelopes of potentially removed stock, not cutter or holder collisions.
/// No assumed mesh/curve topology: outlines use the whole stock XY bounds.
fn envelope(setup: &Setup, op: &Operation, resources: &Value) -> Option<Bounds> {
    let mut b = stock(setup);
    let id = op.tool.as_ref()?.as_deref()?;
    let tool = resources["tools"].as_array()?.iter().find(|t| t["id"] == id)?;
    let breakthrough = resources["breakthrough_mm"].as_f64().unwrap_or(1.);
    let tip = || if tool["type"] == "drill" { crate::resources::drill_tip_length(tool).ok() } else { Some(0.) };
    let drilled = move |bottom: f64, tip: f64, centers: &[Point2], diameter: f64| {
        let low = (bottom - breakthrough - tip).max(b[0][2]);
        xy_points(centers.iter().copied(), diameter / 2., low, b[1][2])
    };
    let through = move |low: f64, bottom: &str| if bottom == "through" { (low - breakthrough).max(b[0][2]) } else { low };
    match &op.cut {
        Cut::Facing { floor_z } => b[0][2] = *floor_z,
        Cut::Roughing(c) => b[0][2] = c.floor_z,
        Cut::WallFinish(c) => b[0][2] = c.floor_z,
        Cut::Flats { levels, .. } => b[0][2] = levels.iter().copied().min_by(f64::total_cmp)?,
        Cut::Holes(c) => return drilled(c.bottom_z, tip()?, &c.centers, c.diameter),
        Cut::Counterbore(c) => return drilled(c.floor_z, tip()?, &c.centers, c.diameter),
        Cut::Thread(c) => return drilled(c.bottom_z, tip()?, &c.centers, c.major),
        Cut::Slot(c) => return xy_points([c.a, c.b], c.width / 2., through(c.floor_z, &c.bottom), b[1][2]),
        Cut::Notch(c) => return xy_points(c.outer.iter().map(|p| [p[0], p[1]]), 0., through(c.floor_z, &c.bottom), b[1][2]),
        Cut::VChannel(c) => return xy_points([c.a, c.b], c.width / 2., c.root_z, b[1][2]),
        Cut::EdgeChamfer(c) => {
            let half = (tool["angle"].as_f64()? / 2.).to_radians().tan();
            b[0][2] = c.top_z - (c.width + crate::resources::TIP_CLEARANCE) / half;
        }
        Cut::SpotDrill(_) => return None,
    }
    b[0][2] = b[0][2].max(setup.stock.min[2]);
    Some(b)
}

fn bands(setup: &Setup) -> Option<Vec<Bounds>> {
    let stock = stock(setup);
    let c = setup.clamping.as_ref()?;
    let axis = match c["faces"][0].as_str()? { "stock.x.min" => 0, "stock.y.min" => 1, _ => return None };
    let along = 1 - axis;
    let mut bands = vec![];
    for (n, jaw) in c["jaws"].as_array()?.iter().enumerate() {
        let mut b = stock;
        let plane = jaw[if n==0 {"max"} else {"min"}][axis].as_f64()?;
        b[0][axis] = plane;
        b[1][axis] = plane;
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
fn prepared_contacts(setup: &Setup, earlier: &Setup) -> bool {
    let seat=setup.flip_seating.as_ref().unwrap_or(&Value::Null);
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
        for wall in earlier.operations.iter().filter(|o| matches!(o.tool, Some(Some(_))))
            .filter_map(|o| match &o.cut { Cut::WallFinish(w) => Some(w), _ => None }).filter(|w| w.floor_z<=low+eps) {
            let outer=&wall.outer;
            for i in 0..outer.len() {
                let a=outer[i];let b=outer[(i+1)%outer.len()];
                if a[2]==0. && (a[axis]-side).abs()<=eps && (b[axis]-side).abs()<=eps {
                    let (a,b)=(a[along],b[along]);
                    intervals.push([a.min(b)-eps,a.max(b)+eps]);
                }
            }
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
fn established_seat(setup: &Setup, earlier: &Setup) -> bool {
    let support=setup.flip_seating.as_ref().and_then(|s| s["seat_z_model"].as_f64())
        .zip(earlier.origin_model.map(|o| o[2])).map(|(seat,origin)|seat-origin);
    let eps=crate::rules::workholding()["flip_frame_tolerance_mm"].as_f64().unwrap();
    earlier.operations.iter().any(|op|
        matches!(op.tool, Some(Some(_))) && support.is_some_and(|support| match &op.cut {
            Cut::Facing { floor_z } => (floor_z-support).abs()<eps,
            Cut::Flats { levels, .. } => levels.iter().any(|z|(z-support).abs()<eps),
            _ => false,
        }))
}

fn spot_envelope(op: &SpotDrill) -> Option<Bounds> {
    let corners: Option<Vec<_>> = op.spots.iter().map(|s| {
        let [x, y] = s.center;
        let z=s.top_z;
        let radius=s.diameter/2.;
        let depth=s.depth?;
        Some([[x-radius,y-radius,z-depth],[x+radius,y+radius,z]])
    }).collect();
    bounds(corners?.into_iter().flatten())
}

/// The final part is a lower bound on retained material. Carried height fields
/// are upper bounds and deliberately contribute no contact. Only planar faces
/// coincident with the jaw plane count; clip triangles, never their envelopes.
fn certain_contacts(plan: &Plan, setup: &Setup, earlier: &[Setup], resources: &Value) -> Option<Vec<f64>> {
    let frame = Frame::read(setup)?;
    let bands = bands(setup)?;
    let tris: Vec<[Point; 3]> = serde_json::from_value(plan.contact_surface_model.clone()?).ok()?;
    let mut cuts = vec![];
    for previous in earlier {
        let from = Frame::read(previous)?;
        for op in previous.operations.iter().filter(|o| matches!(o.tool, Some(Some(_)))) {
            // These operations preserve the target surface. Other operations
            // may cut beyond its mesh (threads, drill tips, edge breaks).
            if preserves(op) {continue;}
            let cut = if let Cut::SpotDrill(spot) = &op.cut {spot_envelope(spot)} else {envelope(previous, op, resources)}?;
            cuts.push(transformed(cut, &from, &frame));
        }
    }
    Some(bands.iter().map(|band| {
        let axis = if (band[1][0]-band[0][0]).abs() < 1e-7 {0} else {1};
        let along = 1-axis;
        tris.iter().filter_map(|tri| {
            let points: [Point;3] = tri.map(|p| std::array::from_fn(|i| (0..3).map(|j| frame.rotation[i][j]*(p[j]-frame.origin[j])).sum()));
            if !points.iter().all(|p| (p[axis]-band[0][axis]).abs() <= 1e-7) {return None;}
            let polygon = points.iter().map(|p| [p[along],p[2]]).collect();
            let mut patches = vec![crate::flip::clipped(polygon, [[band[0][along],band[0][2]],[band[1][along],band[1][2]]])];
            for cut in cuts.iter().filter(|cut| band[0][axis] >= cut[0][axis]-1e-7 && band[0][axis] <= cut[1][axis]+1e-7) {
                let (x0,x1,y0,y1) = (cut[0][along],cut[1][along],cut[0][2],cut[1][2]);
                let rectangles = [
                    [[f64::NEG_INFINITY,f64::NEG_INFINITY],[x0,f64::INFINITY]],
                    [[x1,f64::NEG_INFINITY],[f64::INFINITY,f64::INFINITY]],
                    [[x0,f64::NEG_INFINITY],[x1,y0]],
                    [[x0,y1],[x1,f64::INFINITY]],
                ];
                patches = patches.into_iter().flat_map(|p| rectangles.map(|rect| crate::flip::clipped(p.clone(),rect))).collect();
                patches.retain(|p| crate::flip::area(p)>1e-8);
            }
            Some(patches.iter().map(|p| crate::flip::area(p)).sum::<f64>())
        }).sum::<f64>()
    }).collect())
}

/// Roughing, flats, wall finishing and facing: the operations that keep to the part's surface.
fn preserves(op: &Operation) -> bool {
    matches!(op.cut.kind(), OpKind::Roughing | OpKind::Flats | OpKind::WallFinish | OpKind::Facing)
}

/// Called after settlement, exclusions and tool resolution, so absent/untooled operations
/// do not invent earlier cuts. The final process retry recomputes this audit after refusals.
pub fn check(plan: &mut Plan, resources: &Value, trace: &mut Trace) {
    // Read the setups as they stand before this pass changes any of them.
    let setups = plan.setups.clone();
    for (index, setup) in setups.iter().enumerate().skip(1).filter(|(_, s)| s.clamping.as_ref().is_some_and(Value::is_object)) {
        let id = setup.id.as_str();
        let contacts = certain_contacts(plan, setup, &setups[..index], resources);
        let certain = contacts.as_ref().is_some_and(|a| a.len() == 2 && a.iter().all(|a| *a > 1e-8));
        let no_contact = contacts.as_ref().is_some_and(|a| a.iter().any(|a| *a <= 1e-8));
        let contact_says = match &contacts {
            Some(a) => format!("certain part-face contact per jaw: {:.4} mm², {:.4} mm²; carried upper-bound stock contributes no contact", a[0], a[1]),
            None if !plan.contact_surface_model.as_ref().is_some_and(Value::is_array) => "contact unknown: no part contact model; conservative envelope audit retained".into(),
            None => "contact unknown: setup frame, grip band or prior-cut geometry unavailable; conservative envelope audit retained".into(),
        };
        let mut hits = vec![];
        let mut unchecked = vec![];
        let flip=setup.flip_seating.as_ref().is_some_and(Value::is_object);
        let top=setups[..index].iter().find(|s|s.id=="top");
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
            let cuts: Vec<_> = earlier.operations.iter().enumerate().filter(|(_, o)| matches!(o.tool, Some(Some(_)))).collect();
            if cuts.is_empty() { continue; }
            let (Some(from), Some(to), Some(bands)) = (Frame::read(setup), Frame::read(earlier), bands(setup)) else {
                unchecked.push(json!({"setup": earlier.id, "reason": "missing setup frame or grip band geometry"}));
                continue;
            };
            let bands: Vec<_> = bands.into_iter().map(|b| transformed(b, &from, &to)).collect();
            // Only this option's mesh-proven contact faces can treat retained
            // outside finishing as preparation, rather than removed material.
            // Still check holes/notches/slots and all other setups normally.
            // The mesh-proven contact itself survives conservative envelopes
            // that include the finished part. Seating/preparation remain separate checks.

            let flip_top = flip && earlier.id == "top";
            let seat = setup.flip_seating.as_ref().unwrap_or(&Value::Null);
            for (n, op) in cuts {
                if certain && (!flip || prepared) && preserves(op) {continue;}
                if flip_top && prepared && (matches!(op.cut.kind(), OpKind::Roughing | OpKind::Flats | OpKind::WallFinish) || crate::flip::modeled_cut(seat, op)) { continue; }
                let operation = format!("{}-{}-{}", earlier.id, n + 1, op.cut.kind().name());
                let cut = if flip && let Cut::SpotDrill(spot) = &op.cut {
                    spot_envelope(spot)
                } else { envelope(earlier, op, resources) };
                let Some(cut) = cut else {
                    unchecked.push(json!({"operation": operation, "reason": "missing or unsupported cutting geometry"}));
                    continue;
                };
                let faces: Vec<_> = bands.iter().enumerate().filter(|(_, band)| overlaps(**band, cut)).map(|(n, _)| setup.clamping.as_ref().unwrap_or(&Value::Null)["faces"][n].clone()).collect();
                if (!certain || flip) && !faces.is_empty() { hits.push(json!({"operation": operation, "faces": faces})); }
            }
        }
        let status = if !hits.is_empty() { "possible_overlap" } else if !unchecked.is_empty() { "unchecked" } else { "no_overlap" };
        let mut detail = json!({"contact_area_mm2": contacts, "contact_evidence": contact_says, "status": status, "scope": if certain {"planar finished-part faces clipped to each jaw band, minus earlier drill/thread/edge cutting envelopes; carried height-field stock is not a lower bound; contact area does not prove holding strength or seating"} else {SCOPE}, "overlaps": hits, "unchecked": unchecked});
        if flip {
            detail["prepared_contacts"]=json!(prepared);
            detail["established_seat"]=json!(seated);
        }
        let says = match status {
            "possible_overlap" => format!("setup {id}: earlier cuts may have removed material in the later grip band: {}; {SCOPE}", detail["overlaps"]),
            "unchecked" => format!("setup {id}: later grip band could not be checked completely against earlier cuts: {}; {SCOPE}", detail["unchecked"]),
            _ => format!("setup {id}: no overlap found between later grip band and checked earlier cuts; {SCOPE}"),
        };
        let says = if certain && status == "no_overlap" {format!("setup {id}: {contact_says}; final part faces prove retained contact; seating and holding strength are separate checks")}
            else if certain {format!("{says}; {contact_says}")}
            else if no_contact {format!("{says}; {contact_says}; grip refused: either jaw has no certain contact")}
            else {format!("{says}; {contact_says}")};
        let clamping = plan.setups[index].clamping.as_mut().expect("the setup has clamping");
        clamping["contact_area_mm2"] = json!(contacts);
        // Store the audit in the actual given/chosen clamping decision as well as its warning.
        let primary = setup.clamping.as_ref().unwrap_or(&Value::Null)["because"][0].as_str().unwrap_or("");
        if let Some(d) = trace.decisions.iter_mut().find(|d| d["id"] == primary) {
            d["chosen"]["prior_setups"] = detail.clone();
            d["chosen"]["contact_area_mm2"] = json!(contacts);
            d["says"] = json!(format!("{}; {contact_says}", d["says"].as_str().unwrap_or("")));
            if no_contact { d["outcome"] = json!("none"); }
            if flip {
                let ready=!no_contact && prepared && seated && status=="no_overlap";
                let conclusion=if ready {"finished grip faces and edge seating confirmed by retained top operations"}
                    else {"grip preparation not established; flip clamping is unavailable"};
                d["says"]=json!(format!("{}; {conclusion}",d["says"].as_str().unwrap_or("")));
                if !ready {d["outcome"]=json!("none");}
            }
        }
        let decision = if status == "no_overlap" {
            trace.decide(RULE, id, if no_contact {"none"} else {"chosen"}, &says, json!({"chosen": detail}))
        } else { trace.warn(RULE, id, says, json!({})) };
        because(clamping, [decision]);
        if no_contact || (flip && (!prepared || !seated || status != "no_overlap")) {
            let b=setup.stock.max[2];
            let reason = if no_contact {format!("setup {id} withheld: either jaw has no certain contact; {contact_says}")} else {format!("bottom flip setup withheld: retained Op 1 operations do not establish checked finished grip faces and edge seating; the part remains on {b} mm of sacrificial bottom stock, with the outline and through features stopping in it; face it off by hand or another setup if not replanned")};
            trace.warn(RULE, id, reason.clone(), json!({}));
            for feature in setup.operations.iter().flat_map(|o| o.cut.features().into_iter().flatten()) {
                trace.skip(RULE, feature, reason.clone());
            }
            plan.setups[index].operations.clear();
        }
    }
}
