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
use crate::kind::OpKind;
use crate::model::Plan;
use crate::plan::{r, show};
use crate::trace::Trace;
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// The mill the plan is cut on: the first machine of kind `mill`.
pub fn mill(resources: &Value) -> Option<&Value> {
    resources["machines"].as_array()?.iter().find(|m| m["kind"] == "mill")
}

/// The first mill's first vise, only when all geometric dimensions can be checked.
pub fn dimensioned_vise(resources: &Value) -> Option<&Value> {
    mill(resources)?["workholding"].as_array()?.iter().find(|w|w["kind"]=="vise")
        .filter(|v|["jaw_width_mm","jaw_height_mm","max_jaw_opening_mm"].iter()
            .all(|k|v[*k].as_f64().is_some_and(|n|n.is_finite() && n>0.)))
}

fn name(m: &Value) -> String {
    m["name"].as_str().or(m["id"].as_str()).unwrap_or("the mill").to_string()
}

/// [`place_with`] on a plan as JSON. Panics if the plan does not decode.
pub fn place(plan: &mut Value, resources: &Value, trace: &mut Trace) {
    let mut typed = Plan::deserialize(&*plan).expect("the plan decodes");
    place_with(&mut typed, resources, trace);
    *plan = serde_json::to_value(&typed).expect("the plan encodes");
}

/// Puts each setup on the mill and checks it fits: its travels, and the vise. A setup that does
/// not fit loses its operations, its features saying why.
pub(crate) fn place_with(plan: &mut Plan, resources: &Value, trace: &mut Trace) {
    let Some(mill) = mill(resources) else {
        trace.decide("machine.for_setup", "part", "none", "no machine is described in the resources: travels, tool holding and the tool changer are not checked", json!({"chosen": null}));
        return;
    };
    let id = mill["id"].as_str().unwrap_or("mill").to_string();
    let travel = &mill["axes"]["travel_mm"];
    let vise = mill["workholding"].as_array().into_iter().flatten().find(|w| w["kind"] == "vise");
    // Later setups carry the plan's mesh, `null` when it has none.
    let surface = plan.workholding_surface_model.clone().unwrap_or(Value::Null);
    let mut lost: Vec<(&str, String, String)> = vec![];
    let mut lost_roughing = None;
    for (index, setup) in plan.setups.iter_mut().enumerate() {
        if index > 0 && !setup.flip_seating.as_ref().is_some_and(Value::is_object) {setup.grip_surface_model = Some(surface.clone());}
        let setup_id = setup.id.clone();
        trace.decide("machine.for_setup", &setup_id, "chosen", &format!("cut on {}, a 3-axis mill", name(mill)), json!({"chosen": id}));
        setup.machine = Some(id.clone());
        let size = |i: usize| setup.stock.max[i] - setup.stock.min[i];
        let (dx, dy, dz) = (size(0), size(1), size(2));
        let limit = |axis: &str| travel[axis].as_f64();
        let over: Vec<String> = [("x", dx), ("y", dy), ("z", dz)]
            .iter()
            .filter_map(|(axis, d)| limit(axis).filter(|t| d > t).map(|t| format!("{} mm in {axis} against {} mm of travel", show(r(*d)), show(t))))
            .collect();
        let clamping_failure = if over.is_empty() { crate::workholding::supplied_clamp(setup, vise, trace) } else { None };
        if !over.is_empty() || clamping_failure.is_some() {
            let turned = limit("x").zip(limit("y")).is_some_and(|(tx, ty)| dy <= tx && dx <= ty && dz <= limit("z").unwrap_or(f64::INFINITY));
            let hint = if turned { "; turned 90° it would fit, which this plan does not do" } else { "" };
            let rule = if clamping_failure.is_some() { "machine.clamping" } else { "machine.travel" };
            let says = clamping_failure.unwrap_or_else(|| format!("the {setup_id} setup's stock does not fit {}: {}{hint}", name(mill), over.join(", ")));
            trace.warn(rule, &setup_id, format!("setup {setup_id} is not cut: {says}"), json!({}));
            for op in &setup.operations {
                if op.cut.kind() == OpKind::Roughing {
                    lost_roughing = Some((rule, says.clone()));
                }
                for f in op.cut.features().into_iter().flatten() {
                    lost.push((rule, f.to_string(), says.clone()));
                }
            }
            setup.operations.clear();
        }
    }
    for (rule, feature, says) in lost {
        trace.skip(rule, &feature, format!("not cut: {says}"));
    }
    // Nothing is covered by roughing that does not run. Wait until every setup is checked,
    // so a roughing operation retained in another setup still counts.
    let roughs = plan.setups.iter().any(|s| s.operations.iter().any(|o| o.cut.kind() == OpKind::Roughing));
    if let Some((rule, says)) = lost_roughing.filter(|_| !roughs) {
        let covered = std::mem::take(&mut plan.covered_by_roughing);
        for id in &covered {
            let not_cut = trace.skip(rule, id, format!("not cut: {says}"));
            trace.supersede(id, "covered", &not_cut);
        }
    }
}

/// Caps cutting data at the spindle's top speed, scaling the feeds so each tooth takes the same
/// chip. Returns what it did, for the trace.
pub fn cap_speed(technology: &mut Map<String, Value>, resources: &Value) -> Option<String> {
    let max = mill(resources)?["spindle"]["max_rpm"].as_f64()?;
    let rpm = technology.get("spindle_rpm")?.as_f64()?;
    if rpm <= max {
        return None;
    }
    let k = max / rpm;
    for key in ["feed_mm_min", "plunge_mm_min"] {
        if let Some(v) = technology.get(key).and_then(Value::as_f64) {
            technology.insert(key.into(), json!(r(v * k)));
        }
    }
    technology.insert("spindle_rpm".into(), json!(max));
    technology.insert("capped".into(), json!(true));
    Some(format!("{} rpm is more than the spindle's {}: run at {}, the feeds scaled by the same {}", show(rpm), show(max), show(max), show(r(k))))
}

/// [`check_changer_with`] on a plan as JSON. Panics if the plan does not decode.
pub fn check_changer(plan: &Value, resources: &Value, trace: &mut Trace) {
    check_changer_with(&Plan::deserialize(plan).expect("the plan decodes"), resources, trace);
}

/// Flags a setup that needs more tools than the changer holds, and says when every change is by
/// hand.
pub(crate) fn check_changer_with(plan: &Plan, resources: &Value, trace: &mut Trace) {
    let Some(mill) = mill(resources) else { return };
    let changer = &mill["tool_changer"];
    for setup in &plan.setups {
        let setup_id = setup.id.as_str();
        let mut tools: Vec<&str> = setup.operations.iter().filter_map(|o| o.tool.as_ref()?.as_deref()).collect();
        tools.sort_unstable();
        tools.dedup();
        if tools.is_empty() {
            continue;
        }
        if mill["spindle"]["collet_change"] == "manual" {
            let mut shanks: Vec<f64> = tools.iter().filter_map(|id| {
                let tool = resources["tools"].as_array()?.iter().find(|t| t["id"] == *id)?;
                tool["shank_diameter"].as_f64().or(tool["diameter"].as_f64())
            }).collect();
            shanks.sort_by(f64::total_cmp);
            shanks.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
            if shanks.len() > 1 {
                let sizes = shanks.iter().map(|s| show(*s)).collect::<Vec<_>>().join(", ");
                trace.warn("machine.spindle.manual_collets", setup_id,
                    format!("setup {setup_id}: tools need shanks of {sizes} mm; spindle collets are changed by hand. The ATC cannot perform these collet changes; arrange manual changes or verified adapters before running this setup"),
                    json!({"shank_sizes_mm": shanks, "tools": tools}));
            }
        }
        if changer["kind"] == "manual" {
            trace.warn("machine.tool_changer.manual", setup_id, format!("setup {setup_id}: {} tools, each changed by hand", tools.len()), json!({}));
            continue;
        }
        if let Some(pockets) = changer["pockets"].as_u64() {
            if tools.len() as u64 > pockets {
                trace.warn("machine.tool_changer.pockets", setup_id, format!("setup {setup_id} uses {} tools and the changer holds {pockets}: swap tools part-way through ({})", tools.len(), tools.join(", ")), json!({}));
            }
        }
    }
}
