//! Spot operations are inserted once, before index-based execution exclusions are replayed.
//! Later reconciliation only removes spots; a refused spot operation is never recreated.
use crate::kind::OpKind;
use crate::model::{Cut, Holes, Operation, Plan, Spot, SpotDrill};
use crate::resources::{Library, drill_tip_length, id, num};
use crate::trace::{Trace, because_op};
use serde_json::{Value, json};

/// The spot drilling an operation is, if it is one.
fn spot_drill(op: &Operation) -> Option<&SpotDrill> {
    match &op.cut { Cut::SpotDrill(spot) => Some(spot), _ => None }
}

/// The holes an operation drills, if it drills them.
fn holes(op: &Operation) -> Option<&Holes> {
    match &op.cut { Cut::Holes(hole) => Some(hole), _ => None }
}

/// Inserts spot drilling before the holes it starts, in every setup.
pub(crate) fn create_with(plan: &mut Plan, library: &Library, trace: &mut Trace) -> Result<(), String> {
    for setup in &mut plan.setups {
        let top = setup.stock.max[2];
        let setup_id = setup.id.clone();
        let ops = &mut setup.operations;
        // Rebuild spots when hole choices are supplied, including finish on a stored plan.
        let supplied_holes = trace.supplied.iter().any(|s| s["rule"] == "tool.hole.drill_or_bore" && s["subject"] == format!("{setup_id}:holes"));
        if supplied_holes { ops.retain(|o| o.cut.kind() != OpKind::SpotDrill); }
        let existing: Vec<String> = ops.iter().filter_map(spot_drill).flat_map(|o| o.features.iter().cloned()).collect();
        let groups = layout(ops, &setup_id, top, library, trace, false, |count, trace| groups(ops, &setup_id, top, library, trace, count, &|feature| !existing.iter().any(|f| f == feature)))?;
        for mut op in groups {
            let made = trace.decide("process.spot_drill", &format!("{setup_id}:spot_drill"), "planned",
                "spot twist-drilled holes before drilling; countersinks require a matching cone within 1°", json!({"features":op.cut.features()}));
            because_op(&mut op, [made]);
            ops.push(op);
        }
        ops.sort_by_key(|op| crate::rules::rank(op.cut.kind().name()).unwrap_or(usize::MAX));
    }
    Ok(())
}

/// Takes out spots whose holes are no longer drilled; with `finalize`, a countersink cut by its
/// spot is no longer skipped.
pub(crate) fn prune_with(plan: &mut Plan, library: &Library, finalize: bool, trace: &mut Trace) -> Result<(), String> {
    let mut cut = vec![];
    for setup in &mut plan.setups {
        let top = setup.stock.max[2];
        let setup_id = setup.id.clone();
        let ops = &mut setup.operations;
        let retained = layout(ops, &setup_id, top, library, trace, true, |count, trace| pruned(ops, &setup_id, top, library, trace, count))?;
        cut.extend(retained.iter().filter_map(spot_drill).flat_map(|o| &o.spots).filter(|s| s.countersink).flat_map(|s| s.features.iter().cloned()));
        // Reconcile groups from surviving spot features only; excluded spots stay excluded.
        ops.retain(|op| op.cut.kind() != OpKind::SpotDrill);
        ops.extend(retained);
        ops.sort_by_key(|op| crate::rules::rank(op.cut.kind().name()).unwrap_or(usize::MAX));
    }
    if !finalize { return Ok(()); }
    for feature in cut.iter().map(String::as_str) {
        let made = trace.decide("process.spot_drill", feature, "cut", "countersink cut by matching spot drill", Value::Null);
        // Only replace the countersink skip, never other skips on the same feature.
        let decisions: Vec<String> = trace.skipped.iter().filter(|s| s.feature == feature && s.reason.starts_with("countersink:")).filter_map(|s| s.decision.clone()).collect();
        trace.skipped.retain(|s| !(s.feature == feature && s.reason.starts_with("countersink:")));
        for d in &mut trace.decisions {
            if decisions.iter().any(|id| d["id"] == id.as_str()) { d["superseded_by"] = json!(made); }
        }
    }
    Ok(())
}

/// Preview the exact hole choice resolve will use, without accepting or refusing it yet.
/// The caller supplies the number after spot insertion/pruning, not the pre-spot index.
fn hole_drill<'a>(ops: &[Operation], setup: &str, number: usize, op: &Operation, top: f64, library: &Library<'a>, trace: &Trace) -> Option<&'a Value> {
    let subject = format!("{setup}:holes");
    let operation = format!("{setup}-{number}-holes");
    let ambiguous = ops.iter().filter(|o| o.cut.kind() == OpKind::Holes).count() > 1;
    if let Some(n) = trace.supplied_for("tool.hole.drill_or_bore", &subject, Some(&operation))
        && (!ambiguous || trace.supplied[n].get("operation").is_some())
        && let Ok((tool, _)) = library.supplied_tool(op, top, &trace.supplied[n]["choice"]) {
        return (tool["type"] == "drill").then_some(tool);
    }
    // Only hole operations are drilled.
    let hole = holes(op)?;
    let diameter = hole.diameter;
    let depth = library.drill_depth(hole, top);
    // Match hole resolution: a usable bore mill takes precedence over a short drill.
    library.drill(diameter, depth).or_else(|| {
        if library.bore_mill(diameter).is_some() { None } else { library.deferred_drill(diameter, depth) }
    })
}

/// The spot drilling operations a tool cuts: finish takes out the others, so only these number the
/// holes after them.
fn tooled(ops: &[Operation]) -> usize {
    ops.iter().filter_map(spot_drill).filter(|s| s.spot_tool.is_some()).count()
}

/// The hole operations, each with its number counted without spots.
fn numbered_holes(ops: &[Operation]) -> impl Iterator<Item = (usize, &Operation, &Holes)> {
    ops.iter().filter(|o| o.cut.kind() != OpKind::SpotDrill).enumerate().filter_map(|(index, o)| holes(o).map(|hole| (index, o, hole)))
}

fn groups(ops: &[Operation], setup_id: &str, top: f64, library: &Library, trace: &Trace, spot_count: usize, include: &dyn Fn(&str) -> bool) -> Result<Vec<Operation>, String> {
    let mut groups: Vec<(Vec<String>, SpotDrill)> = vec![];
    for (index, op, hole) in numbered_holes(ops) {
        let Some(drill) = hole_drill(ops, setup_id, index + spot_count + 1, op, top, library, trace) else { continue; };
        let diameter = num(drill, "diameter")?;
        let angle = num(drill, "point_angle")?;
        let mouth = hole.top_z;
        let reach = (top - mouth).max(0.);
        for (n, center) in hole.centers.iter().enumerate() {
            let feature = hole.features.get(n).ok_or("holes name fewer features than centres")?;
            if !include(feature) { continue; }
            let csink = hole.countersinks.as_ref().and_then(|c| c.get(n)).and_then(Option::as_ref).unwrap_or(&Value::Null);
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
            let index = groups.iter().position(|(_, g)| g.spot_tool.as_deref() == name).unwrap_or_else(|| {
                let parent: Vec<String> = op.because.iter().flatten().filter(|id|
                    trace.decisions.iter().any(|d| d["id"] == id.as_str() && (d["stage"] == "setup" || d["stage"] == "sequence")))
                    .cloned().collect();
                groups.push((parent, SpotDrill { spot_tool: name.map(String::from), diameter, angle, reach, features: vec![], spots: vec![] }));
                groups.len()-1
            });
            let group = &mut groups[index].1;
            group.diameter = group.diameter.max(diameter);
            group.angle = group.angle.max(angle);
            group.reach = group.reach.max(reach);
            let depth = tool.map(|t| drill_tip_length(t).map(|point| point * spot_diameter / num(t, "diameter").unwrap())).transpose()?;
            group.spots.push(Spot { center: *center, top_z: mouth, diameter: spot_diameter, depth, features: vec![feature.clone()], countersink });
            group.features.push(feature.clone());
        }
    }
    Ok(groups.into_iter().map(|(because, spot)| Operation { cut: Cut::SpotDrill(spot), tool: None, tool_note: None,
        because: Some(because), iso14649: None, technology: None }).collect())
}

/// Spots precede all holes. Enumerate their possible count to match supplied execution
/// names against the resulting layout, rather than accidentally matching pre-spot indices.
/// Prefer layouts that actually use the supplied choices. Ambiguous/inconsistent names
/// are refused; no preview decisions leak into the trace.
fn layout(ops: &[Operation], setup: &str, top: f64, library: &Library, trace: &mut Trace, pruning: bool, build: impl Fn(usize, &Trace) -> Result<Vec<Operation>, String>) -> Result<Vec<Operation>, String> {
    let subject = format!("{setup}:holes");
    if ops.iter().filter(|o| o.cut.kind() == OpKind::Holes).count() > 1
        && let Some(n) = trace.supplied_for("tool.hole.drill_or_bore", &subject, None) {
        trace.refuse(n, "subject matches more than one operation; name operation explicitly");
    }
    let indexed: Vec<usize> = trace.supplied.iter().enumerate().filter(|(_, s)| s["rule"] == "tool.hole.drill_or_bore" && s["subject"] == subject && s.get("operation").is_some() && s.get("accepted").is_none()).map(|(n, _)| n).collect();
    let current = tooled(ops);
    if indexed.is_empty() { return build(current, trace); }
    // Keep an already consistent layout during pruning. Choices that omitted spots in
    // creation must not appear to permit a different mapping just because those spots
    // are absent now. Only exclusions or a removed group can require another layout.
    if pruning {
        let candidate = build(current, trace)?;
        if tooled(&candidate) == current { return Ok(candidate); }
    }
    let maximum: usize = ops.iter().filter_map(holes).map(|hole| hole.features.len()).sum();
    let mut best = None;
    let mut best_score = 0;
    let mut ties = 0;
    for count in 0..=maximum {
        let candidate = build(count, trace)?;
        if tooled(&candidate) != count { continue; }
        let mut score = 0;
        for (index, op, _) in numbered_holes(ops) {
            let operation = format!("{setup}-{}-holes", index + count + 1);
            if let Some(n) = trace.supplied_for("tool.hole.drill_or_bore", &subject, Some(&operation))
                && library.supplied_tool(op, top, &trace.supplied[n]["choice"]).is_ok() { score += 1; }
        }
        if best.is_none() || score > best_score { best = Some(candidate); best_score = score; ties = 1; }
        else if score == best_score { ties += 1; }
    }
    if ties == 1 { return Ok(best.unwrap()); }
    for n in indexed { trace.refuse(n, "operation numbering is ambiguous or inconsistent after spot drilling; use the finished plan's operation number"); }
    // With indexed choices refused, spot count no longer affects the choice.
    build(tooled(ops), trace)
}

fn pruned(ops: &[Operation], setup_id: &str, top: f64, library: &Library, trace: &Trace, spot_count: usize) -> Result<Vec<Operation>, String> {
    if trace.supplied.iter().any(|s| s["rule"] == "tool.hole.drill_or_bore" && s["subject"] == format!("{setup_id}:holes") && s.get("accepted").is_none()) {
        let old: Vec<&Operation> = ops.iter().filter(|o| o.cut.kind() == OpKind::SpotDrill).collect();
        let allowed: Vec<&String> = old.iter().filter_map(|o| spot_drill(o)).flat_map(|o| &o.features).collect();
        let mut rebuilt = groups(ops, setup_id, top, library, trace, spot_count, &|feature| allowed.iter().any(|f| *f == feature))?;
        for group in &mut rebuilt {
            let features = spot_drill(group).map(|spot| spot.features.clone()).unwrap_or_default();
            let mut because = vec![];
            for previous in &old {
                if spot_drill(previous).is_some_and(|p| p.features.iter().any(|f| features.contains(f))) {
                    for decision in previous.because.iter().flatten() {
                        if !because.contains(decision) { because.push(decision.clone()); }
                    }
                }
            }
            group.because = Some(because);
        }
        return Ok(rebuilt);
    }
    let mut drilled = vec![];
    let mut angles = vec![];
    for (index, op, hole) in numbered_holes(ops) {
        if let Some(drill) = hole_drill(ops, setup_id, index + spot_count + 1, op, top, library, trace) {
            for feature in &hole.features {
                drilled.push(feature.clone());
                angles.push((feature.clone(), num(drill, "point_angle")?));
            }
        }
    }
    let mut retained: Vec<Operation> = ops.iter().filter(|o| o.cut.kind() == OpKind::SpotDrill).cloned().collect();
    for op in &mut retained {
        let Cut::SpotDrill(spot) = &mut op.cut else { continue; };
        spot.spots.retain(|s| s.features.iter().all(|f| drilled.contains(f)));
        let features: Vec<String> = spot.spots.iter().flat_map(|s| s.features.iter().cloned()).collect();
        spot.diameter = spot.spots.iter().map(|s| s.diameter).fold(0., f64::max);
        spot.reach = spot.spots.iter().map(|s| (top-s.top_z).max(0.)).fold(0., f64::max);
        spot.angle = angles.iter().filter(|(f, _)| features.contains(f)).map(|(_, a)| *a).fold(0., f64::max);
        spot.features = features;
    }
    retained.retain(|o| spot_drill(o).is_some_and(|s| !s.spots.is_empty()));
    Ok(retained)
}
