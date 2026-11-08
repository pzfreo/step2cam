//! Stock-face clamping in each setup's own frame. No inferred machine dimensions.
use crate::{plan::{r, show}, trace::Trace};
use serde_json::{Value, json};

const RULE: &str = "machine.clamping";

/// Records the chosen clamping, or returns why the setup cannot be held. Missing jaw
/// dimensions retain the old opening warning and explicitly skip geometric checks.
pub fn clamp(setup: &mut Value, vise: Option<&Value>, proposal: Option<&Value>, trace: &mut Trace) -> Option<String> {
    let id = setup["id"].as_str().unwrap_or("").to_string();
    setup.as_object_mut()?.remove("clamping");
    let lo: [f64; 3] = serde_json::from_value(setup["stock"]["min"].clone()).ok()?;
    let hi: [f64; 3] = serde_json::from_value(setup["stock"]["max"].clone()).ok()?;
    let given = match proposal.map(Proposal::read).transpose() {
        Ok(given) => given,
        Err(e) => return Some(format!("invalid clamping override: {e}")),
    };
    let axis = given.as_ref().map_or_else(|| usize::from(hi[0] - lo[0] > hi[1] - lo[1]), |p| p.axis);
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
    let parallel = if proposal.is_some_and(|p| p.get("parallels_height_mm").is_some()) {
        given.as_ref().and_then(|p| p.parallels_height_mm)
    } else { vise.and_then(|v| v["parallels_height_mm"].as_f64()) };
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
    let center = (lo[along] + hi[along]) / 2.;
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
        let bar = rule["min_grip_mm"].as_f64().unwrap().min(across / 3.);
        for (n, side) in [lo[axis], hi[axis] - bar].iter().enumerate() {
            let mut min = lo;
            let mut max = hi;
            min[axis] = *side;
            max[axis] = side + bar;
            min[2] = lo[2] - p;
            max[2] = lo[2];
            parallels.push(json!({"id": format!("{id}-parallel-{}", n + 1), "min": min.map(r), "max": max.map(r)}));
        }
    }
    let faces = json!([format!("stock.{}.min", ["x", "y"][axis]), format!("stock.{}.max", ["x", "y"][axis])]);
    let support = if parallel.is_some() {
        "seated on two parallels".to_string()
    } else {
        format!("raise stock on parallels or similar so jaws grip only {} mm; parallels_height_mm absent: support is not modeled or checked, so through cuts are not checked against it", show(grip))
    };
    let choice = if given.is_some() { "given clamping override, validated" } else { "stock long sides, closing across the narrower XY dimension (X on a tie)" };
    let decision = trace.decide(RULE, &id, if given.is_some() { "given" } else { "chosen" }, &format!("grip {faces}: {choice}; {} mm grip, jaw top Z{}, {} mm side-cut clearance; material below the cap needs a later setup; {}", show(grip), show(top), show(clearance), support), json!({"chosen": {"faces": faces, "grip_height_mm": grip, "parallels_height_mm": parallel}}));
    setup["clamping"] = json!({"faces": faces, "grip_height_mm": grip, "jaw_top_z": top, "clearance_mm": clearance, "jaws": jaws, "parallels": parallels, "because": [decision]});
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
