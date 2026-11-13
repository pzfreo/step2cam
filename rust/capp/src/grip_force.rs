//! A sliding-force screening calculation, not a contact or overturning simulation.
use crate::model::{Cut, Operation, Plan};
use crate::trace::{Trace, because_op};
use serde_json::{Value, json};
use std::f64::consts::PI;

const RULE: &str = "machine.grip_vs_cut";

// Engagement envelopes are deliberately simple; the trace states the inputs, including
// unknown roughing subregion entry (full slot). Axial cycles are handled separately.
fn engagement(op: &Operation, top: f64, diameter: f64, step: f64, over: f64, allowance: f64) -> (f64, f64) {
    match &op.cut {
        Cut::Facing { floor_z } => ((top-floor_z).max(0.).min(step), over.min(diameter)),
        Cut::Roughing(r) => {
            // Layer heights remain authored from r.top_z, while facing removes air
            // above `top`. Check both end layers and any full middle layer.
            let first = (r.top_z-step).max(r.floor_z);
            let layers = ((r.top_z-r.floor_z)/step).ceil();
            let depth = (top-first).max(0.).max(if layers > 2. {step} else {(first-r.floor_z).max(0.)});
            (depth.min(step), if r.entry_access.as_deref() == Some("stock_edge") {over.min(diameter)} else {diameter})
        },
        Cut::Flats { floor_z, .. } => ((top-floor_z).max(0.).min(step), over.min(diameter)),
        Cut::WallFinish(w) => ((top-w.floor_z).max(0.).min(step), allowance.min(diameter)),
        Cut::Holes(h) => ((h.bore_top_z.min(top)-h.bottom_z).max(0.).min(step), diameter),
        Cut::SpotDrill(_) => (0., 0.),
        Cut::Thread(t) => ((t.top_z.min(top)-t.bottom_z).max(0.).min(step), diameter),
        Cut::Counterbore(b) => ((b.top_z.min(top)-b.floor_z).max(0.).min(step), diameter),
        Cut::Slot(s) => ((s.top_z.min(top)-s.floor_z).max(0.).min(step), s.width.min(diameter)),
        Cut::Notch(n) => ((n.top_z.min(top)-n.floor_z).max(0.).min(step), n.width.min(diameter)),
        Cut::VChannel(c) => (c.depth.min(step), c.width.min(diameter)),
        Cut::EdgeChamfer(c) => (c.width.min(step), diameter),
    }
}

pub(crate) fn check(plan: &mut Plan, library: &Value, trace: &mut Trace) {
    let Some(vise) = crate::machines::mill(library).and_then(|m| m["workholding"].as_array())
        .and_then(|ws| ws.iter().find(|w| w["kind"] == "vise")) else { return };
    let name = vise["name"].as_str().or(vise["id"].as_str()).unwrap_or("vise");
    let rule = &crate::rules::workholding()["grip_force"];
    let mu = rule["friction_coefficient"].as_f64().unwrap();
    let safety = rule["safety_factor"].as_f64().unwrap();
    let kc1 = rule["kc1_n_mm2"].as_f64().unwrap();
    let mc = rule["mc"].as_f64().unwrap();
    let quantum = rule["facing_depth_quantum_mm"].as_f64().unwrap();
    let material = plan.material.as_ref().and_then(|m| m["used"].as_str()).unwrap_or("").to_lowercase();
    let aluminium = material.contains("aluminium") || material.contains("aluminum");
    for setup in &mut plan.setups {
        if !setup.clamping.as_ref().is_some_and(Value::is_object) { continue; }
        let Some(clamp) = vise["clamp_force_n"].as_f64().filter(|_| vise["_sources"]["clamp_force"]["url"].is_string()) else {
            trace.warn(RULE, &setup.id, format!("grip not checked against the cut: the library gives no clamping force for {name}"), json!({}));
            continue;
        };
        let faces = setup.clamping.as_ref().unwrap()["faces"].as_array().map_or(0, Vec::len);
        let holding = mu * clamp * faces as f64;
        let torque = vise["clamp_torque_nm"].as_f64().map_or_else(|| "torque unspecified".into(), |n| format!("at {n} Nm"));
        let detail = json!({"clamp_force_n":clamp,"clamp_torque_nm":vise["clamp_torque_nm"],"friction_coefficient":mu,
            "jaw_faces":faces,"holding_force_n":holding,"safety_factor":safety,"allowable_cutting_force_n":holding/safety,
            "clamp_source":vise["_sources"]["clamp_force"]});
        trace.decide(RULE, &setup.id, "checked", &format!("{name}: clamp {clamp:.2} N {torque}, mu {mu}, {faces} jaw faces: holding force {holding:.2} N; safety factor {safety}, allowable cutting force {:.2} N. Sliding only; contact pressure, lift and overturning are not checked", holding/safety), detail.clone());
        setup.clamping.as_mut().unwrap()["force_check"] = detail.clone();
        let mut top = setup.stock.max[2];
        if !aluminium {
            let material_name = if material.is_empty() { "unspecified material" } else { material.as_str() };
            trace.warn(RULE, &setup.id, format!("grip not checked against the cut: no steel-jaw friction and cutting-force reference for {material_name}"), detail.clone());
        }
        for (index, op) in setup.operations.iter_mut().enumerate() {
            let subject = format!("{}-{}-{}", setup.id, index+1, op.cut.kind().name());
            let Some(tool) = op.tool.as_ref().and_then(|id| id.as_ref()).and_then(|id| library["tools"].as_array()?.iter().find(|t| t["id"] == *id)) else { continue };
            if matches!(op.cut, Cut::SpotDrill(_)) || matches!(tool["type"].as_str(), Some("drill" | "spot_drill" | "tap")) {
                let says = "thrust into the supports; not a sliding load on the grip";
                let mut extra = detail.clone();
                extra["force_model"] = json!(says);
                because_op(op, [trace.decide(RULE, &subject, "none", says, extra)]);
                continue;
            }
            if !aluminium {
                continue;
            }
            let tech = op.technology.as_ref().expect("resolved cutting data");
            let number = |key: &str| tech.get(key).or_else(|| tool.get(key)).and_then(Value::as_f64);
            let diameter = tool["diameter"].as_f64().unwrap();
            let rpm = number("spindle_rpm").unwrap();
            let feed = number("feed_mm_min").unwrap();
            let step = number("stepdown_mm").unwrap_or(diameter);
            let over = number("stepover_mm").unwrap_or(diameter);
            let allowance = library["radial_allowance_mm"].as_f64().unwrap_or(0.3);
            let (ap, ae) = engagement(op, top, diameter, step, over, allowance);
            // The pinned bore emits one revolution per stepdown, not a 3° helix.
            // For holes/counterbores ap is bounded by that pitch and the material
            // depth; do not substitute a smaller imaginary pitch for the job.
            let assumption = match &op.cut {
                Cut::Roughing(r) => match r.entry_access.as_deref() {
                    Some("stock_edge") => "stock-edge raster".to_string(),
                    Some("closed") => "closed-entry full slot".to_string(),
                    _ => "full slot (entry unknown)".to_string(),
                },
                Cut::Holes(_) | Cut::Counterbore(_) => format!("helix (pitch at most {step:.2} mm/rev)"),
                _ if ae == diameter => "full slot".to_string(),
                _ => "radial engagement".to_string(),
            };
            let flutes = tool["flutes"].as_f64().unwrap_or(3.);
            let fz = feed / (rpm * flutes);
            let phi = (1.-2.*ae/diameter).clamp(-1., 1.).acos();
            let h = if phi > 0. { fz * (1.-phi.cos()) / phi } else { 0. };
            if h <= 0. || !h.is_finite() {
                because_op(op, [trace.warn(RULE, &subject, "grip force not checked: no positive chip thickness".into(), detail.clone())]);
                continue;
            }
            let kc = kc1 * h.powf(-mc);
            let section = ap * ae * feed / (PI * diameter * rpm);
            let force = kc * section;
            let mut extra = detail.clone();
            extra["cutting_force_n"] = json!(force);
            extra["kc1_n_mm2"] = json!(kc1);
            extra["mc"] = json!(mc);
            extra["kc_n_mm2"] = json!(kc);
            extra["chip_thickness_mm"] = json!(h);
            extra["axial_depth_mm"] = json!(ap);
            extra["radial_width_mm"] = json!(ae);
            extra["feed_mm_min"] = json!(feed);
            extra["spindle_rpm"] = json!(rpm);
            extra["flutes"] = json!(flutes);
            extra["force_model"] = json!(format!("mean tangential milling force; {assumption}"));
            let tightening = vise["clamp_torque_nm"].as_f64().map_or_else(|| "library clamping force (torque unspecified)".into(), |n| format!("{:.0} ft·lbf tightening", n/1.3558179483314));
            let mut says = format!("{subject}: assumes ⌀{diameter:.2} {assumption}, ap{ap:.2} × ae{ae:.2} mm; {tightening}; F{force:.2} N, allowable {:.2} N (holding {holding:.2} N / safety {safety})", holding/safety);
            let decision = if force*safety > holding {
                if let Cut::Facing { floor_z } = &op.cut {
                    let limit = (ap * holding / (safety*force) / quantum).floor() * quantum;
                    if limit < quantum {
                        let minimum_force = force*quantum/ap;
                        extra["per_pass_force_n"] = json!(minimum_force);
                        says.push_str(&format!("; cannot be held at any depth: even {quantum} mm needs {minimum_force:.2} N before safety factor {safety}; facing unchanged"));
                        let faced_top = *floor_z;
                        because_op(op, [trace.warn(RULE, &subject, says, extra)]);
                        top = top.min(faced_top);
                        continue;
                    }
                    let passes = ((top-floor_z)/limit).ceil() as u64;
                    op.technology.as_mut().unwrap().insert("stepdown_mm".into(), json!(limit));
                    extra["stepdown_mm"] = json!(limit);
                    extra["passes"] = json!(passes);
                    extra["per_pass_force_n"] = json!(force*limit/ap);
                    says.push_str(&format!("; face in {passes} passes at most {limit:.2} mm deep ({:.2} N per full pass), last pass takes the remainder", force*limit/ap));
                    trace.decide(RULE, &subject, "stepped_down", &says, extra)
                } else {
                    let limit = (ap * holding / (safety*force) / quantum).floor() * quantum;
                    if limit >= quantum {
                        extra["suggested_stepdown_mm"] = json!(limit);
                        says.push_str(&format!("; reduce stepdown to ≤{limit:.2} mm under this model; operation unchanged"));
                    } else {
                        says.push_str(&format!("; even {quantum} mm cannot be held under this model; increase verified clamping capacity or change the engagement; operation unchanged"));
                    }
                    trace.warn(RULE, &subject, says, extra)
                }
            } else { trace.decide(RULE, &subject, "checked", &says, extra) };
            because_op(op, [decision]);
            if let Cut::Facing { floor_z } = op.cut { top = top.min(floor_z); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spot_engagement_is_thrust_even_without_a_tool() {
        let op: Operation = serde_json::from_value(json!({"kind":"spot_drill", "spot_tool":null,
            "diameter":3.3,"angle":120.,"reach":1.,"features":[],"spots":[]})).unwrap();
        assert_eq!(engagement(&op,0.,10.,5.,4.,0.3),(0.,0.));
    }
    #[test]
    fn facing_air_does_not_hide_a_full_middle_roughing_layer() {
        let op: Operation = serde_json::from_value(json!({"kind":"roughing", "top_z":0.,
            "floor_z":-29.,"web_mm":0.,"surface_error_mm":0.01})).unwrap();
        assert_eq!(engagement(&op,-1.,20.,10.,8.,0.3),(10.,20.));
    }
}
