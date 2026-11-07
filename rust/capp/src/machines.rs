//! The machines in the resources, and what a 3-axis mill's capabilities decide about the plan.
//!
//! `machines` in the resources lists them; each has an `id`, a `kind` and what it can do:
//! - `axes.travel_mm`: x, y and z travel, so a setup whose stock will not fit is not cut;
//! - `workholding`: a vise's `max_jaw_opening_mm`, so stock wider than it is flagged;
//! - `spindle.max_rpm`: cutting data above it are capped, the feeds scaled to keep the chip;
//! - `spindle.max_tool_diameter_mm`: a tool whose shank is wider is never chosen
//!   (`resources::Library`);
//! - `tool_changer`: `pockets`, so a setup needing more tools is flagged; `kind: "manual"` means
//!   every change is by hand.
//!
//! The controller's limits (feeds, planes, arcs, tool numbers) stay in the resources' `machine`,
//! which the post-processor checks. Without `machines`, none of this is checked, and the trace
//! says so.
use crate::plan::{r, show};
use crate::trace::Trace;
use serde_json::{Value, json};

/// The mill the plan is cut on: the first machine of kind `mill`.
pub fn mill(resources: &Value) -> Option<&Value> {
    resources["machines"].as_array()?.iter().find(|m| m["kind"] == "mill")
}

fn name(m: &Value) -> String {
    m["name"].as_str().or(m["id"].as_str()).unwrap_or("the mill").to_string()
}

/// Puts each setup on the mill and checks it fits: its travels, and the vise. A setup that does
/// not fit loses its operations, its features saying why.
pub fn place(plan: &mut Value, resources: &Value, trace: &mut Trace) {
    let Some(mill) = mill(resources) else {
        trace.decide("machine.for_setup", "part", "none", "no machine is described in the resources: travels, tool holding and the tool changer are not checked", json!({"chosen": null}));
        return;
    };
    let id = mill["id"].as_str().unwrap_or("mill").to_string();
    let travel = &mill["axes"]["travel_mm"];
    let jaws = mill["workholding"].as_array().into_iter().flatten().find(|w| w["kind"] == "vise").and_then(|w| w["max_jaw_opening_mm"].as_f64());
    let mut lost: Vec<(String, String)> = vec![];
    for setup in plan["setups"].as_array_mut().into_iter().flatten().filter(|s| s["kind"] != "lathe") {
        let setup_id = setup["id"].as_str().unwrap_or("").to_string();
        trace.decide("machine.for_setup", &setup_id, "chosen", &format!("cut on {}, a 3-axis mill", name(mill)), json!({"chosen": id}));
        setup["machine"] = json!(id);
        let size = |i: usize| setup["stock"]["max"][i].as_f64().unwrap_or(0.) - setup["stock"]["min"][i].as_f64().unwrap_or(0.);
        let (dx, dy, dz) = (size(0), size(1), size(2));
        let limit = |axis: &str| travel[axis].as_f64();
        let over: Vec<String> = [("x", dx), ("y", dy), ("z", dz)]
            .iter()
            .filter_map(|(axis, d)| limit(axis).filter(|t| d > t).map(|t| format!("{} mm in {axis} against {} mm of travel", show(r(*d)), show(t))))
            .collect();
        if !over.is_empty() {
            let turned = limit("x").zip(limit("y")).is_some_and(|(tx, ty)| dy <= tx && dx <= ty && dz <= limit("z").unwrap_or(f64::INFINITY));
            let hint = if turned { "; turned 90° it would fit, which this plan does not do" } else { "" };
            let says = format!("the {setup_id} setup's stock does not fit {}: {}{hint}", name(mill), over.join(", "));
            trace.warn("machine.travel", &setup_id, format!("setup {setup_id} is not cut: {says}"));
            for op in setup["operations"].as_array().into_iter().flatten() {
                for f in op["features"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    lost.push((f.to_string(), says.clone()));
                }
            }
            setup["operations"] = json!([]);
            continue;
        }
        if let Some(jaws) = jaws {
            let across = dx.min(dy);
            if across > jaws {
                trace.warn("machine.workholding", &setup_id, format!("setup {setup_id}: the stock is {} mm across at its narrowest, wider than the vise opens ({} mm): hold it another way", show(r(across)), show(jaws)));
            }
        }
    }
    for (feature, says) in lost {
        trace.skip("machine.travel", &feature, format!("not cut: {says}"));
    }
}

/// Caps cutting data at the spindle's top speed, scaling the feeds so each tooth takes the same
/// chip. Returns what it did, for the trace.
pub fn cap_speed(technology: &mut Value, machine: &Value) -> Option<String> {
    let max = machine["spindle"]["max_rpm"].as_f64()?;
    let rpm = technology["spindle_rpm"].as_f64()?;
    if rpm <= max {
        return None;
    }
    let k = max / rpm;
    for key in ["feed_mm_min", "plunge_mm_min"] {
        if let Some(v) = technology[key].as_f64() {
            technology[key] = json!(r(v * k));
        }
    }
    technology["spindle_rpm"] = json!(max);
    technology["capped"] = json!(true);
    Some(format!("{} rpm is more than the spindle's {}: run at {}, the feeds scaled by the same {}", show(rpm), show(max), show(max), show(r(k))))
}

/// Flags a setup that needs more tools than the changer holds, and says when every change is by
/// hand.
pub fn check_changer(plan: &Value, resources: &Value, trace: &mut Trace) {
    let Some(mill) = mill(resources) else { return };
    let changer = &mill["tool_changer"];
    for setup in plan["setups"].as_array().into_iter().flatten().filter(|s| s["kind"] != "lathe") {
        let setup_id = setup["id"].as_str().unwrap_or("");
        let mut tools: Vec<&str> = setup["operations"].as_array().into_iter().flatten().filter_map(|o| o["tool"].as_str()).collect();
        tools.sort_unstable();
        tools.dedup();
        if tools.is_empty() {
            continue;
        }
        if changer["kind"] == "manual" {
            trace.warn("machine.tool_changer.manual", setup_id, format!("setup {setup_id}: {} tools, each changed by hand", tools.len()));
            continue;
        }
        if let Some(pockets) = changer["pockets"].as_u64() {
            if tools.len() as u64 > pockets {
                trace.warn("machine.tool_changer.pockets", setup_id, format!("setup {setup_id} uses {} tools and the changer holds {pockets}: swap tools part-way through ({})", tools.len(), tools.join(", ")));
            }
        }
    }
}
