//! After planning: what can run with these resources, what an execution engine refused, and which
//! tool cuts each operation.
//!
//! [`finish`] takes a plan from [`crate::plan::plan`], the resources and the exclusions an
//! execution engine has reported, and leaves the plan ready to execute: every operation names its
//! tool (`tool`); one no tool cuts is taken out and its features say why. [`resolve`] alone leaves
//! it in place and says why it has none (`tool_note`, null when nothing needs saying).
use crate::kind::{OpId, OpKind, OpRef};
use crate::model::{Considered, Cut, Exclusion, Holes, Notch, Operation, Plan, Roughing, Setup, Slot, VChannel, WallFinish};
use crate::plan::{r, show};
use crate::resources::{Library, id, num};
use crate::trace::{Trace, because_op};
use serde::Deserialize;
use serde_json::{Value, json};

/// The plan as it must be executed: settled, then for each exclusion in turn that operation
/// taken out and the plan settled again, then every operation's tool resolved. An exclusion names
/// an operation as the execution engine named it (`top-2-roughing`, numbered in the plan as it
/// stood when that engine saw it) and says why it refused it.
pub fn finish(plan: &mut Value, library: &Value, exclusions: &[Value]) -> Result<(), String> {
    // The library is checked before the plan is decoded; finish_plan normalizes it again, which
    // changes nothing and cannot fail once it has passed.
    let normalized = crate::resources::normalize(library)?;
    let mut typed = Plan::deserialize(&*plan).map_err(|e| format!("the plan does not decode: {e}"))?;
    let exclusions = exclusions.iter().map(|exclusion| Exclusion::deserialize(exclusion)
        .map_err(|e| format!("the exclusion {exclusion} does not decode: {e}"))).collect::<Result<Vec<_>, _>>()?;
    let result = finish_plan(&mut typed, normalized.as_ref(), &exclusions);
    *plan = serde_json::to_value(&typed).map_err(|e| e.to_string())?;
    result
}

/// [`finish`] on the typed plan.
pub fn finish_plan(plan: &mut Plan, library: &Value, exclusions: &[Exclusion]) -> Result<(), String> {
    let normalized = crate::resources::normalize(library)?;
    let library = normalized.as_ref();
    crate::supplied::load_with(plan, library)?;
    let mut trace = Trace::take_plan(plan);
    let result = finish_with(plan, library, exclusions, &mut trace);
    // Whatever happened, the plan keeps what it had changed and decided.
    trace.store_plan(plan);
    // The tools still needed are read from the stored trace.
    if result.is_ok() {
        crate::needed::collect(plan);
    }
    result
}

/// The body of [`finish`], on the typed plan with one trace: put the setups on the machine,
/// settle, make spots, take out each exclusion and settle again, resolve the tools, then check
/// the clamping against earlier cuts, the tool changer and the supplied decisions left unused.
fn finish_with(plan: &mut Plan, library: &Value, exclusions: &[Exclusion], trace: &mut Trace) -> Result<(), String> {
    crate::machines::place_with(plan, library, trace);
    settle_with(plan, library, trace)?;
    crate::spots::create_with(plan, &Library::new(library)?, trace)?;
    for exclusion in exclusions {
        let operation = &exclusion.operation.to_string();
        let id = &unfinished_id(plan, library, trace, &exclusion.operation)?;
        let (message, need) = crate::needed::refusal(plan, id, &exclusion.message, exclusion.tool.as_deref(), exclusion.reach.as_ref(), &Library::new(library)?)?;
        let removed = take_out(plan, id)
            .ok_or_else(|| format!("excluded operation {operation} is not in the plan"))?;
        exclude_with(plan, operation, &message, need.as_deref(), id, &removed, trace);
        // Preserve the engine's unedited evidence independently of the reader-facing wording.
        if let Some(decision) = trace.decisions.iter_mut().rev().find(|d| d["rule"].as_str().is_some_and(|r| r.split('@').next() == Some("execution.refused")) && d["outcome"] == "removed") {
            decision["message"] = json!(exclusion.message);
            if let Some(reach) = &exclusion.reach {
                decision["reach"] = json!(reach);
            }
            let refused = decision["id"].clone();
            if let Some(selection) = exclusion.selection.as_ref().and_then(Value::as_object) {
                let mut detail = selection.clone();
                detail.remove("id");
                detail.insert("superseded_by".into(), refused);
                trace.decide(selection["rule"].as_str().unwrap().split('@').next().unwrap(), selection["subject"].as_str().unwrap(), "chosen", selection["says"].as_str().unwrap(), json!(detail));
            }
        }
        settle_with(plan, library, trace)?;
        crate::spots::prune_with(plan, &Library::new(library)?, false, trace)?;
    }
    crate::spots::prune_with(plan, &Library::new(library)?, true, trace)?;
    resolve_with(plan, library, trace)?;
    leave_out_untooled(plan, trace);
    crate::clamping_history::check(plan, &Library::new(library)?, trace);
    crate::machines::check_changer_with(plan, library, trace);
    trace.refuse_unused();
    Ok(())
}

/// The operation `id` names in the finished plan an execution engine saw, which has no operation
/// that no tool cuts, as it is numbered in `plan` before tools are resolved: the plan that finish
/// would give is worked out on a copy, and `id` counts only its tooled operations.
fn unfinished_id(plan: &Plan, library: &Value, trace: &Trace, id: &OpId) -> Result<OpId, String> {
    let (mut finished, mut copy) = (plan.clone(), trace.clone());
    crate::spots::prune_with(&mut finished, &Library::new(library)?, true, &mut copy)?;
    resolve_with(&mut finished, library, &mut copy)?;
    let number = finished.setups.iter().find(|s| s.id == id.setup).and_then(|s| s.operations.iter().enumerate()
        .filter(|(_, o)| !matches!(o.tool, Some(None))).nth(id.number.checked_sub(1)?).map(|(n, _)| n + 1));
    // One it does not have is in neither: past the end.
    Ok(OpId { number: number.unwrap_or(usize::MAX), ..id.clone() })
}

/// Takes each operation that no tool cuts out of its setup. Its features are skipped with why;
/// a spot drilling's holes are still drilled, so instead of being skipped they are named by one
/// warning per setup, which says they are drilled without spotting.
/// The operations after it are numbered again, as after an exclusion.
fn leave_out_untooled(plan: &mut Plan, trace: &mut Trace) {
    for setup in 0..plan.setups.len() {
        let setup_id = plan.setups[setup].id.clone();
        let untooled: Vec<OpId> = plan.setups[setup].operations.iter().enumerate().filter(|(_, o)| matches!(o.tool, Some(None)))
            .map(|(n, o)| OpId { setup: setup_id.clone(), number: n + 1, kind: o.cut.kind(), part: None }).collect();
        let mut unspotted: Vec<String> = vec![];
        let mut why = String::new();
        // Each one taken out moves those after it up by one.
        for (taken, id) in untooled.iter().enumerate() {
            let Some(removed) = take_out(plan, &OpId { number: id.number - taken, ..id.clone() }) else { continue; };
            let operation = id.to_string();
            let note = removed.tool_note.clone().flatten().unwrap_or_else(|| "no tool".into());
            let need = crate::needed::op_need(&trace.decisions, &removed).map(String::from);
            let noun = crate::plan::operation_noun(id.kind.name());
            let reason = match note.strip_prefix("left out: ") { Some(rest) => format!("{noun} left out: {rest}"), None => format!("{noun} left out: {note}") };
            let features = removed.cut.features().unwrap_or_default();
            let left = trace.decide(UNTOOLED, &operation, "removed", &reason, json!({"operation": OpRef::from(id), "features": features}));
            trace.needs(&left, need.as_deref());
            // Spot drilling's holes are still drilled, so they are not skipped: the warning below names them.
            if id.kind != OpKind::SpotDrill {
                for feature in features {
                    let skipped = trace.skip(UNTOOLED, feature, reason.clone());
                    trace.needs(&skipped, need.as_deref());
                }
            }
            // Nothing is covered by roughing that does not run.
            if id.kind == OpKind::Roughing {
                for feature in std::mem::take(&mut plan.covered_by_roughing) {
                    let skipped = trace.skip(UNTOOLED, &feature, format!("no operation of its own, and {reason}"));
                    trace.needs(&skipped, need.as_deref());
                    trace.supersede(&feature, "covered", &skipped);
                }
            }
            if id.kind == OpKind::SpotDrill {
                unspotted.extend(features.iter().cloned());
                why = note;
            } else if features.is_empty() {
                // With no feature to skip, a warning says it is left out.
                trace.warn(UNTOOLED, &operation, format!("{operation} left out: {note}"), json!({}));
            }
            supersede_own(trace, &removed, &left);
        }
        if !unspotted.is_empty() {
            let holes = if unspotted.len() == 1 { "hole" } else { "holes" };
            trace.warn(UNSPOTTED, &format!("{setup_id}:spot_drill"), format!("setup {setup_id}: {} {holes} drilled without spotting: {why}", unspotted.len()), json!({"features": unspotted}));
        }
    }
}

const UNTOOLED: &str = "tool.none.left_out";
const UNSPOTTED: &str = "tool.none.unspotted";

/// Marks the decisions that are the removed operation's own (what made it, its tool and cutting
/// data) as replaced by `by`; those it shares with the rest of the setup (the setup, the order,
/// where its holes are cut) stand. Feature-subject warnings are borrowed by every operation
/// cutting that feature, so they stand too.
fn supersede_own(trace: &mut Trace, removed: &Operation, by: &str) {
    let own: Vec<&str> = removed.because.iter().flatten().map(String::as_str).collect();
    for d in trace.decisions.iter_mut().filter(|d| d["id"].as_str().is_some_and(|id| own.contains(&id))
        && (d["stage"] == "process" || d["stage"] == "tools")
        && !(d["outcome"] == "warning" && d["features"].as_array().is_some_and(|features| features.contains(&d["subject"])))) {
        d["superseded_by"] = json!(by);
    }
}

/// Takes an operation named as the execution engine names them (`top-2-roughing`) out of the
/// plan, and says what that leaves uncut. None if there is no such operation.
pub fn exclude(plan: &mut Value, operation: &str, message: &str) -> Option<Value> {
    let mut typed = Plan::deserialize(&*plan).ok()?;
    let id: OpId = operation.parse().ok()?;
    let removed = take_out(&mut typed, &id)?;
    let mut trace = Trace::take(plan);
    exclude_with(&mut typed, operation, message, crate::needed::engine_need(message), &id, &removed, &mut trace);
    store(plan, &typed, trace).ok()?;
    serde_json::to_value(removed).ok()
}

/// Whether [`exclude`] can take `operation` out of the plan.
pub fn excludable(plan: &Plan, operation: &OpId) -> bool {
    take_out(&mut plan.clone(), operation).is_some()
}

/// Puts the typed plan, then its trace, back into the plan as JSON.
fn store(plan: &mut Value, typed: &Plan, trace: Trace) -> Result<(), String> {
    let result = serde_json::to_value(typed).map(|typed| *plan = typed).map_err(|e| e.to_string());
    trace.store(plan);
    result
}

/// The operation `id` names, taken out of its setup. None if there is no such operation.
fn take_out(plan: &mut Plan, id: &OpId) -> Option<Operation> {
    let setup = plan.setups.iter_mut().find(|s| s.id == id.setup)?;
    let ops = &mut setup.operations;
    let removed = (id.number >= 1 && id.number <= ops.len()).then(|| ops.remove(id.number - 1))?;
    // Refused or not, the finished stock is still compared with the part this holds.
    if removed.cut.kind() == OpKind::Roughing {
        setup.part_surface = Some(Box::new(removed.clone()));
    }
    Some(removed)
}

/// `need` is the tool `message` ends by saying is needed, after `: needs `.
fn exclude_with(plan: &mut Plan, operation: &str, message: &str, need: Option<&str>, id: &OpId, removed: &Operation, trace: &mut Trace) {
    let original = need.and_then(|need| message.strip_suffix(need)?.strip_suffix(": needs ")).unwrap_or(message);
    let brief: String = original.chars().take(160).collect();
    let brief = match need { Some(need) => format!("{brief}: needs {need}"), None => brief };
    let refused = trace.decide("execution.refused", operation, "removed", &format!("refused by the execution engine: {brief}"), json!({"message": message, "operation": OpRef::from(id)}));
    trace.needs(&refused, need);
    // Nothing is covered by roughing that does not run.
    if id.kind == OpKind::Roughing && id.part.is_none() {
        let covered = std::mem::take(&mut plan.covered_by_roughing);
        for id in &covered {
            let not_cut = trace.skip("execution.refused.roughing", id, match need {
                Some(need) => format!("no operation of its own, and roughing was refused: needs {need}"),
                None => "no operation of its own, and roughing was refused".into(),
            });
            trace.needs(&not_cut, need);
            trace.supersede(id, "covered", &not_cut);
        }
    }
    // Its features say why they were not cut.
    for feature in removed.cut.features().into_iter().flatten() {
        let skipped = trace.skip("execution.refused", feature, format!("left out: {brief}"));
        trace.needs(&skipped, need);
    }
    let warned = trace.warn("execution.refused", operation, format!("{operation} left out: {brief}"), json!({"features": removed.cut.features()}));
    trace.needs(&warned, need);
    // Its own decisions no longer stand.
    supersede_own(trace, removed, &refused);
}

const REACH_RULE: &str = "process.roughing.reach";
const REACH_MARGIN_MM: f64 = 1.;

fn reach_reason(rough: &Roughing, mill: &Value) -> Result<String, String> {
    Ok(format!("roughing reaches only Z{} ({} mm below the stock top, {}'s {} mm flutes, {} mm stickout, {} mm clearance)",
        show(rough.floor_z), show(rough.top_z - rough.floor_z),
        id(mill), show(num(mill, "flute_length")?), show(num(mill, "stickout")?), show(REACH_MARGIN_MM)))
}

/// Both the flutes and the holder face must clear the stock by the reach margin.
fn needed_for_reach(op: &Operation, top: f64, library: &Library, depth: f64) -> Result<String, String> {
    let required = depth + REACH_MARGIN_MM;
    let need = crate::needed::for_operation(op, top, library, Some(required))?.ok_or("no reach query for operation")?;
    Ok(format!("{}, and stickout of at least {} mm", need.text, crate::needed::reach(required)))
}

/// The setup's roughing, if it has one.
fn roughing(setup: &Setup) -> Option<&Roughing> {
    setup.operations.iter().find_map(|o| match &o.cut { Cut::Roughing(rough) => Some(rough), _ => None })
}

/// Where the drill enters these holes: after roughing, the lowest roughing layer that clears the
/// disc over their mouth (a cutter that does not fit the hole stops at the last layer its bottom
/// allowance keeps above the mouth, and flats cut only the face around it), or the mouth itself
/// when the roughing cutter fits the hole; otherwise the stock's top as facing leaves it. Clearing
/// no tool cuts is taken out, so it does not count. Slots, notches and V-channels are cut after
/// holes, so a hole in one's floor is entered from that recess's top.
fn drill_entry_z(setup: &Setup, hole: &Holes, library: &Library, group: Option<&str>) -> Result<f64, String> {
    let faced = setup.operations.iter().find_map(|o| match &o.cut { Cut::Facing { floor_z } => Some(*floor_z), _ => None });
    let top = faced.filter(|_| library.clearing("facing").is_ok_and(|t| t.is_some())).unwrap_or(setup.stock.max[2]);
    let roughed = match (roughing(setup), library.clearing("roughing")?) {
        (Some(rough), Some(mill)) => {
            let radial = library.settings.radial_allowance_mm.max(2. * rough.surface_error_mm);
            let bottom = library.settings.bottom_allowance_mm.max(2. * rough.surface_error_mm);
            if num(mill, "diameter")? + 2. * radial < hole.diameter {
                hole.top_z.max(rough.floor_z)
            } else {
                // Layers run down from the roughing top by the stepdown it cuts with, as camlib
                // schedules them, to its floor plus the bottom allowance.
                let step = technology(mill, group).0.get("stepdown_mm").and_then(Value::as_f64).ok_or_else(|| format!("{} has no stepdown_mm", id(mill)))?;
                let layers = ((rough.top_z - hole.top_z - bottom) / step + 1e-9).floor().max(0.);
                (rough.top_z - layers * step).max(rough.floor_z + bottom)
            }
        }
        _ => return Ok(top),
    };
    let near = |a: [f64; 2], b: [f64; 2], half: f64| hole.centers.iter().any(|c| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let t = (((c[0] - a[0]) * dx + (c[1] - a[1]) * dy) / (dx * dx + dy * dy).max(1e-12)).clamp(0., 1.);
        (c[0] - a[0] - t * dx).hypot(c[1] - a[1] - t * dy) <= half
    });
    let recessed = setup.operations.iter().filter_map(|o| match &o.cut {
        Cut::Notch(notch) => {
            let (xs, ys) = (notch.outer.iter().map(|v| v[0]), notch.outer.iter().map(|v| v[1]));
            let (x0, x1) = (xs.clone().fold(f64::MAX, f64::min), xs.fold(f64::MIN, f64::max));
            let (y0, y1) = (ys.clone().fold(f64::MAX, f64::min), ys.fold(f64::MIN, f64::max));
            hole.centers.iter().any(|c| (x0..=x1).contains(&c[0]) && (y0..=y1).contains(&c[1])).then_some(notch.top_z)
        }
        Cut::Slot(Slot { a, b, width, top_z, .. }) | Cut::VChannel(VChannel { a, b, width, top_z, .. }) => near(*a, *b, width / 2.).then_some(*top_z),
        _ => None,
    }).fold(roughed, f64::max);
    Ok(recessed.min(top))
}

/// Where the vise jaws stop side cutting in this setup: jaw top plus clearance. None without jaws.
fn jaw_cap(setup: &Setup) -> Option<f64> {
    let clamping = setup.clamping.as_ref()?;
    clamping["jaw_top_z"].as_f64().map(|z| z + clamping["clearance_mm"].as_f64().unwrap())
}

/// Apply before any execution attempt, while the selected roughing tool is known. Repeating
/// settlement leaves an already capped operation and its trace alone.
fn cap_roughing(plan: &mut Plan, library: &Library, trace: &mut Trace) -> Result<(), String> {
    let Some(mill) = library.clearing("roughing")? else { return Ok(()); };
    let reach = (num(mill, "flute_length")?.min(num(mill, "stickout")?) - REACH_MARGIN_MM).max(0.);
    let depths = &plan.roughing_feature_depths;
    let mut uncovered = vec![];
    for setup in &mut plan.setups {
        let setup_id = setup.id.clone();
        let jaw_floor = jaw_cap(setup).map(r);
        let ops = &mut setup.operations;
        let Some(rough) = ops.iter_mut().find(|o| o.cut.kind() == OpKind::Roughing) else { continue; };
        let Cut::Roughing(cut) = &mut rough.cut else { unreachable!() };
        let top = cut.top_z;
        let old = cut.floor_z;
        let floor = r(jaw_floor.map_or(top - reach, |z| z.max(top - reach)));
        let rule = if jaw_floor == Some(floor) { "machine.clamping" } else { REACH_RULE };
        if old >= floor { continue; }
        cut.floor_z = floor;
        cut.web_mm = r(cut.web_mm + floor - old);
        let why = if rule == "machine.clamping" {
            format!("side cutting stops at Z{}: jaw top plus {} mm clearance; the gripped stock and material below the jaws need a later setup", show(floor), show(crate::rules::workholding()["clearance_mm"].as_f64().unwrap()))
        } else { reach_reason(cut, mill)? };
        let part_top = cut.positions.as_ref().and_then(|p| p.iter().skip(2).step_by(3).copied().max_by(f64::total_cmp)).unwrap_or(top);
        let mut extras = json!({"features": rough.cut.features()});
        if rule == "machine.clamping" { extras["jaw_cap_z"] = json!(floor); }
        let decision = trace.warn(rule, &format!("{setup_id}:roughing"), format!("{why}; stock below this level is left uncut"), extras);
        because_op(rough, [decision]);
        if let Some(depths) = depths {
            for (id, depth) in depths {
                if depth.floor_z >= floor { continue; }
                let required = top - depth.floor_z;
                let need = if rule == REACH_RULE { Some(needed_for_reach(rough, top, library, required)?) } else { None };
                let suffix = need.as_ref().map(|need| format!(": needs {need}")).unwrap_or_default();
                uncovered.push((rule.to_string(), id.clone(), format!("{}: no operation of its own; {why}{suffix}", depth.family), need));
            }
        }
        // Use the original flats, before filtering, to recover the bands made by
        // wall_operations. A crossing band can still finish its reachable upper section.
        let levels: Vec<f64> = ops.iter().flat_map(|o| match &o.cut { Cut::Flats { levels, .. } => levels.as_slice(), _ => &[] }).copied().collect();
        for wall in ops.iter_mut() {
            let Cut::WallFinish(cut) = &wall.cut else { continue; };
            let low = cut.floor_z;
            if low >= floor { continue; }
            let high = levels.iter().copied().filter(|z| *z > low).min_by(f64::total_cmp).unwrap_or(part_top);
            // Leave wholly unreachable bands to wall_refusal and the existing needs_wall handling.
            if high <= floor { continue; }
            let need = if rule == REACH_RULE { Some(needed_for_reach(wall, top, library, top - low)?) } else { None };
            let suffix = need.as_ref().map(|need| format!(": needs {need}")).unwrap_or_default();
            let Cut::WallFinish(cut) = &mut wall.cut else { unreachable!() };
            cut.floor_z = floor;
            // It now stops in the roughed stock, above the flat it originally stood on.
            cut.on_flat = false;
            let subject = format!("{setup_id}:{}", cut.wall);
            let mut extras = json!({"features": wall.cut.features()});
            if rule == "machine.clamping" { extras["jaw_cap_z"] = json!(floor); }
            let d = trace.warn(rule, &subject,
                format!("outside wall limited to Z{}: {why}; its finishing allowance below this level is left uncut{suffix}", show(floor)), extras);
            trace.needs(&d, need.as_deref());
            because_op(wall, [d]);
        }
        for op in ops.iter_mut() {
            let Cut::Flats { floor_z, levels } = &mut op.cut else { continue; };
            let before = levels.len();
            levels.retain(|z| *z >= floor);
            let changed = levels.len() != before;
            *floor_z = floor;
            if changed {
                let mut extras = json!({"features": op.cut.features()});
                if rule == "machine.clamping" { extras["jaw_cap_z"] = json!(floor); }
                let d = trace.warn(rule, &format!("{setup_id}:flats"), format!("flats below Z{} left out: {why}", show(floor)), extras);
                because_op(op, [d]);
            }
        }
        ops.retain(|o| !matches!(&o.cut, Cut::Flats { levels, .. } if levels.is_empty()));
        if reach == 0. {
            let index = ops.iter().position(|o| o.cut.kind() == OpKind::Roughing).unwrap();
            setup.part_surface = Some(Box::new(ops.remove(index)));
        }
    }
    let covered = &mut plan.covered_by_roughing;
    for (rule, id, reason, need) in uncovered {
        if !covered.contains(&id) { continue; }
        covered.retain(|f| f != &id);
        let skipped = trace.skip(&rule, &id, reason);
        trace.needs(&skipped, need.as_deref());
        trace.supersede(&id, "covered", &skipped);
    }
    Ok(())
}

/// A refusal of a wall or V-channel: the rule, why, and the tool the why ends by saying is needed
/// (after `: needs `), if it names one.
type Refusal = (&'static str, String, Option<String>);

/// Why this outside wall cannot be finished, if it cannot. A contour knows only its outline: it has
/// to follow a roughing that cleared the stock beside the wall, with room for the cutter and for
/// the allowance roughing leaves on whatever stands opposite.
fn wall_refusal(setup: &Setup, op: &Operation, wall: &WallFinish, library: &Library) -> Result<Option<Refusal>, String> {
    let Some(rough) = roughing(setup) else {
        return Ok(Some(("feasibility.wall.needs_roughing", "no roughing has cleared the stock beside the wall, so it is not finished".into(), None)));
    };
    if wall.on_flat && !setup.operations.iter().any(|o| o.cut.kind() == OpKind::Flats) {
        return Ok(Some(("feasibility.wall.needs_flat", "the flat this wall stands on is not cut, so it is not finished".into(), None)));
    }
    let Some(mill) = library.clearing("roughing")? else {
        let need = crate::needed::for_operation(op, setup.stock.max[2], library, None)?.unwrap().text;
        return Ok(Some(("feasibility.wall.needs_mill", format!("no end mill to finish the wall with: needs {need}"), Some(need))));
    };
    if wall.floor_z < rough.floor_z {
        if jaw_cap(setup).is_some_and(|cap| wall.floor_z < cap) {
            return Ok(Some(("machine.clamping", "this wall band is below the jaw clearance cap; left for a later setup".into(), None)));
        }
        return Ok(Some((REACH_RULE, format!("{}; none of this wall band remains above the cleared stock", reach_reason(rough, mill)?), None)));
    }
    let radial = library.settings.radial_allowance_mm.max(2. * rough.surface_error_mm);
    let needed = num(mill, "diameter")? + 2. * radial;
    let Some(room) = wall.room.filter(|room| *room < needed) else { return Ok(None); };
    let mut why = format!("{} mm beside the wall is too narrow for {} (it needs {}), so it keeps its roughing allowance", show(room), id(mill), show(r(needed)));
    let maximum = ((room - 2. * radial) * 100.).floor() / 100.;
    let mut needs = None;
    if maximum > 0. {
        let need = crate::needed::for_operation(op, setup.stock.max[2], library, None)?.unwrap().text;
        let need = format!("{need}, diameter at most {maximum:.2} mm to leave room for roughing allowance");
        why.push_str(&format!(": needs {need}"));
        needs = Some(need);
    }
    Ok(Some(("feasibility.wall.room", why, needs)))
}

/// The stock a V-bit run with its tip on the channel's root leaves on each wall, measured across
/// the channel's top: its half-width less the bit's half-width at the channel's depth.
fn v_channel_wall_stock(channel: &VChannel, bit: &Value) -> Result<f64, String> {
    let tip = bit.get("tip_diameter").and_then(Value::as_f64).unwrap_or(0.);
    let half = (num(bit, "angle")? / 2.).to_radians().tan();
    Ok((channel.width - tip) / 2. - channel.depth * half)
}

/// Why this V-channel cannot be cut, if it cannot.
fn v_channel_refusal(setup: &Setup, op: &Operation, channel: &VChannel, library: &Library) -> Result<Option<Refusal>, String> {
    if channel.on_flat
        && let Some(rough) = roughing(setup).filter(|rough| channel.top_z < rough.floor_z)
        && let Some(mill) = library.clearing("roughing")? {
        return Ok(Some((REACH_RULE, format!("the flat it is cut in is below the cleared stock; {}", reach_reason(rough, mill)?), None)));
    }
    if channel.on_flat && !setup.operations.iter().any(|o| o.cut.kind() == OpKind::Flats) {
        return Ok(Some(("feasibility.v_channel.needs_flat", "the flat it is cut in has not been cut, so it is not cut".into(), None)));
    }
    let Some(bit) = library.v_bit(channel.angle) else {
        let need = crate::needed::for_operation(op, setup.stock.max[2], library, None)?.unwrap().text;
        return Ok(Some(("feasibility.v_channel.needs_bit", format!("no sharp V-bit of {}° or less: needs {need}", show(channel.angle)), Some(need))));
    };
    if bit.get("stepdown_mm").and_then(Value::as_f64).is_none() {
        return Ok(Some(("feasibility.v_channel.needs_stepdown", format!("{} has no stepdown_mm", id(bit)), None)));
    }
    let clear = num(bit, "diameter")? / 2. - channel.width / 2.;
    let Some(room) = channel.room.filter(|room| *room < clear) else { return Ok(None); };
    let mut why = format!("{} mm beside it is too narrow for {}", show(room), id(bit));
    let maximum = ((channel.width + 2. * room) * 100.).floor() / 100.;
    let mut needs = None;
    if maximum > 0. {
        let need = format!("{}, diameter at most {maximum:.2} mm", library.needed("v_bit", &[("angle", channel.angle)])?);
        why.push_str(&format!(": needs {need}"));
        needs = Some(need);
    }
    Ok(Some(("feasibility.v_channel.room", why, needs)))
}

/// Takes out of the plan what can no longer be cut: walls and flats with nothing to follow, and an
/// outside chamfer whose wall is not at size, whose rounds go back to not being cut.
pub fn settle(plan: &mut Value, library_data: &Value) -> Result<(), String> {
    let mut typed = Plan::deserialize(&*plan).map_err(|e| format!("the plan does not decode: {e}"))?;
    let mut trace = Trace::take(plan);
    let result = settle_with(&mut typed, library_data, &mut trace);
    // Whatever happened, the plan keeps what it had said and decided.
    store(plan, &typed, trace).and(result)
}

fn settle_with(plan: &mut Plan, library_data: &Value, trace: &mut Trace) -> Result<(), String> {
    let library = Library::new(library_data)?;
    cap_roughing(plan, &library, trace)?;
    for setup in &mut plan.setups {
        let setup_id = setup.id.clone();
        if let Some(cap) = jaw_cap(setup).map(r) {
            let axis = usize::from(setup.clamping.as_ref().unwrap()["faces"][0] == "stock.y.min");
            let low = setup.stock.min[axis];
            let high = setup.stock.max[axis];
            setup.operations.retain_mut(|op| {
                let kind = op.cut.kind();
                let floor = match &op.cut {
                    Cut::Slot(slot) => Some(slot.floor_z),
                    Cut::Notch(notch) => Some(notch.floor_z),
                    Cut::VChannel(channel) => Some(channel.root_z),
                    Cut::EdgeChamfer(chamfer) if chamfer.boundaries != "holes" => {
                        library.chamfer_mill(chamfer.width).and_then(|tool| {
                            let half = (tool["angle"].as_f64()? / 2.).to_radians().tan();
                            Some(chamfer.top_z - (chamfer.width + crate::resources::TIP_CLEARANCE) / half)
                        })
                    }
                    _ => None,
                };
                if floor.is_none_or(|z| z >= cap) { return true; }
                let edge = match &op.cut {
                    Cut::Notch(notch) => notch.outer.iter().map(|p| p[axis]).any(|v| v <= low || v >= high),
                    Cut::Slot(Slot { width, a, b, .. }) | Cut::VChannel(VChannel { width, a, b, .. }) => {
                        let half = width / 2.;
                        [a, b].iter().map(|p| p[axis]).any(|v| v - half <= low || v + half >= high)
                    }
                    _ => true,
                };
                if !edge { return true; }
                let reason = format!("{kind} material below Z{} jaw clearance cap left for a later setup", show(cap));
                let d = trace.warn("machine.clamping", &setup_id, reason.clone(), json!({"features": op.cut.features(), "jaw_cap_z": cap}));
                for feature in op.cut.features().into_iter().flatten() {
                    trace.skip("machine.clamping", feature, reason.clone());
                }
                match &mut op.cut {
                    Cut::Slot(Slot { top_z, floor_z, bottom, .. }) | Cut::Notch(Notch { top_z, floor_z, bottom, .. }) if *top_z > cap => {
                        *floor_z = cap;
                        // It no longer breaks through: job generation must not extend below the cap.
                        *bottom = "blind".into();
                    }
                    _ => return false,
                }
                because_op(op, [d]);
                true
            });
        }
        let mut refused = vec![];
        for op in &setup.operations {
            let Cut::WallFinish(wall) = &op.cut else { continue; };
            if let Some((rule, why, need)) = wall_refusal(setup, op, wall, &library)? {
                let subject = format!("{setup_id}:{}", wall.wall);
                let warned = trace.warn(rule, &subject, format!("setup {setup_id}: outside wall above Z{} left out: {why}", show(wall.floor_z)), json!({"features": op.cut.features()}));
                trace.needs(&warned, need.as_deref());
                refused.push(wall.wall.clone());
            }
        }
        let mut no_channel = vec![];
        for (n, op) in setup.operations.iter().enumerate() {
            let Cut::VChannel(channel) = &op.cut else { continue; };
            if let Some((rule, why, need)) = v_channel_refusal(setup, op, channel, &library)? {
                let warned = trace.warn(rule, &format!("{setup_id}:v_channel"), format!("setup {setup_id}: V-channel left out: {why}"), json!({"features": op.cut.features()}));
                trace.needs(&warned, need.as_deref());
                for id in &channel.features {
                    let skipped = trace.skip(rule, id, format!("V-channel left out: {why}"));
                    trace.needs(&skipped, need.as_deref());
                }
                no_channel.push(n);
            }
        }
        let ops = &mut setup.operations;
        for n in no_channel.into_iter().rev() {
            ops.remove(n);
        }
        let rough = ops.iter().any(|o| o.cut.kind() == OpKind::Roughing);
        if !rough && ops.iter().any(|o| o.cut.kind() == OpKind::Flats) {
            trace.decide("feasibility.flats.need_roughing", &format!("{setup_id}:flats"), "removed", "flats follow roughing, which is not cut in this setup", Value::Null);
        }
        ops.retain(|o| !matches!(&o.cut, Cut::WallFinish(wall) if refused.contains(&wall.wall)) && (rough || o.cut.kind() != OpKind::Flats));
        let walls: Vec<String> = ops.iter().filter_map(|o| match &o.cut { Cut::WallFinish(wall) => Some(wall.wall.clone()), _ => None }).collect();
        ops.retain_mut(|op| {
            let Cut::EdgeChamfer(chamfer) = &mut op.cut else { return true; };
            if chamfer.needs_wall.as_ref().is_none_or(|wall| walls.contains(wall)) {
                return true;
            }
            let lost = chamfer.wall_features.clone();
            for id in &lost {
                let edge = chamfer.radius.map_or("edge chamfer".into(), |radius| format!("r{} edge round", show(radius)));
                let reason = if chamfer.radius.is_some() {
                    format!("{edge}: its wall keeps its roughing allowance, so it is not chamfered")
                } else {
                    format!("{edge}: its wall keeps its roughing allowance, so the edge stays sharp (chamfer it by hand); leg {} mm", show(chamfer.width))
                };
                trace.skip("feasibility.chamfer.needs_wall", id, reason);
            }
            if chamfer.boundaries != "all" {
                return false;
            }
            chamfer.boundaries = "holes".into();
            chamfer.needs_wall = None;
            chamfer.features.retain(|f| !lost.contains(f));
            true
        });
    }
    Ok(())
}

/// Names the tool that cuts each operation (`tool`), or says why none can (`tool_note`; null when
/// nothing needs saying). A plan that does not decode, such as one with an operation kind capp does
/// not plan, is an error and is left as it was.
pub fn resolve(plan: &mut Value, library_data: &Value) -> Result<(), String> {
    let mut typed = Plan::deserialize(&*plan).map_err(|e| format!("the plan does not decode: {e}"))?;
    let mut trace = Trace::take(plan);
    let result = resolve_with(&mut typed, library_data, &mut trace);
    // Whatever happened, the plan keeps what it had resolved and decided.
    store(plan, &typed, trace).and(result)
}

fn resolve_with(plan: &mut Plan, library_data: &Value, trace: &mut Trace) -> Result<(), String> {
    let library = Library::new(library_data)?;
    for (tool, why) in &library.too_big {
        trace.decide("machine.spindle.tool_size", id(tool), "removed", why, Value::Null);
    }
    let group = material(plan, library_data, trace);
    let plan_material = plan.material.as_ref().and_then(|m| m["used"].as_str()).map(String::from);
    for setup in &mut plan.setups {
        // Read the setup as it stood before this pass changed any of its operations.
        let view = setup.clone();
        let setup_id = view.id.clone();
        let top = view.stock.max[2];
        let has_rough = view.operations.iter().any(|o| o.cut.kind() == OpKind::Roughing);
        // Operations before this one that no tool cuts: finish takes them out, so a supplied
        // choice names this one by its number without them.
        let mut untooled = 0;
        for (index, op) in setup.operations.iter_mut().enumerate() {
            let op_kind = op.cut.kind();
            let kind = op_kind.name();
            let features = || op.cut.features().into_iter().flatten().map(String::as_str).collect::<Vec<_>>().join(", ");
            // The tool, or why there is none (None: nothing to say); the rule; the candidates
            // turned down, and why; the tool the why ends by saying is needed.
            let mut considered: Vec<Considered> = vec![];
            let mut needs: Option<String> = None;
            let (mut chosen, rule): (Result<&Value, Option<String>>, &str) = match &op.cut {
                Cut::Facing { .. } | Cut::Roughing(_) => {
                    if library.clearing_tool(kind).is_none() {
                        considered = library.candidates("largest_end_mill", &[]);
                    }
                    (library.clearing(kind)?.ok_or_else(|| Some(format!("no end mill to {} with", if op_kind == OpKind::Facing { "face" } else { "rough" }))), "tool.clearing")
                }
                Cut::Flats { .. } => match (has_rough, library.clearing("roughing")?) {
                    (true, Some(s)) => (Ok(s), "tool.flats.follow_roughing"),
                    (true, None) => {
                        let need = crate::needed::for_operation(op, top, &library, None)?.unwrap().text;
                        let why = format!("left out: it follows roughing, with the same end mill: needs {need}");
                        needs = Some(need);
                        (Err(Some(why)), "tool.flats.follow_roughing")
                    }
                    _ => (Err(Some("left out: it follows roughing, with the same end mill".into())), "tool.flats.follow_roughing"),
                },
                Cut::SpotDrill(spot) => {
                    let selected = spot.spot_tool.as_deref().and_then(|name| library.data["tools"].as_array()?.iter().find(|t| id(t) == name));
                    let need = crate::needed::for_operation(op, top, &library, None)?.ok_or("no spot-drill query")?.text;
                    let tool = selected.ok_or_else(|| Some(format!("no usable spot drill: needs {need}")));
                    needs = Some(need);
                    (tool, "tool.spot_drill")
                }
                Cut::Holes(hole) => {
                    let d = hole.diameter;
                    let depth = library.drill_depth(hole, top);
                    let drilled = library.drill(d, depth);
                    considered = library.candidates("drill", &[("diameter", d), ("depth", depth)]);
                    if drilled.is_none() {
                        // Bored: the end mills decided it, so they come first.
                        let drills = std::mem::take(&mut considered);
                        considered = library.candidates("bore_mill", &[("diameter", d)]);
                        considered.extend(drills);
                    }
                    let tool = drilled.or_else(|| library.bore_mill(d)).or_else(|| library.deferred_drill(d, depth)).ok_or_else(|| Some(format!("no drill of ⌀{} and no end mill small enough to bore it ({})", show(d), features())));
                    (tool, "tool.hole.drill_or_bore")
                }
                Cut::Counterbore(counterbore) => {
                    let d = counterbore.diameter;
                    considered = library.candidates("bore_mill", &[("diameter", d)]);
                    (library.bore_mill(d).ok_or_else(|| Some(format!("no end mill small enough for the ⌀{} counterbore", show(d)))), "tool.counterbore.bore_mill")
                }
                Cut::Thread(thread) => {
                    let (tap, taps) = library.tap(thread, top, library.thread_open_z(thread, Some(&view), top)?, group.as_deref())?;
                    let (mill, why) = library.thread_mill(thread, top)?;
                    considered = why.clone();
                    considered.extend(taps);
                    if tap.is_some() && let Some(mill) = mill {
                        considered.push(Considered::passed_over(id(mill), "qualifies, but matching small-thread tap in ISO N material is preferred"));
                    }
                    let tool = tap.or(mill).ok_or_else(|| {
                        let texts: Vec<String> = why.iter().map(|c| format!("{}: {}", c.candidate, c.rejected)).collect();
                        let reasons = if texts.is_empty() { String::new() } else { format!(" ({})", texts.join("; ")) };
                        Some(format!("{} ({}): no thread mill can cut it{reasons}", thread.spec, features()))
                    });
                    (tool, if library.has_taps() { "tool.thread.tap_or_mill" } else { "tool.thread_mill" })
                }
                Cut::Slot(slot) => {
                    let width = slot.width;
                    considered = library.candidates("bore_mill", &[("diameter", width)]);
                    (library.bore_mill(width).ok_or_else(|| Some(format!("no end mill small enough for the {} mm slot ({})", show(width), features()))), "tool.slot.bore_mill")
                }
                Cut::Notch(notch) => {
                    let width = notch.width;
                    let depth = notch.top_z - notch.floor_z;
                    considered = library.candidates("pocket_mill", &[("width", width), ("depth", depth)]);
                    let tool = library.pocket_mill(width, depth).or_else(|| library.deferred_pocket_mill(width, depth))
                        .ok_or_else(|| Some(format!("no end mill both narrower than the {} mm width and long enough ({})", show(width), features())));
                    (tool, "tool.notch.pocket_mill")
                }
                Cut::VChannel(channel) => match v_channel_refusal(&view, op, channel, &library)? {
                    Some((rule, why, need)) => {
                        needs = need;
                        (Err(Some(why)), rule)
                    }
                    None => {
                        considered = library.candidates("v_bit", &[("angle", channel.angle)]);
                        (library.v_bit(channel.angle).ok_or(None), "tool.v_channel.v_bit")
                    }
                },
                Cut::WallFinish(wall) => match wall_refusal(&view, op, wall, &library)? {
                    Some((rule, why, need)) => {
                        needs = need;
                        (Err(Some(why)), rule)
                    }
                    None => (library.clearing("roughing")?.ok_or(None), "tool.wall.roughing_mill"),
                },
                Cut::EdgeChamfer(chamfer) => {
                    let width = chamfer.width;
                    let maximum = chamfer.maximum_diameter.flatten().unwrap_or(f64::INFINITY);
                    let room = chamfer.room.flatten().unwrap_or(f64::INFINITY);
                    let depth = top - chamfer.top_z + width + crate::resources::TIP_CLEARANCE;
                    considered = library.candidates("chamfer_mill", &[("width", width), ("maximum_diameter", maximum), ("depth", depth), ("room", room)]);
                    (library.chamfer_mill_in(width, maximum, depth, room).ok_or_else(|| Some(format!("no 90° chamfer mill covers the whole {} mm chamfer with tip clearance, loop clearance and cutting-length reach; width passes cannot extend its cone", show(width)))), "tool.edge_chamfer.chamfer_mill")
                }
            };
            let subject = format!("{setup_id}:{kind}");
            let operation = format!("{setup_id}-{}-{kind}", index + 1 - untooled);
            let mut supplied = None;
            let supplied_rule = if rule == "tool.thread.tap_or_mill" && trace.supplied_for(rule, &subject, Some(&operation)).is_none() && trace.supplied_for("tool.thread_mill", &subject, Some(&operation)).is_some() { "tool.thread_mill" } else { rule };
            if view.operations.iter().filter(|o| o.cut.kind() == op_kind).count() > 1
                && let Some(n) = trace.supplied_for(supplied_rule, &subject, None) {
                trace.refuse(n, "subject matches more than one operation; name operation explicitly");
            }
            if let Some(n) = trace.supplied_for(supplied_rule, &subject, Some(&operation))
                && crate::supplied::supported(rule) {
                    match library.supplied_thread_tool(op, top, &trace.supplied[n]["choice"], Some(&view), group.as_deref()) {
                        Ok((tool, candidates)) => {
                            considered = candidates;
                            chosen = Ok(tool);
                            supplied = Some(n);
                        }
                        Err(reason) => trace.refuse(n, &reason),
                    }
            }
            // A tap cuts a default blind thread only as deep as its lead leaves room for.
            let mut shortened = None;
            if let (Ok(tool), Cut::Thread(thread)) = (&chosen, &mut op.cut) && tool["type"] == "tap"
                && let Some(bottom) = library.tap_bottom(tool, thread, library.thread_open_z(thread, Some(&view), top)?)?
                && bottom != thread.bottom_z {
                shortened = Some((r(thread.top_z - thread.bottom_z), r(thread.top_z - bottom)));
                thread.bottom_z = bottom;
            }
            let (tool, note, decided) = match chosen {
                Ok(tool) => {
                    let deferred = considered.iter().any(|c| c.candidate == id(tool) && c.deferred);
                    let mut says = if deferred { format!("{} selected; reach is left to camlib's check", id(tool)) } else if tool["type"] == "tap" { format!("{} taps the small metric thread in ISO N material: matching pitch, hand and pre-drill; chamfer, one-pitch bottom clearance and reach fit", id(tool)) } else { format!("{} cuts it", id(tool)) };
                    let mut data = json!({"chosen": id(tool), "considered": considered});
                    if let Some((default, cut)) = shortened {
                        says.push_str(&format!("; the default {} mm full thread is cut {} mm deep, leaving room for its lead and one pitch above the drilled full diameter", show(default), show(cut)));
                        data["full_thread_mm"] = json!(cut);
                    }
                    let decided = trace.decide(rule, &subject, "chosen", &says, data);
                    (Some(id(tool).to_string()), None, decided)
                }
                Err(note) => {
                    let note = match note {
                        Some(text) if needs.is_none() && matches!(rule, "tool.clearing" | "tool.hole.drill_or_bore" | "tool.counterbore.bore_mill" | "tool.thread_mill" | "tool.thread.tap_or_mill" | "tool.slot.bore_mill" | "tool.notch.pocket_mill" | "tool.edge_chamfer.chamfer_mill" | "feasibility.v_channel.needs_bit" | "feasibility.wall.needs_mill") => {
                            Some(match crate::needed::for_operation_with_context(op, top, &library, None, Some(&view), group.as_deref())? {
                                Some(need) => {
                                    let text = format!("{text}: needs {}", need.text);
                                    needs = Some(need.text);
                                    text
                                }
                                None => text,
                            })
                        }
                        other => other,
                    };
                    let says = note.clone().unwrap_or_else(|| "no tool".into());
                    let decided = trace.decide(rule, &subject, "none", &says, json!({"chosen": null, "considered": considered}));
                    trace.needs(&decided, note.as_ref().and(needs.as_deref()));
                    (None, note, decided)
                }
            };
            if let Some(n) = supplied { trace.accept(n, &decided); }
            let tool_type = tool.as_deref().and_then(|t| library.data["tools"].as_array()?.iter().find(|x| x["id"] == t)?["type"].as_str());
            if let Some(names) = crate::reference::iso14649(op, tool_type) {
                op.iso14649 = Some(names);
            }
            let mut why = vec![decided];
            // Feature warnings made before grouping belong to each operation cutting that feature.
            why.extend(trace.decisions.iter().filter(|d| d["outcome"] == "warning" && d.get("superseded_by").is_none()
                && !op.because.as_ref().is_some_and(|ids| ids.iter().any(|id| d["id"] == id.as_str()))
                && d["features"].as_array().is_some_and(|features| features.contains(&d["subject"]) && features.iter().any(|f| op.cut.features().is_some_and(|own| own.iter().any(|o| f == o.as_str())))))
                .filter_map(|d| d["id"].as_str().map(String::from)));
            if let Some(name) = tool.as_deref() {
                let spec = library.data["tools"].as_array().and_then(|t| t.iter().find(|x| x["id"] == name)).ok_or("a chosen tool is not in the library")?;
                // No dog-bone relief: neither analysis nor answers can request it today.
                if let Cut::Notch(notch) = &op.cut {
                    let model = notch.corner_radius.unwrap_or(0.);
                    let radius = num(spec, "diameter")? / 2.;
                    if radius > model + 1e-6 {
                        let corners = notch.inside_corners.map_or(4, u64::from);
                        let shape = if model == 0. { "square inside corners" } else { "inside corners" };
                        let model_note = if model == 0. { "the model has them sharp".to_string() } else { format!("the model has r{}", warning_number(model)) };
                        let feature_names = format!("setup {setup_id}: {} mm notch", warning_number(notch.width));
                        why.push(trace.warn("process.notch.corner_radius", &subject, format!("{}: its {corners} {shape} are left r{} by {name} ({model_note})", feature_names, warning_number(radius)), json!({"features": op.cut.features(), "radius": radius, "model_radius": model, "corners": corners})));
                    }
                }
                if let Cut::VChannel(channel) = &op.cut {
                    let left = v_channel_wall_stock(channel, spec)?;
                    if left > 0.005 {
                        let bit = num(spec, "angle")?;
                        why.push(trace.warn("process.v_channel.wall_stock", &subject, format!("setup {setup_id}: {} mm V-channel: its walls are left up to {left:.3} mm thick at the top by {name}, a {}° bit in the {}° channel",
                            warning_number(channel.width), warning_number(bit), warning_number(channel.angle)),
                            json!({"features": op.cut.features(), "wall_stock_mm": r(left)})));
                    }
                }
                let (mut technology, source) = technology(spec, group.as_deref());
                if spec["type"] == "tap" { technology.remove("feed_mm_min"); }
                let says = match &group {
                    Some(g) if source != "tool" => format!("{name}'s cutting data for ISO 513 group {g}"),
                    Some(g) => format!("{name}'s own cutting data: it has none for ISO 513 group {g}"),
                    None if plan_material.is_none() => format!("{name}'s own cutting data: no material is given"),
                    None => format!("{name}'s own cutting data: the material is in no ISO 513 group"),
                };
                why.push(trace.decide("tool.cutting_data.by_group", &subject, "chosen", &says, json!({"chosen": source})));
                if let Some(capped) = crate::machines::cap_speed(&mut technology, library_data) {
                    why.push(trace.decide("machine.spindle.max_rpm", &subject, "chosen", &capped, json!({"chosen": technology["spindle_rpm"]})));
                }
                if let Cut::Holes(hole) = &op.cut && spec["type"] == "drill" {
                    // The depth in material: from where the drill enters, not the stock top.
                    let depth = library.drill_depth(hole, drill_entry_z(&view, hole, &library, group.as_deref())?);
                    let diameter = num(spec, "diameter")?;
                    let cycle = crate::reference::drilling(depth, diameter, spec);
                    if let Some(ratio) = crate::reference::deep_drilling_ratio(depth, diameter) {
                        why.push(trace.warn("process.hole.peck_by_depth", &subject, format!(
                            "setup {setup_id}: {} mm holes, depth/D {ratio:.1}: deep-hole drilling (pilot plus deep-hole drill) is not planned, so it is drilled with full-retract pecks",
                            warning_number(diameter)), json!({"features": op.cut.features(), "depth_diameter_ratio": ratio})));
                    }
                    why.push(trace.decide("process.hole.peck_by_depth", &subject, "chosen",
                        "Drilling cycle from depth below the surface the drill enters / drill diameter; positive tool peck_mm caps the increment",
                        json!({"chosen": cycle})));
                    technology.extend(cycle.as_object().unwrap().clone());
                }
                if let Cut::WallFinish(wall) = &op.cut {
                    let depth = (top - wall.floor_z).max(0.);
                    let flute = num(spec, "flute_length")?;
                    if depth > 0. {
                        let passes = (depth / flute).ceil().max(1.);
                        let stepdown = depth / passes;
                        technology.insert("stepdown_mm".into(), json!(stepdown));
                        why.push(trace.decide("process.wall_finish.axial_passes", &subject, "chosen",
                            "Finish the wall in the fewest equal axial passes its flute length allows; roughing stepdown does not limit the finishing allowance cut. Camlib still checks reach and stock contact.",
                            json!({"chosen": {"passes": passes as u64, "stepdown_mm": stepdown, "depth_mm": depth, "flute_length_mm": flute}})));
                    }
                }
                op.technology = Some(technology);
            }
            untooled += usize::from(tool.is_none());
            op.tool = Some(tool);
            op.tool_note = Some(note);
            because_op(op, why);
        }
        block_chamfers_without_openings(setup, trace);
    }
    Ok(())
}

/// Resolve every opening first, then gate its mouth chamfer on the actual result.
/// Feature ids survive operation grouping, renumbering and execution exclusions.
/// A combined round-as-chamfer operation is conservatively blocked as a whole.
fn block_chamfers_without_openings(setup: &mut Setup, trace: &mut Trace) {
    // Read the openings as they stand before any chamfer here is blocked.
    let view = setup.operations.clone();
    let subject = format!("{}:edge_chamfer", setup.id);
    for op in &mut setup.operations {
        let Cut::EdgeChamfer(chamfer) = &op.cut else { continue; };
        let blocked = chamfer.needs_openings.iter().find_map(|feature| {
            let opening = view.iter().find(|o| matches!(o.cut, Cut::Holes(_) | Cut::Slot(_))
                && o.cut.features().is_some_and(|f| f.contains(feature)));
            if opening.is_some_and(|o| matches!(o.tool, Some(Some(_)))) { return None; }
            // Why, and the tool it ends by saying is needed.
            let (why, need) = match opening.and_then(|o| o.tool_note.as_ref().and_then(Option::as_deref)) {
                Some(note) => (note, opening.and_then(|o| crate::needed::op_need(&trace.decisions, o))),
                None => match trace.skipped.iter().find(|s| s.feature == *feature) {
                    Some(s) => (s.reason.as_str(), s.decision.as_deref().and_then(|id| crate::needed::need_of(&trace.decisions, id))),
                    None => ("its hole or slot operation was left out", None),
                },
            };
            Some((feature.clone(), why.to_owned(), need.map(String::from)))
        });
        let Some((feature, why, need)) = blocked else { continue; };
        let reason = format!("mouth chamfer: required opening {feature} is not cut, so the edge stays sharp (chamfer it by hand)");
        let (note, need) = match op.tool_note.as_ref().and_then(Option::as_deref).filter(|_| !matches!(op.tool, Some(Some(_)))) {
            Some(own) => {
                let why = need.as_deref().and_then(|need| why.strip_suffix(need)?.strip_suffix(": needs ")).unwrap_or(&why);
                (format!("{reason}: {why}; {own}"), crate::needed::op_need(&trace.decisions, op).map(String::from))
            }
            None => (format!("{reason}: {why}"), need),
        };
        let decided = trace.decide("feasibility.chamfer.needs_opening", &subject, "none", &note, json!({"chosen": null, "features": [feature]}));
        trace.needs(&decided, need.as_deref());
        for decision in &mut trace.decisions {
            if decision["rule"].as_str().is_some_and(|r| r.starts_with("tool.edge_chamfer.chamfer_mill@"))
                && op.because.as_ref().is_some_and(|ids| ids.iter().any(|id| decision["id"] == id.as_str())) {
                decision["superseded_by"] = json!(decided);
            }
        }
        op.tool = Some(None);
        op.tool_note = Some(Some(note));
        op.technology = None;
        because_op(op, [decided]);
    }
}

/// The cutting data a tool's operations use: its own, with what it states for the material's ISO
/// 513 group (`cutting_data`) in their place. And where they came from: `tool`, or
/// `cutting_data.<group>`.
fn technology(tool: &Value, group: Option<&str>) -> (serde_json::Map<String, Value>, String) {
    const KEYS: [&str; 5] = ["feed_mm_min", "spindle_rpm", "plunge_mm_min", "stepdown_mm", "stepover_mm"];
    let mut out = serde_json::Map::new();
    for key in KEYS {
        if let Some(v) = tool.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    let by_group = group.and_then(|g| tool["cutting_data"].get(g).filter(|d| d.is_object()).map(|d| (g, d)));
    let source = match by_group {
        Some((g, data)) => {
            for key in KEYS {
                if let Some(v) = data.get(key) {
                    out.insert(key.into(), v.clone());
                }
            }
            format!("cutting_data.{g}")
        }
        None => "tool".into(),
    };
    out.insert("source".into(), json!(source));
    (out, source)
}

/// The material the part is cut from: Specify's answer, unless the resources override it, and its
/// ISO 513 group. Writes `material` into the plan and returns the group.
fn material(plan: &mut Plan, library: &Value, trace: &mut Trace) -> Option<String> {
    let specified = plan.material.as_ref().and_then(|m| m["specified"].as_str()).map(String::from);
    let over = library["overrides"]["material"].as_str().filter(|m| !m.is_empty()).map(String::from);
    let used = over.clone().or(specified.clone());
    let placed = used.as_deref().and_then(crate::reference::iso513_group);
    let from = if over.is_some() { "the resources' override" } else { "Specify (part.material)" };
    let (outcome, says) = match (&used, &placed) {
        (None, _) => ("none", "no material is given: the tools' own cutting data are used".to_string()),
        (Some(m), None) => ("none", format!("{m}, from {from}, is in no ISO 513 group this can place: the tools' own cutting data are used")),
        (Some(m), Some((g, word))) => ("chosen", format!("{m}, from {from}, is ISO 513 group {g} (by the word \"{word}\")")),
    };
    let group = placed.map(|(g, _)| g);
    trace.decide("requirement.material.group", "part", outcome, &says, json!({"chosen": group}));
    plan.material = Some(json!({"specified": specified, "override": over, "used": used, "iso513": group}));
    group
}

/// Millimetres in warning prose, rounded without trailing zeroes.
fn warning_number(value: f64) -> String {
    crate::plan::r(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v_channel_wall_stock_is_the_half_width_the_bit_does_not_reach() {
        // The stepped plate's channel: 1.8 mm wide, 1.5 deep, so 61.9275° included.
        let channel = VChannel { angle: 61.9275, width: 1.8, depth: 1.5, length: 10., a: [0., 0.], b: [10., 0.],
            top_z: 0., root_z: -1.5, on_flat: false, room: None, features: vec![] };
        let stock = |angle: f64, tip: f64| v_channel_wall_stock(&channel, &json!({"angle": angle, "tip_diameter": tip})).unwrap();
        // 0.9 - 1.5 × tan 15° = 0.9 - 0.40192 = 0.49808.
        assert!((stock(30., 0.) - 0.49808).abs() < 1e-5, "{}", stock(30., 0.));
        // 0.9 - 1.5 × tan 30° = 0.9 - 0.86603 = 0.03397.
        assert!((stock(60., 0.) - 0.03397).abs() < 1e-5, "{}", stock(60., 0.));
        // A bit of the channel's own angle leaves nothing.
        assert!(stock(61.9275, 0.).abs() < 1e-4, "{}", stock(61.9275, 0.));
        // A 0.2 mm flat tip carries the flank 0.1 mm further out.
        assert!((stock(30., 0.2) - 0.39808).abs() < 1e-5, "{}", stock(30., 0.2));
    }

    #[test]
    fn a_drill_enters_where_the_operations_before_it_leave_the_surface() {
        let data: Value = serde_json::from_str(include_str!("../../../demo/tools.json")).unwrap();
        let library = Library::new(&data).unwrap();
        let hole = |diameter: f64, top_z: f64, x: f64| -> Holes { serde_json::from_value(json!({"diameter": diameter, "top_z": top_z, "bore_top_z": top_z,
            "bottom_z": -19., "bottom": "through", "centers": [[x, 10.]], "exit_clear_mm": null})).unwrap() };
        let setup = |ops: Value| -> Setup { serde_json::from_value(json!({"id": "top", "stock": {"min": [0., 0., -26.], "max": [100., 60., 0.]}, "operations": ops})).unwrap() };
        let entry = |ops: Value, hole: Holes, library: &Library| drill_entry_z(&setup(ops), &hole, library, None).unwrap();
        let near = |got: f64, want: f64| assert!((got - want).abs() < 1e-9, "{got} != {want}");
        let facing = json!({"kind": "facing", "floor_z": -1.});
        let roughing = json!({"kind": "roughing", "top_z": 0., "floor_z": -19., "web_mm": 7., "surface_error_mm": 0.});
        let notch = json!({"kind": "notch", "width": 8., "length": 20., "open": false, "outer": [[40., 6., 0.], [60., 6., 0.], [60., 14., 0.], [40., 14., 0.]],
            "top_z": -1., "floor_z": -7., "wall_top": -1., "bottom": "blind"});
        // Unfaced and unroughed: the stock top.
        near(entry(json!([]), hole(3.3, -7., 10.), &library), 0.);
        // Faced only: the faced top, however low the mouth.
        near(entry(json!([facing]), hole(3.3, -7., 10.), &library), -1.);
        // Roughed by em6 (0.2 mm stepdown), too big for the stepped plate's 3.3 mm holes at Z-7:
        // its last layer that keeps the 0.2 mm bottom allowance above the mouth, Z-6.8.
        near(entry(json!([facing, roughing]), hole(3.3, -7., 10.), &library), -6.8);
        // An 8 mm hole takes em6 with its 0.3 mm radial allowance (6.6 mm), so roughing reaches the mouth.
        near(entry(json!([facing, roughing]), hole(8., -7., 10.), &library), -7.);
        // With a 2 mm stepdown the layers are Z-2, -4, -6, -8: the disc over the mouth is left at Z-6.
        let mut coarse = data.clone();
        for tool in coarse["tools"].as_array_mut().unwrap().iter_mut().filter(|t| t["id"] == "em6") { tool["stepdown_mm"] = json!(2.); }
        near(entry(json!([facing, roughing]), hole(3.3, -7., 10.), &Library::new(&coarse).unwrap()), -6.);
        // A mouth below the roughing floor is entered from the floor plus its bottom allowance.
        let shallow = json!({"kind": "roughing", "top_z": 0., "floor_z": -5., "web_mm": 21., "surface_error_mm": 0.});
        near(entry(json!([facing, shallow]), hole(3.3, -7., 10.), &library), -4.8);
        // In the floor of a notch cut after it: the notch's top; beside the notch: as roughing leaves it.
        near(entry(json!([facing, roughing, notch.clone()]), hole(3.3, -7., 50.), &library), -1.);
        near(entry(json!([facing, roughing, notch]), hole(3.3, -7., 10.), &library), -6.8);
    }
}
