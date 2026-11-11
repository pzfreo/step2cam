//! Stock-face clamping in each setup's own frame. No inferred machine dimensions.
use crate::{plan::{r, show}, trace::Trace};
use serde_json::{Value, json};

const RULE: &str = "machine.clamping";

/// Records the chosen clamping, or returns why the setup cannot be held. Missing jaw
/// dimensions retain the old opening warning and explicitly skip geometric checks.
pub fn clamp(setup: &mut Value, vise: Option<&Value>, proposal: Option<&Value>, trace: &mut Trace) -> Option<String> {
    let id = setup["id"].as_str().unwrap_or("").to_string();
    setup.as_object_mut()?.remove("clamping");
    let mut lo: [f64; 3] = serde_json::from_value(setup["stock"]["min"].clone()).ok()?;
    let mut hi: [f64; 3] = serde_json::from_value(setup["stock"]["max"].clone()).ok()?;
    let seating = setup.get("flip_seating").cloned();
    if let Some(seat) = &seating {
        lo[2] = r(setup["origin_model"][2].as_f64()? - seat["seat_z_model"].as_f64()?);
        // Grip limits refer to real finished plate material, excluding the boss
        // below its edge seating and the sacrificial material above its bottom.
        hi[2] = 0.;
    }
    let given = match proposal.map(Proposal::read).transpose() {
        Ok(given) => given,
        Err(e) => return Some(format!("invalid clamping override: {e}")),
    };
    let axis = given.as_ref().map_or_else(|| seating.as_ref().map_or_else(|| usize::from(hi[0] - lo[0] > hi[1] - lo[1]), |s| s["axis"].as_u64().unwrap() as usize), |p| p.axis);
    let along = 1 - axis;
    let across = hi[axis] - lo[axis];
    let opening = vise.and_then(|v| v["max_jaw_opening_mm"].as_f64());
    let dimensions = vise.and_then(|v| v["jaw_width_mm"].as_f64().zip(v["jaw_height_mm"].as_f64()).zip(opening));
    let Some(((width, height), opening)) = dimensions else {
        if given.is_some() { return Some("clamping override cannot be validated without vise jaw width, height and opening".into()); }
        trace.decide(RULE, &id, "skipped", "vise jaw width, height or opening absent: clamping check and fixture scene skipped; no dimensions assumed", Value::Null);
        if let Some(opening) = opening.filter(|v| across > *v) {
            trace.warn("machine.workholding", &id, format!("setup {id}: the stock is {} mm across at its narrowest, wider than the vise opens ({} mm): hold it another way", show(across), show(opening)), json!({}));
        }
        return None;
    };
    if ![width, height, opening].iter().all(|v| v.is_finite() && *v > 0.) {
        return Some("vise dimensions must be positive finite numbers".into());
    }
    if across > opening {
        return Some(format!("stock is {} mm across the gripped {} axis, wider than the vise opens ({} mm)", show(across), ["X", "Y"][axis], show(opening)));
    }
    let rule = crate::rules::workholding();
    let stock_height = hi[2] - lo[2];
    let minimum = r(rule["min_grip_mm"].as_f64().unwrap().max(stock_height * rule["min_grip_fraction"].as_f64().unwrap()));
    // An explicit null disables inherited parallels; an absent field inherits the vise's.
    let mut parallel = if proposal.is_some_and(|p| p.get("parallels_height_mm").is_some()) {
        given.as_ref().and_then(|p| p.parallels_height_mm)
    } else { vise.and_then(|v| v["parallels_height_mm"].as_f64()) };
    if parallel.is_none() && !proposal.is_some_and(|p| p.get("parallels_height_mm").is_some())
        && let Some(heights) = vise.and_then(|v| v["parallels_heights_mm"].as_array()) {
            let maximum = stock_height * rule["max_grip_fraction"].as_f64().unwrap();
            parallel = heights.iter().filter_map(Value::as_f64).filter(|p| {
                let grip = height-p;
                grip >= minimum && grip <= maximum && grip <= height
                    && seating.as_ref().is_none_or(|s| (grip-s["grip_height_mm"].as_f64().unwrap()).abs()<1e-4)
            }).max_by(f64::total_cmp);
            if parallel.is_none() {return Some("no available parallels height satisfies grip limits and seating".into());}
            trace.decide(RULE, &id, "chosen", "choose the tallest available parallels satisfying grip limits and mesh seating (least hidden side material)", json!({"chosen":{"parallels_height_mm":parallel},"sources":vise.map(|v| &v["_sources"])}));
    }
    if let Some(seat) = &seating {
        if given.is_some() { return Some("flip seating is mesh-validated; clamping override requires withholding the flip option".into()); }
        if parallel.is_some_and(|p| (height-p-seat["grip_height_mm"].as_f64().unwrap()).abs()>1e-4) {
            return Some("parallels height disagrees with mesh seating grip".into());
        }
    }
    if parallel.is_some_and(|p| !p.is_finite() || p <= 0.) { return Some("parallels height must be positive and finite".into()); }
    let grip = r(given.as_ref().map_or_else(|| parallel.map_or(minimum, |p| height - p), |p| p.grip_height_mm));
    if !grip.is_finite() { return Some("grip height must be finite".into()); }
    if given.is_some() && parallel.is_some_and(|p| r(height - p) != grip) {
        return Some(format!("given grip {} mm disagrees with {} mm grip set by {} mm parallels and {} mm jaws", show(grip), show(r(height - parallel.unwrap())), show(parallel.unwrap()), show(height)));
    }
    let clearance = rule["clearance_mm"].as_f64().unwrap();
    // The faced top must also stand clear, not just the uncut stock top.
    let part_top = setup["operations"].as_array().into_iter().flatten().find(|o| o["kind"] == "facing").and_then(|o| o["floor_z"].as_f64()).unwrap_or(hi[2]);
    let source = parallel.map_or(String::new(), |p| format!(" set by {} mm parallels", show(p)));
    if grip < minimum {
        return Some(format!("grip {} mm{source} is below minimum grip {} mm{}", show(grip), show(minimum), if parallel.is_some() { "; use shorter parallels" } else { "" }));
    }
    if grip > height {
        return Some(format!("grip {} mm exceeds jaw height {} mm", show(grip), show(height)));
    }
    let maximum = r(stock_height * rule["max_grip_fraction"].as_f64().unwrap());
    if grip > maximum {
        return Some(format!("grip {} mm{source} exceeds maximum grip {} mm ({}% of {} mm stock height){}", show(grip), show(maximum), show(r(100. * rule["max_grip_fraction"].as_f64().unwrap())), show(r(stock_height)), if parallel.is_some() { "; use taller parallels" } else { "" }));
    }
    let required_top = r(lo[2] + grip + clearance);
    if part_top <= required_top {
        return Some(format!("part top Z{} does not clear jaw top Z{} plus {} mm side-cut clearance: must be above Z{}", show(part_top), show(r(lo[2] + grip)), show(clearance), show(required_top)));
    }
    let top = r(lo[2] + grip);
    // A side setup grips the plate where the jaws actually reach, not the
    // whole-part box (a remote boss can make that box much thicker).
    if seating.is_none() && let Some(bounds) = band_bounds(setup, axis, lo[2], top) {
        lo[axis] = bounds[0];
        hi[axis] = bounds[1];
    }

    let center = (lo[along] + hi[along]) / 2.;
    let (contact_min, contact_max) = if let Some(seat) = &seating {
        let a = seat["along_min_model"].as_f64()?;
        let b = seat["along_max_model"].as_f64()?;
        let origin = setup["origin_model"][along].as_f64()?;
        if along == 1 { (r(origin-b),r(origin-a)) } else { (r(a-origin),r(b-origin)) }
    } else { (lo[along],hi[along]) };
    let mut jaws = vec![];
    for (n, side) in [lo[axis] - height, hi[axis]].iter().enumerate() {
        let mut min = [0., 0., top - height];
        let mut max = [0., 0., top];
        min[axis] = *side;
        max[axis] = side + height;
        min[along] = center - width / 2.;
        max[along] = center + width / 2.;
        jaws.push(json!({"id": format!("{id}-jaw-{}", n + 1), "min": min.map(r), "max": max.map(r)}));
    }
    let mut parallels = vec![];
    if let Some(p) = parallel {
        // Two support bars at the gripped edges. The rule minimum is their envelope width;
        // no hidden support is omitted from verification when parallels are requested.
        let across = hi[axis]-lo[axis];
        let bar = seating.as_ref().and_then(|s|s["support_width_mm"].as_f64())
            .unwrap_or_else(||rule["min_grip_mm"].as_f64().unwrap().min(across / 3.));
        for (n, side) in [lo[axis], hi[axis] - bar].iter().enumerate() {
            let mut min = lo;
            let mut max = hi;
            let index = if seating.is_some() && axis==1 {1-n} else {n};
            let offset=seating.as_ref().and_then(|s|s["supports"][index]["offset_mm"].as_f64()).unwrap_or(0.);
            min[axis] = side + if n==0 {offset} else {-offset};
            max[axis] = min[axis] + bar;
            min[along] = contact_min;
            max[along] = contact_max;
            min[2] = lo[2] - p;
            max[2] = lo[2];
            parallels.push(json!({"id": format!("{id}-parallel-{}", n + 1), "min": min.map(r), "max": max.map(r)}));
        }
    }
    let faces = json!([format!("stock.{}.min", ["x", "y"][axis]), format!("stock.{}.max", ["x", "y"][axis])]);
    let support = if let Some(seat)=&seating && parallel.is_some() {
        format!("two modeled parallels on horizontal faces at model Z{}, width {} mm; support measurements {}; actual side contact {}; holes and edge breaks reduce measured contact",seat["seat_z_model"],seat["support_width_mm"],seat["supports"],seat["contacts"])
    } else if parallel.is_some() {
        "seated on two parallels".to_string()
    } else {
        format!("raise stock on parallels or similar so jaws grip only {} mm; parallels_height_mm absent: support is not modeled or checked, so through cuts are not checked against it", show(grip))
    };
    let choice = if setup["grip_surface_model"].is_array() { "finished-part extent within the jaw height band; remote bosses do not set jaw opening" } else if seating.is_some() { "proposed mesh-checked opposed side contact and edge seating; retained top finishing must establish the grip" } else if given.is_some() { "given clamping override, validated" } else { "stock long sides, closing across the narrower XY dimension (X on a tie)" };
    let remainder = if seating.is_some() { "grip preparation must be verified against retained top operations" }
        else if setup["flip_role"] == "sacrificial_grip" { "sacrificial stock below the cap is removed by bottom facing" }
        else { "material below the cap needs a later setup" };
    let decision = trace.decide(RULE, &id, if given.is_some() { "given" } else { "chosen" }, &format!("grip {faces}: {choice}; {} mm grip, jaw top Z{}, {} mm side-cut clearance; {remainder}; {}", show(grip), show(top), show(clearance), support), json!({"chosen": {"faces": faces, "grip_height_mm": grip, "parallels_height_mm": parallel}}));
    setup["clamping"] = json!({"faces": faces, "grip_height_mm": grip, "jaw_top_z": top, "clearance_mm": clearance, "jaws": jaws, "parallels": parallels, "because": [decision]});
    if let Some(seat)=&seating {
        let z=seat["contact_patches_model"].as_array().into_iter().flatten().flat_map(|side|side.as_array().into_iter().flatten()).flat_map(|patch|patch.as_array().into_iter().flatten()).filter_map(|p|p[2].as_f64()).fold(f64::NEG_INFINITY,f64::max);
        setup["clamping"]["contact_min_z"] = json!(r(setup["origin_model"][2].as_f64()? - z));
        let mut contacts=seat["contacts"].as_array()?.clone();
        if axis==1 {contacts.reverse();}
        setup["clamping"]["contacts"] = json!(contacts);
        setup["clamping"]["contact_along_min"] = json!(contact_min);
        setup["clamping"]["contact_along_max"] = json!(contact_max);
    }
    if let Some(seat)=seating
        && let Some(d)=trace.decisions.iter_mut().find(|d|d["id"]==setup["clamping"]["because"][0]) {
            d["chosen"]["contact_measurements"]=setup["clamping"]["contacts"].clone();
            d["chosen"]["support_measurements"]=seat["supports"].clone();
    }
    None
}

struct Proposal {
    axis: usize,
    grip_height_mm: f64,
    parallels_height_mm: Option<f64>,
}
impl Proposal {
    fn read(value: &Value) -> Result<Self, String> {
        let object = value.as_object().ok_or("expected an object")?;
        for key in object.keys() {
            if !["axis", "grip_height_mm", "parallels_height_mm"].contains(&key.as_str()) {
                return Err(format!("unknown field {key}"));
            }
        }
        let axis = match value["axis"].as_str() { Some("x") => 0, Some("y") => 1, _ => return Err("axis must be x or y".into()) };
        let grip_height_mm = value["grip_height_mm"].as_f64().ok_or("grip_height_mm must be a number")?;
        let parallels_height_mm = match value.get("parallels_height_mm").filter(|v| !v.is_null()) {
            Some(v) => Some(v.as_f64().ok_or("parallels_height_mm must be a number or null")?),
            None => None,
        };
        Ok(Self { axis, grip_height_mm, parallels_height_mm })
    }
}

/// Validate a supplied proposal with the ordinary clamp path; refusal falls back to the rule.
pub fn supplied_clamp(setup: &mut Value, vise: Option<&Value>, trace: &mut Trace) -> Option<String> {
    let subject = setup["id"].as_str().unwrap_or("");
    if let Some(n) = trace.supplied_for(RULE, subject, None) {
        let choice = trace.supplied[n]["choice"].clone();
        if let Some(reason) = clamp(setup, vise, Some(&choice), trace) {
            trace.refuse(n, &reason);
        } else {
            let decision = setup["clamping"]["because"][0].as_str().unwrap().to_owned();
            trace.accept(n, &decision);
            return None;
        }
    }
    clamp(setup, vise, None, trace)
}

/// Bounds of the complete mesh intersecting the grip-height slab. Include all
/// curved faces for exclusion, but only analytic planes later certify contact.
fn band_bounds(setup: &Value, axis: usize, low: f64, high: f64) -> Option<[f64; 2]> {
    let tris: Vec<[[f64;3];3]> = serde_json::from_value(setup["grip_surface_model"].clone()).ok()?;
    let rotation: [[f64;3];3] = serde_json::from_value(setup["rotation"].clone()).ok()?;
    let origin: [f64;3] = serde_json::from_value(setup["origin_model"].clone()).ok()?;
    let mut bounds = [f64::INFINITY,f64::NEG_INFINITY];
    for tri in tris {
        let polygon = tri.into_iter().map(|p| {
            let local: [f64;3] = std::array::from_fn(|i| (0..3).map(|j| rotation[i][j]*(p[j]-origin[j])).sum());
            [local[axis],local[2]]
        }).collect();
        for p in crate::flip::clipped(polygon,[[f64::NEG_INFINITY,low],[f64::INFINITY,high]]) {
            bounds[0] = bounds[0].min(p[0]);
            bounds[1] = bounds[1].max(p[0]);
        }
    }
    (bounds[0]<bounds[1]).then(|| bounds.map(r))
}
