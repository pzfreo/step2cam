//! Spot operations are inserted once, before index-based execution exclusions are replayed.
//! Later reconciliation only removes spots; a refused spot operation is never recreated.
use crate::model::{Cut, Operation};
use crate::resources::{Library, drill_tip_length, id, num};
use crate::trace::{Trace, because};
use serde::Deserialize;
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
        // Rebuild spots when hole choices are supplied, including finish on a stored plan.
        let supplied_holes = trace.supplied.iter().any(|s| s["rule"] == "tool.hole.drill_or_bore" && s["subject"] == format!("{setup_id}:holes"));
        if supplied_holes { ops.retain(|o| o["kind"] != "spot_drill"); }
        let existing: Vec<Value> = ops.iter().filter(|o| o["kind"] == "spot_drill")
            .flat_map(|o| o["features"].as_array().into_iter().flatten().cloned()).collect();
        let groups = layout(ops, &setup_id, top, library, trace, false, |count, trace| groups(ops, &setup_id, top, library, trace, count, &|feature| !existing.contains(feature)))?;
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
    let mut trace = Trace::take(plan);
    let result = prune_with(plan, library, finalize, &mut trace);
    trace.store(plan);
    result
}

fn prune_with(plan: &mut Value, library: &Library, finalize: bool, trace: &mut Trace) -> Result<(), String> {
    let mut cut = vec![];
    for setup in plan["setups"].as_array_mut().ok_or("plan has no setups")? {
        let top = setup["stock"]["max"][2].as_f64().ok_or("bad stock")?;
        let setup_id = setup["id"].as_str().unwrap_or("").to_owned();
        let ops = setup["operations"].as_array_mut().ok_or("setup has no operations")?;
        let retained = layout(ops, &setup_id, top, library, trace, true, |count, trace| pruned(ops, &setup_id, top, library, trace, count))?;
        cut.extend(retained.iter().flat_map(|o| o["spots"].as_array().into_iter().flatten()).filter(|s| s["countersink"] == true).flat_map(|s| s["features"].as_array().into_iter().flatten().cloned()));
        // Reconcile groups from surviving spot features only; excluded spots stay excluded.
        ops.retain(|op| op["kind"] != "spot_drill");
        ops.extend(retained);
        ops.sort_by_key(|op| crate::rules::rank(op["kind"].as_str().unwrap_or("")).unwrap_or(usize::MAX));
    }
    if !finalize { return Ok(()); }
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
    Ok(())
}

/// Preview the exact hole choice resolve will use, without accepting or refusing it yet.
/// The caller supplies the number after spot insertion/pruning, not the pre-spot index.
fn hole_drill<'a>(ops: &[Value], setup: &str, number: usize, op: &Value, top: f64, library: &Library<'a>, trace: &Trace) -> Result<Option<&'a Value>, String> {
    let subject = format!("{setup}:holes");
    let operation = format!("{setup}-{number}-holes");
    let ambiguous = ops.iter().filter(|o| o["kind"] == "holes").count() > 1;
    // A bridge: the library takes the typed operation; spots hold JSON until stage 5.
    let typed = Operation::deserialize(op).map_err(|e| e.to_string())?;
    if let Some(n) = trace.supplied_for("tool.hole.drill_or_bore", &subject, Some(&operation))
        && (!ambiguous || trace.supplied[n].get("operation").is_some())
        && let Ok((tool, _)) = library.supplied_tool(&typed, top, &trace.supplied[n]["choice"]) {
        return Ok((tool["type"] == "drill").then_some(tool));
    }
    let Cut::Holes(hole) = &typed.cut else { return Err("not a hole operation".into()) };
    let diameter = hole.diameter;
    let depth = library.drill_depth(hole, top);
    // Match hole resolution: a usable bore mill takes precedence over a short drill.
    Ok(library.drill(diameter, depth).or_else(|| {
        if library.bore_mill(diameter).is_some() { None } else { library.deferred_drill(diameter, depth) }
    }))
}

fn groups(ops: &[Value], setup_id: &str, top: f64, library: &Library, trace: &Trace, spot_count: usize, include: &dyn Fn(&Value) -> bool) -> Result<Vec<Value>, String> {
    let mut groups: Vec<Value> = vec![];
    for (index, op) in ops.iter().filter(|o| o["kind"] != "spot_drill").enumerate().filter(|(_, o)| o["kind"] == "holes") {
        let Some(drill) = hole_drill(ops, setup_id, index + spot_count + 1, op, top, library, trace)? else { continue; };
        let diameter = num(drill, "diameter")?;
        let angle = num(drill, "point_angle")?;
        let mouth = num(op, "top_z")?;
        let reach = (top - mouth).max(0.);
        for (n, center) in op["centers"].as_array().ok_or("holes have no centers")?.iter().enumerate() {
            let feature = &op["features"][n];
            if !include(feature) { continue; }
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
    Ok(groups)
}

/// Spots precede all holes. Enumerate their possible count to match supplied execution
/// names against the resulting layout, rather than accidentally matching pre-spot indices.
/// Prefer layouts that actually use the supplied choices. Ambiguous/inconsistent names
/// are refused; no preview decisions leak into the trace.
fn layout<T>(ops: &[Value], setup: &str, top: f64, library: &Library, trace: &mut Trace, pruning: bool, build: impl Fn(usize, &Trace) -> Result<Vec<T>, String>) -> Result<Vec<T>, String> {
    let subject = format!("{setup}:holes");
    if ops.iter().filter(|o| o["kind"] == "holes").count() > 1
        && let Some(n) = trace.supplied_for("tool.hole.drill_or_bore", &subject, None) {
        trace.refuse(n, "subject matches more than one operation; name operation explicitly");
    }
    let indexed: Vec<usize> = trace.supplied.iter().enumerate().filter(|(_, s)| s["rule"] == "tool.hole.drill_or_bore" && s["subject"] == subject && s.get("operation").is_some() && s.get("accepted").is_none()).map(|(n, _)| n).collect();
    let current = ops.iter().filter(|o| o["kind"] == "spot_drill").count();
    if indexed.is_empty() { return build(current, trace); }
    // Keep an already consistent layout during pruning. Choices that omitted spots in
    // creation must not appear to permit a different mapping just because those spots
    // are absent now. Only exclusions or a removed group can require another layout.
    if pruning {
        let candidate = build(current, trace)?;
        if candidate.len() == current { return Ok(candidate); }
    }
    let maximum: usize = ops.iter().filter(|o| o["kind"] == "holes").map(|o| o["features"].as_array().map_or(0, Vec::len)).sum();
    let mut best = None;
    let mut best_score = 0;
    let mut ties = 0;
    for count in 0..=maximum {
        let candidate = build(count, trace)?;
        if candidate.len() != count { continue; }
        let mut score = 0;
        for (index, op) in ops.iter().filter(|o| o["kind"] != "spot_drill").enumerate().filter(|(_, o)| o["kind"] == "holes") {
            let operation = format!("{setup}-{}-holes", index + count + 1);
            if let Some(n) = trace.supplied_for("tool.hole.drill_or_bore", &subject, Some(&operation))
                // A bridge: the library takes the typed operation; spots hold JSON until stage 5.
                && library.supplied_tool(&Operation::deserialize(op).map_err(|e| e.to_string())?, top, &trace.supplied[n]["choice"]).is_ok() { score += 1; }
        }
        if best.is_none() || score > best_score { best = Some(candidate); best_score = score; ties = 1; }
        else if score == best_score { ties += 1; }
    }
    if ties == 1 { return Ok(best.unwrap()); }
    for n in indexed { trace.refuse(n, "operation numbering is ambiguous or inconsistent after spot drilling; use the finished plan's operation number"); }
    // With indexed choices refused, spot count no longer affects the choice.
    build(ops.iter().filter(|o| o["kind"] == "spot_drill").count(), trace)
}

fn pruned(ops: &[Value], setup_id: &str, top: f64, library: &Library, trace: &Trace, spot_count: usize) -> Result<Vec<Value>, String> {
    if trace.supplied.iter().any(|s| s["rule"] == "tool.hole.drill_or_bore" && s["subject"] == format!("{setup_id}:holes") && s.get("accepted").is_none()) {
        let old: Vec<&Value> = ops.iter().filter(|o| o["kind"] == "spot_drill").collect();
        let allowed: Vec<&Value> = old.iter().flat_map(|o| o["features"].as_array().into_iter().flatten()).collect();
        let mut rebuilt = groups(ops, setup_id, top, library, trace, spot_count, &|feature| allowed.contains(&feature))?;
        for group in &mut rebuilt {
            let mut because = vec![];
            for previous in &old {
                if previous["features"].as_array().into_iter().flatten().any(|f| group["features"].as_array().is_some_and(|fs| fs.contains(f))) {
                    for decision in previous["because"].as_array().into_iter().flatten() {
                        if !because.contains(decision) { because.push(decision.clone()); }
                    }
                }
            }
            group["because"] = json!(because);
        }
        return Ok(rebuilt);
    }
    let mut drilled = vec![];
    let mut angles = vec![];
    for (index, op) in ops.iter().filter(|o| o["kind"] != "spot_drill").enumerate().filter(|(_, o)| o["kind"] == "holes") {
        if let Some(drill) = hole_drill(ops, setup_id, index + spot_count + 1, op, top, library, trace)? {
            for feature in op["features"].as_array().into_iter().flatten() {
                drilled.push(feature.clone());
                angles.push((feature.clone(), num(drill, "point_angle")?));
            }
        }
    }
    let mut retained: Vec<Value> = ops.iter().filter(|o| o["kind"] == "spot_drill").cloned().collect();
    for op in &mut retained {
        let spots = op["spots"].as_array_mut().ok_or("spot operation has no spots")?;
        spots.retain(|s| s["features"].as_array().into_iter().flatten().all(|f| drilled.contains(f)));
        let features: Vec<Value> = spots.iter().flat_map(|s| s["features"].as_array().into_iter().flatten().cloned()).collect();
        let diameter = spots.iter().filter_map(|s| s["diameter"].as_f64()).fold(0., f64::max);
        let reach = spots.iter().filter_map(|s| s["top_z"].as_f64()).map(|z| (top-z).max(0.)).fold(0., f64::max);
        let angle = angles.iter().filter(|(f, _)| features.contains(f)).map(|(_, a)| *a).fold(0., f64::max);
        op["diameter"] = json!(diameter);
        op["reach"] = json!(reach);
        op["angle"] = json!(angle);
        op["features"] = json!(features);
    }
    retained.retain(|o| o["spots"].as_array().is_some_and(|s| !s.is_empty()));
    Ok(retained)
}
