//! Stock-face clamping in each setup's own frame. No inferred machine dimensions.
use crate::{plan::show, trace::Trace};
use serde_json::{Value, json};

const RULE: &str = "machine.clamping";

/// Records the chosen clamping, or returns why the setup cannot be held. Missing jaw
/// dimensions retain the old opening warning and explicitly skip geometric checks.
pub fn clamp(setup: &mut Value, vise: Option<&Value>, trace: &mut Trace) -> Option<String> {
    let id = setup["id"].as_str().unwrap_or("").to_string();
    setup.as_object_mut()?.remove("clamping");
    let lo: [f64; 3] = serde_json::from_value(setup["stock"]["min"].clone()).ok()?;
    let hi: [f64; 3] = serde_json::from_value(setup["stock"]["max"].clone()).ok()?;
    let axis = usize::from(hi[0] - lo[0] > hi[1] - lo[1]);
    let along = 1 - axis;
    let across = hi[axis] - lo[axis];
    let opening = vise.and_then(|v| v["max_jaw_opening_mm"].as_f64());
    let dimensions = vise.and_then(|v| v["jaw_width_mm"].as_f64().zip(v["jaw_height_mm"].as_f64()).zip(opening));
    let Some(((width, height), opening)) = dimensions else {
        trace.decide(RULE, &id, "skipped", "vise jaw width, height or opening absent: clamping check and fixture scene skipped; no dimensions assumed", Value::Null);
        if let Some(opening) = opening.filter(|v| across > *v) {
            trace.warn("machine.workholding", &id, format!("setup {id}: the stock is {} mm across at its narrowest, wider than the vise opens ({} mm): hold it another way", show(across), show(opening)));
        }
        return None;
    };
    if ![width, height, opening].iter().all(|v| v.is_finite() && *v > 0.) {
        return Some("vise dimensions must be positive finite numbers".into());
    }
    if across > opening {
        return Some(format!("stock is {} mm across at its narrowest, wider than the vise opens ({} mm)", show(across), show(opening)));
    }
    let rule = crate::rules::workholding();
    let stock_height = hi[2] - lo[2];
    let minimum = rule["min_grip_mm"].as_f64().unwrap().max(stock_height * rule["min_grip_fraction"].as_f64().unwrap());
    let parallel = vise.and_then(|v| v["parallels_height_mm"].as_f64());
    let grip = parallel.map_or(minimum, |p| height - p);
    let clearance = rule["clearance_mm"].as_f64().unwrap();
    // The faced top must also stand clear, not just the uncut stock top.
    let part_top = setup["operations"].as_array().into_iter().flatten().find(|o| o["kind"] == "facing").and_then(|o| o["floor_z"].as_f64()).unwrap_or(hi[2]);
    if grip < minimum || grip > height || grip > stock_height * rule["max_grip_fraction"].as_f64().unwrap() || part_top <= lo[2] + grip + clearance {
        return Some(format!("too little stock/part above the jaws: {} mm grip required (minimum {} mm), jaw height {} mm, stock height {} mm; part must clear jaw top by {} mm", show(grip), show(minimum), show(height), show(stock_height), show(clearance)));
    }
    let top = lo[2] + grip;
    let center = (lo[along] + hi[along]) / 2.;
    let mut jaws = vec![];
    for (n, side) in [lo[axis] - height, hi[axis]].iter().enumerate() {
        let mut min = [0., 0., top - height];
        let mut max = [0., 0., top];
        min[axis] = *side;
        max[axis] = side + height;
        min[along] = center - width / 2.;
        max[along] = center + width / 2.;
        jaws.push(json!({"id": format!("{id}-jaw-{}", n + 1), "min": min, "max": max}));
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
            parallels.push(json!({"id": format!("{id}-parallel-{}", n + 1), "min": min, "max": max}));
        }
    }
    let faces = json!([format!("stock.{}.min", ["x", "y"][axis]), format!("stock.{}.max", ["x", "y"][axis])]);
    let decision = trace.decide(RULE, &id, "chosen", &format!("grip {faces}: stock long sides, closing across the narrower XY dimension (X on a tie); {} mm grip, jaw top Z{}, {} mm side-cut clearance; material below the cap needs a later setup; {}", show(grip), show(top), show(clearance), if parallel.is_some() { "seated on two parallels" } else { "jaws positioned for the chosen grip, no parallels used" }), json!({"chosen": {"faces": faces, "grip_height_mm": grip}}));
    setup["clamping"] = json!({"faces": faces, "grip_height_mm": grip, "jaw_top_z": top, "clearance_mm": clearance, "jaws": jaws, "parallels": parallels, "because": [decision]});
    None
}
