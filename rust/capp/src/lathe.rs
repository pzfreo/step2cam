//! Turning: a part with turned steps is made on a lathe first, when the resources have one.
//!
//! The plan records the part's turned profile (`turned`: the axis, its steps, the holes on the
//! axis, and the features lying on the profile). With a lathe in the resources' `machines`, the
//! part is turned from bar in a lathe setup: face the free end, rough and finish the profile,
//! drill the holes on the axis from the tailstock, and part off. The mill's setups then start
//! from the turned part, so they lose their facing, roughing and what follows roughing, and the
//! holes the lathe drilled. Every choice is a traced decision.
//!
//! The lathe setup is planned, not posted: camlib cuts on mills only.
use crate::plan::{r, show};
use crate::trace::Trace;
use serde_json::{Value, json};

/// The lathe: the first machine of kind `lathe`.
pub fn lathe(resources: &Value) -> Option<&Value> {
    resources["machines"].as_array()?.iter().find(|m| m["kind"] == "lathe")
}

fn name(m: &Value) -> String {
    m["name"].as_str().or(m["id"].as_str()).unwrap_or("the lathe").to_string()
}

/// The shop's turning allowances (`turning` in the resources), with defaults.
fn policy(resources: &Value, key: &str, default: f64) -> f64 {
    resources["turning"][key].as_f64().unwrap_or(default)
}

/// Operations a part turned first no longer needs on the mill: it arrives at size.
const BLOCK_ONLY: [&str; 5] = ["facing", "roughing", "flats", "wall_finish", "edge_chamfer"];

/// Plans the lathe setup, if the part has a turned profile and the resources a lathe that can
/// hold it, and takes off the mill what the lathe does.
pub fn route(plan: &mut Value, resources: &Value, trace: &mut Trace) {
    let Some(turned) = plan.get("turned").filter(|t| !t.is_null()).cloned() else { return };
    let steps: Vec<&Value> = turned["steps"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    if steps.is_empty() {
        return;
    }
    let Some(machine) = lathe(resources) else {
        trace.decide("lathe.route", "part", "none", "the part has turned steps but the resources have no lathe: they are not cut", json!({"chosen": null}));
        return;
    };
    let lo = steps.iter().filter_map(|s| s["lo"].as_f64()).fold(f64::INFINITY, f64::min);
    let hi = steps.iter().filter_map(|s| s["hi"].as_f64()).fold(f64::NEG_INFINITY, f64::max);
    let widest = steps.iter().filter_map(|s| s["diameter"].as_f64()).fold(0., f64::max);
    let end_diameter = |z: f64| steps.iter().find(|s| (s["lo"].as_f64() == Some(z)) || (s["hi"].as_f64() == Some(z))).and_then(|s| s["diameter"].as_f64()).unwrap_or(widest);
    let (length, bar) = (hi - lo, (widest + policy(resources, "bar_oversize_mm", 1.)).ceil());
    let face = policy(resources, "face_allowance_mm", 0.5);
    let part_off = policy(resources, "part_off_allowance_mm", 3.);
    let grip = machine["workholding"].as_array().into_iter().flatten().find(|w| w["kind"] == "chuck").and_then(|w| w["grip_mm"].as_f64()).unwrap_or(12.);
    let stock_length = r(length + face + part_off + grip);

    // Can this lathe take it?
    let swing = machine["swing_mm"]["over_carriage"].as_f64().or(machine["swing_mm"]["over_bed"].as_f64());
    let centres = machine["between_centres_mm"].as_f64();
    let mut refusals = vec![];
    if let Some(s) = swing.filter(|s| bar > *s) {
        refusals.push(format!("⌀{} bar is larger than {} turns (⌀{} over the carriage)", show(bar), name(machine), show(s)));
    }
    if let Some(c) = centres.filter(|c| stock_length > *c) {
        refusals.push(format!("{} mm of bar is longer than its {} mm between centres", show(stock_length), show(c)));
    }
    if !refusals.is_empty() {
        let says = refusals.join("; ");
        trace.warn("lathe.capacity", "part", format!("the turned steps are not cut: {says}"));
        return;
    }

    // The smaller end faces the tailstock: each step is turned towards the chuck, and the part is
    // parted off at the larger end.
    let free_at_hi = end_diameter(hi) <= end_diameter(lo);
    let (free, chuck) = if free_at_hi { (hi, lo) } else { (lo, hi) };
    let w = |z: f64| r((z - free).abs());
    let route = trace.decide("lathe.route", "part", "chosen", &format!("turned first on {} from ⌀{} bar, {} mm long", name(machine), show(bar), show(stock_length)), json!({"chosen": machine["id"]}));
    let orient = trace.decide("lathe.orientation", "part", "chosen", &format!("the ⌀{} end faces the tailstock; the part is parted off at the ⌀{} end", show(end_diameter(free)), show(end_diameter(chuck))), Value::Null);
    let bore = machine["spindle"]["bore_mm"].as_f64();
    let feed = if bore.is_some_and(|b| bar <= b) { "fed through the spindle" } else { "cut to length first, then held in the chuck" };
    let bar_says = trace.decide("lathe.bar", "part", "chosen", &format!("⌀{} bar ({} mm over the largest diameter), {} mm long: {} mm to face, {} mm to part off, {} mm held in the chuck; {feed}", show(bar), show(r(bar - widest)), show(stock_length), show(face), show(part_off), show(grip)), Value::Null);

    let mut profile: Vec<Value> = steps.iter().map(|s| {
        let (a, b) = (w(s["lo"].as_f64().unwrap_or(lo)), w(s["hi"].as_f64().unwrap_or(hi)));
        json!({"from_w": a.min(b), "to_w": a.max(b), "diameter": s["diameter"], "feature": s["feature"]})
    }).collect();
    profile.sort_by(|a, b| a["from_w"].as_f64().unwrap_or(0.).total_cmp(&b["from_w"].as_f64().unwrap_or(0.)));
    let step_ids: Vec<Value> = steps.iter().map(|s| s["feature"].clone()).collect();
    let allowance = policy(resources, "allowance_mm", 0.3);
    let base = [route.clone(), orient.clone(), bar_says.clone()];
    let mut ops = vec![
        json!({"kind": "lathe_face", "w_mm": 0., "bar_diameter": bar, "stock_mm": face, "features": []}),
        json!({"kind": "lathe_rough", "profile": profile, "bar_diameter": bar, "allowance_mm": allowance, "features": step_ids}),
        json!({"kind": "lathe_finish", "profile": profile, "features": step_ids}),
    ];
    // Holes on the axis that open at the free end: drilled from the tailstock.
    let mut drilled: Vec<String> = vec![];
    for hole in turned["axial_holes"].as_array().into_iter().flatten() {
        let at = hole["at"].as_f64().unwrap_or(f64::NAN);
        let into_part = (free_at_hi && hole["inward"] == "down") || (!free_at_hi && hole["inward"] == "up");
        if (at - free).abs() <= 2. && into_part {
            let id = hole["feature"].as_str().unwrap_or("").to_string();
            ops.push(json!({"kind": "lathe_drill", "diameter": hole["diameter"], "depth": hole["depth"], "bottom": hole["bottom"], "from_w": w(at), "features": [id]}));
            drilled.push(id);
        }
    }
    ops.push(json!({"kind": "lathe_part_off", "w_mm": r(length), "diameter": bar, "features": []}));
    for op in &mut ops {
        let kind = op["kind"].as_str().unwrap_or("").to_string();
        let rule = match kind.as_str() {
            "lathe_face" => "lathe.face",
            "lathe_rough" => "lathe.rough",
            "lathe_finish" => "lathe.finish",
            "lathe_drill" => "lathe.drill",
            _ => "lathe.part_off",
        };
        let features: Vec<String> = op["features"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
        let says = if features.is_empty() { format!("{kind} on the lathe") } else { format!("{kind} on the lathe: {}", features.join(", ")) };
        let made = trace.decide(rule, &format!("lathe:{kind}"), "planned", &says, json!({"features": features}));
        crate::trace::because(op, base.iter().cloned().chain([made]));
    }
    let setup = json!({
        "id": "lathe", "kind": "lathe", "machine": machine["id"], "axis": turned["axis"],
        "free_end": free, "chuck_end": chuck, "bar": {"diameter": bar, "length": stock_length},
        "operations": ops,
    });

    // The turned steps are cut now: their earlier skips are replaced.
    for id in step_ids.iter().filter_map(Value::as_str) {
        trace.supersede(id, "skipped", &route);
    }
    trace.skipped.retain(|s| !step_ids.contains(&s["feature"]));
    // Features on the profile are made by turning: those left to roughing, and those skipped for
    // want of roughing or a mesh.
    let on_profile: Vec<Value> = turned["on_profile"].as_array().cloned().unwrap_or_default();
    let skipped_on_profile: Vec<String> = trace.skipped.iter().filter(|s| on_profile.contains(&s["feature"])).filter_map(|s| s["feature"].as_str().map(String::from)).collect();
    for id in &skipped_on_profile {
        let made = trace.decide("lathe.turned_surface", id, "covered", "on the turned profile: made by turning", Value::Null);
        trace.supersede(id, "skipped", &made);
    }
    trace.skipped.retain(|s| !on_profile.contains(&s["feature"]));
    let mut covered = plan["covered_by_roughing"].as_array().cloned().unwrap_or_default();
    covered.retain(|id| {
        if on_profile.contains(id) {
            let made = trace.decide("lathe.turned_surface", id.as_str().unwrap_or(""), "covered", "on the turned profile: made by turning", Value::Null);
            trace.supersede(id.as_str().unwrap_or(""), "covered", &made);
            false
        } else {
            true
        }
    });

    // The mill starts from the turned part.
    for setup in plan["setups"].as_array_mut().into_iter().flatten() {
        let setup_id = setup["id"].as_str().unwrap_or("").to_string();
        let ops = setup["operations"].as_array_mut().map(std::mem::take).unwrap_or_default();
        let mut kept = vec![];
        for mut op in ops {
            let kind = op["kind"].as_str().unwrap_or("").to_string();
            if BLOCK_ONLY.contains(&kind.as_str()) {
                trace.decide("lathe.mill_after_turning", &format!("{setup_id}:{kind}"), "removed", &format!("{kind} is not needed: the part comes to the mill turned"), Value::Null);
                continue;
            }
            if kind == "holes" {
                let features: Vec<String> = op["features"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
                if features.iter().any(|f| drilled.contains(f)) {
                    let keep: Vec<usize> = (0..features.len()).filter(|i| !drilled.contains(&features[*i])).collect();
                    trace.decide("lathe.drilled_on_lathe", &format!("{setup_id}:holes"), "removed", &format!("drilled on the lathe instead: {}", features.iter().filter(|f| drilled.contains(f)).cloned().collect::<Vec<_>>().join(", ")), Value::Null);
                    if keep.is_empty() {
                        continue;
                    }
                    let pick = |op: &Value, key: &str| json!(keep.iter().map(|i| op[key][*i].clone()).collect::<Vec<_>>());
                    let (centers, features) = (pick(&op, "centers"), pick(&op, "features"));
                    op["centers"] = centers;
                    op["features"] = features;
                }
            }
            kept.push(op);
        }
        setup["operations"] = json!(kept);
    }
    // Nothing on the mill is left to roughing now: what remains is not cut by it.
    for id in covered.iter().filter_map(Value::as_str) {
        let not = trace.skip("lathe.mill_after_turning", id, "no operation of its own, and a part turned first is not roughed on the mill".into());
        trace.supersede(id, "covered", &not);
    }
    plan["covered_by_roughing"] = json!([]);
    // A mill setup the lathe has emptied is not needed.
    if let Some(setups) = plan["setups"].as_array_mut() {
        setups.retain(|s| s["operations"].as_array().is_some_and(|o| !o.is_empty()));
        setups.insert(0, setup);
    }
}
