//! Spot operations are inserted once, before index-based execution exclusions are replayed.
//! Later reconciliation only removes spots; a refused spot operation is never recreated.
use crate::resources::{Library, drill_tip_length, id, num};
use crate::trace::{Trace, because};
use serde_json::{Value, json};

pub(crate) fn create(plan: &mut Value, library: &Library) -> Result<(), String> {
    let mut trace = Trace::take(plan);
    let result = create_with(plan, library, &mut trace);
    trace.store(plan);
    result
}

fn create_with(plan: &mut Value, library: &Library, trace: &mut Trace) -> Result<(), String> {
    for setup in plan["setups"].as_array_mut().ok_or("plan has no setups")? {
        let top = setup["stock"]["max"][2].as_f64().ok_or("bad stock")?;
        let setup_id = setup["id"].as_str().unwrap_or("").to_owned();
        let ops = setup["operations"].as_array_mut().ok_or("setup has no operations")?;
        // finish may also be called on an already finished plan.
        let existing: Vec<Value> = ops.iter().filter(|o| o["kind"] == "spot_drill")
            .flat_map(|o| o["features"].as_array().into_iter().flatten().cloned()).collect();
        let mut groups: Vec<Value> = vec![];
        for op in ops.iter().filter(|o| o["kind"] == "holes") {
            let Some(drill) = library.drill(num(op, "diameter")?, library.drill_depth(op, top)?) else { continue; };
            let diameter = num(drill, "diameter")?;
            let angle = num(drill, "point_angle")?;
            let mouth = num(op, "top_z")?;
            let reach = (top - mouth).max(0.);
            for (n, center) in op["centers"].as_array().ok_or("holes have no centers")?.iter().enumerate() {
                let feature = &op["features"][n];
                if existing.contains(feature) { continue; }
                let csink = &op["countersinks"][n];
                let enlarged = csink["major_diameter"].as_f64().zip(csink["included_angle"].as_f64())
                    .filter(|(d, _)| *d > diameter)
                    // An enlarged countersink uses its own cone angle; the twist-drill
                    // angle minimum applies only to ordinary, drill-diameter spots.
                    .and_then(|(d, a)| library.spot(d, 0., reach, a).map(|t| (t, d)));
                let (tool, spot_diameter, countersink) = match enlarged {
                    Some((t, d)) => (Some(t), d, true),
                    None => (library.spot(diameter, angle, reach, 0.), diameter, false),
                };
                let name = tool.map(id);
                let index = groups.iter().position(|g| g["spot_tool"].as_str() == name).unwrap_or_else(|| {
                    let parent: Vec<Value> = op["because"].as_array().into_iter().flatten().filter(|id|
                        trace.decisions.iter().any(|d| d["id"] == **id && (d["stage"] == "setup" || d["stage"] == "sequence")))
                        .cloned().collect();
                    groups.push(json!({"because":parent,"kind":"spot_drill", "spot_tool":name, "spots":[], "features":[],
                        "diameter":diameter,"angle":angle,"reach":reach}));
                    groups.len()-1
                });
                let group = &mut groups[index];
                for (key, value) in [("diameter", diameter), ("angle", angle), ("reach", reach)] {
                    group[key] = json!(group[key].as_f64().unwrap().max(value));
                }
                let depth = tool.map(|t| drill_tip_length(t).map(|point| point * spot_diameter / num(t, "diameter").unwrap())).transpose()?;
                group["spots"].as_array_mut().unwrap().push(json!({"center":center,"top_z":mouth,
                    "diameter":spot_diameter,"depth":depth,"features":[feature],"countersink":countersink}));
                group["features"].as_array_mut().unwrap().push(feature.clone());
            }
        }
        for mut op in groups {
            let made = trace.decide("process.spot_drill", &format!("{setup_id}:spot_drill"), "planned",
                "spot twist-drilled holes before drilling; countersinks require a matching cone within 1°", json!({"features":op["features"]}));
            because(&mut op, [made]);
            ops.push(op);
        }
        ops.sort_by_key(|op| crate::rules::rank(op["kind"].as_str().unwrap_or("")).unwrap_or(usize::MAX));
    }
    Ok(())
}

pub(crate) fn prune(plan: &mut Value, library: &Library, finalize: bool) -> Result<(), String> {
    let mut cut = vec![];
    for setup in plan["setups"].as_array_mut().ok_or("plan has no setups")? {
        let top = setup["stock"]["max"][2].as_f64().ok_or("bad stock")?;
        let ops = setup["operations"].as_array_mut().ok_or("setup has no operations")?;
        let mut drilled = vec![];
        let mut angles = vec![];
        for op in ops.iter().filter(|o| o["kind"] == "holes") {
            if let Some(drill) = library.drill(num(op, "diameter")?, library.drill_depth(op, top)?) {
                for feature in op["features"].as_array().into_iter().flatten() {
                    drilled.push(feature.clone());
                    angles.push((feature.clone(), num(drill, "point_angle")?));
                }
            }
        }
        for op in ops.iter_mut().filter(|o| o["kind"] == "spot_drill") {
            let spots = op["spots"].as_array_mut().ok_or("spot operation has no spots")?;
            spots.retain(|s| s["features"].as_array().into_iter().flatten().all(|f| drilled.contains(f)));
            let features: Vec<Value> = spots.iter().flat_map(|s| s["features"].as_array().into_iter().flatten().cloned()).collect();
            cut.extend(spots.iter().filter(|s| s["countersink"] == true).flat_map(|s| s["features"].as_array().into_iter().flatten().cloned()));
            let diameter = spots.iter().filter_map(|s| s["diameter"].as_f64()).fold(0., f64::max);
            let reach = spots.iter().filter_map(|s| s["top_z"].as_f64()).map(|z| (top-z).max(0.)).fold(0., f64::max);
            let angle = angles.iter().filter(|(f, _)| features.contains(f)).map(|(_, a)| *a).fold(0., f64::max);
            op["diameter"] = json!(diameter);
            op["reach"] = json!(reach);
            op["angle"] = json!(angle);
            op["features"] = json!(features);
        }
        ops.retain(|o| o["kind"] != "spot_drill" || o["spots"].as_array().is_some_and(|s| !s.is_empty()));
    }
    if !finalize { return Ok(()); }
    let mut trace = Trace::take(plan);
    for feature in &cut {
        let Some(feature) = feature.as_str() else { continue; };
        let made = trace.decide("process.spot_drill", feature, "cut", "countersink cut by matching spot drill", Value::Null);
        // Only replace the countersink skip, never other skips on the same feature.
        let decisions: Vec<Value> = trace.skipped.iter().filter(|s| s["feature"] == feature && s["reason"].as_str().is_some_and(|r| r.starts_with("countersink:"))).filter_map(|s| s.get("decision").cloned()).collect();
        trace.skipped.retain(|s| !(s["feature"] == feature && s["reason"].as_str().is_some_and(|r| r.starts_with("countersink:"))));
        for d in &mut trace.decisions {
            if decisions.contains(&d["id"]) { d["superseded_by"] = json!(made); }
        }
    }
    trace.store(plan);
    Ok(())
}
