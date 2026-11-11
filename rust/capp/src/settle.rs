//! After planning: what can run with these resources, what an execution engine refused, and which
//! tool cuts each operation.
//!
//! [`finish`] takes a plan from [`crate::plan::plan`], the resources and the exclusions an
//! execution engine has reported, and leaves the plan ready to execute: every operation names its
//! tool (`tool`), or says why it has none (`tool_note`, null when nothing needs saying).
use crate::plan::{r, show};
use crate::resources::{Library, id, num};
use crate::trace::{Trace, because};
use serde_json::{Value, json};

/// The plan as it must be executed: settled, then for each exclusion in turn that operation
/// taken out and the plan settled again, then every operation's tool resolved. An exclusion names
/// an operation as the execution engine named it (`top-2-roughing`, numbered in the plan as it
/// stood when that engine saw it) and says why it refused it.
pub fn finish(plan: &mut Value, library: &Value, exclusions: &[Value]) -> Result<(), String> {
    let normalized = crate::resources::normalize(library)?;
    let library = normalized.as_ref();
    crate::supplied::load(plan, library)?;
    let mut trace = Trace::take(plan);
    crate::machines::place(plan, library, &mut trace);
    trace.store(plan);
    settle(plan, library)?;
    crate::spots::create(plan, &Library::new(library)?)?;
    for exclusion in exclusions {
        let operation = exclusion["operation"].as_str().ok_or("an exclusion names no operation")?;
        let message = crate::needed::refusal(plan, operation, exclusion["message"].as_str().unwrap_or(""), exclusion["tool"].as_str(), &exclusion["reach"], &Library::new(library)?)?;
        exclude(plan, operation, &message)
            .ok_or_else(|| format!("excluded operation {operation} is not in the plan"))?;
        // Preserve the engine's unedited evidence independently of the reader-facing wording.
        let mut trace = Trace::take(plan);
        if let Some(decision) = trace.decisions.iter_mut().rev().find(|d| d["rule"] == "execution.refused@1" && d["outcome"] == "removed") {
            decision["message"] = exclusion["message"].clone();
            let refused = decision["id"].clone();
            if let Some(selection) = exclusion["selection"].as_object() {
                let mut detail = selection.clone();
                detail.remove("id");
                detail.insert("superseded_by".into(), refused);
                let selection = &exclusion["selection"];
                trace.decide(selection["rule"].as_str().unwrap().split('@').next().unwrap(), selection["subject"].as_str().unwrap(), "chosen", selection["says"].as_str().unwrap(), json!(detail));
            }
        }
        trace.store(plan);
        settle(plan, library)?;
        crate::spots::prune(plan, &Library::new(library)?, false)?;
    }
    crate::spots::prune(plan, &Library::new(library)?, true)?;
    resolve(plan, library)?;
    let mut trace = Trace::take(plan);
    crate::clamping_history::check(plan, library, &mut trace);
    crate::machines::check_changer(plan, library, &mut trace);
    trace.refuse_unused();
    trace.store(plan);
    crate::needed::collect(plan);
    Ok(())
}

/// Takes an operation named as the execution engine names them (`top-2-roughing`) out of the
/// plan, and says what that leaves uncut. None if there is no such operation.
pub fn exclude(plan: &mut Value, operation: &str, message: &str) -> Option<Value> {
    let mut parts = operation.splitn(3, '-');
    let (setup_id, number) = (parts.next()?, parts.next()?.parse::<usize>().ok()?);
    let setup = plan["setups"].as_array_mut()?.iter_mut().find(|s| s["id"] == setup_id)?;
    let ops = setup["operations"].as_array_mut()?;
    let removed = (number >= 1 && number <= ops.len()).then(|| ops.remove(number - 1))?;
    // Refused or not, the finished stock is still compared with the part this holds.
    if removed["kind"] == "roughing" {
        setup["part_surface"] = removed.clone();
    }
    let mut trace = Trace::take(plan);
    let (original, need) = message.rsplit_once(": needs ").map_or((message, None), |(text, need)| (text, Some(need)));
    let brief: String = original.chars().take(160).collect();
    let brief = match need { Some(need) => format!("{brief}: needs {need}"), None => brief };
    let refused = trace.decide("execution.refused", operation, "removed", &format!("refused by the execution engine: {brief}"), json!({"message": message}));
    // Nothing is covered by roughing that does not run.
    if operation.ends_with("-roughing") {
        let covered = plan["covered_by_roughing"].as_array_mut().map(std::mem::take).unwrap_or_default();
        for id in covered.iter().filter_map(Value::as_str) {
            let not_cut = trace.skip("execution.refused.roughing", id, match need {
                Some(need) => format!("no operation of its own, and roughing was refused: needs {need}"),
                None => "no operation of its own, and roughing was refused".into(),
            });
            trace.supersede(id, "covered", &not_cut);
        }
    }
    // Its features say why they were not cut.
    for feature in removed["features"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        trace.skip("execution.refused", feature, format!("left out: {brief}"));
    }
    trace.warn("execution.refused", operation, format!("{operation} left out: {brief}"), json!({"features": removed["features"]}));
    // Its own decisions (what made it, its tool and cutting data) no longer stand; those it shares
    // with the rest of the setup (the setup, the order, where its holes are cut) do.
    // Feature-subject warnings are borrowed by every operation cutting that feature.
    let own: Vec<&str> = removed["because"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    for d in trace.decisions.iter_mut().filter(|d| d["id"].as_str().is_some_and(|id| own.contains(&id))
        && (d["stage"] == "process" || d["stage"] == "tools")
        && !(d["outcome"] == "warning" && d["features"].as_array().is_some_and(|features| features.contains(&d["subject"])))) {
        d["superseded_by"] = json!(refused);
    }
    trace.store(plan);
    Some(removed)
}

const REACH_RULE: &str = "process.roughing.reach";
const REACH_MARGIN_MM: f64 = 1.;

fn reach_reason(rough: &Value, mill: &Value) -> Result<String, String> {
    Ok(format!("roughing reaches only Z{} ({} mm below the stock top, {}'s {} mm flutes, {} mm stickout, {} mm clearance)",
        show(num(rough, "floor_z")?), show(num(rough, "top_z")? - num(rough, "floor_z")?),
        id(mill), show(num(mill, "flute_length")?), show(num(mill, "stickout")?), show(REACH_MARGIN_MM)))
}

/// Both the flutes and the holder face must clear the stock by the reach margin.
fn needed_for_reach(op: &Value, top: f64, library: &Library, depth: f64) -> Result<String, String> {
    let required = depth + REACH_MARGIN_MM;
    let need = crate::needed::for_operation(op, top, library, Some(required))?.ok_or("no reach query for operation")?;
    Ok(format!("{need}, and stickout of at least {} mm", crate::needed::reach(required)))
}

/// Apply before any execution attempt, while the selected roughing tool is known. Repeating
/// settlement leaves an already capped operation and its trace alone.
fn cap_roughing(plan: &mut Value, library: &Library, trace: &mut Trace) -> Result<(), String> {
    let Some(mill) = library.clearing("roughing")? else { return Ok(()); };
    let reach = (num(mill, "flute_length")?.min(num(mill, "stickout")?) - REACH_MARGIN_MM).max(0.);
    let depths = plan["roughing_feature_depths"].clone();
    let mut uncovered = vec![];
    for setup in plan["setups"].as_array_mut().ok_or("plan has no setups")? {
        let setup_id = setup["id"].as_str().unwrap_or("").to_string();
        let jaw_floor = setup["clamping"]["jaw_top_z"].as_f64().map(|z| r(z + setup["clamping"]["clearance_mm"].as_f64().unwrap()));
        let ops = setup["operations"].as_array_mut().ok_or("setup has no operations")?;
        let Some(rough) = ops.iter_mut().find(|o| o["kind"] == "roughing") else { continue; };
        // Minimal hand-written plans may have no geometry; there is no floor to cap there.
        let Some(top) = rough["top_z"].as_f64() else { continue; };
        let old = num(rough, "floor_z")?;
        let floor = r(jaw_floor.map_or(top - reach, |z| z.max(top - reach)));
        let rule = if jaw_floor == Some(floor) { "machine.clamping" } else { REACH_RULE };
        if old >= floor { continue; }
        rough["floor_z"] = json!(floor);
        if let Some(web) = rough["web_mm"].as_f64() {
            rough["web_mm"] = json!(r(web + floor - old));
        }
        let why = if rule == "machine.clamping" {
            format!("side cutting stops at Z{}: jaw top plus {} mm clearance; the gripped stock and material below the jaws need a later setup", show(floor), show(crate::rules::workholding()["clearance_mm"].as_f64().unwrap()))
        } else { reach_reason(rough, mill)? };
        let mut extras = json!({"features": rough["features"]});
        if rule == "machine.clamping" { extras["jaw_cap_z"] = json!(floor); }
        let decision = trace.warn(rule, &format!("{setup_id}:roughing"), format!("{why}; stock below this level is left uncut"), extras);
        because(rough, [decision]);
        if let Some(depths) = depths.as_object() {
            for (id, depth) in depths {
                if depth["floor_z"].as_f64().is_some_and(|z| z >= floor) { continue; }
                let required = top - depth["floor_z"].as_f64().unwrap_or(old);
                let need = if rule == REACH_RULE { format!(": needs {}", needed_for_reach(rough, top, library, required)?) } else { String::new() };
                uncovered.push((rule.to_string(), id.clone(), format!("{}: no operation of its own; {why}{need}", depth["family"].as_str().unwrap_or("feature"))));
            }
        }
        // Use the original flats, before filtering, to recover the bands made by
        // wall_operations. A crossing band can still finish its reachable upper section.
        let part_top = rough["positions"].as_array().and_then(|p| p.iter().skip(2).step_by(3)
            .filter_map(Value::as_f64).max_by(f64::total_cmp)).unwrap_or(top);
        let levels: Vec<f64> = ops.iter().filter(|o| o["kind"] == "flats")
            .flat_map(|o| o["levels"].as_array().into_iter().flatten()).filter_map(Value::as_f64).collect();
        for wall in ops.iter_mut().filter(|o| o["kind"] == "wall_finish") {
            let low = num(wall, "floor_z")?;
            if low >= floor { continue; }
            let high = levels.iter().copied().filter(|z| *z > low).min_by(f64::total_cmp).unwrap_or(part_top);
            // Leave wholly unreachable bands to wall_refusal and the existing needs_wall handling.
            if high <= floor { continue; }
            let need = if rule == REACH_RULE { format!(": needs {}", needed_for_reach(wall, top, library, top - low)?) } else { String::new() };
            wall["floor_z"] = json!(floor);
            // It now stops in the roughed stock, above the flat it originally stood on.
            wall["on_flat"] = json!(false);
            let mut extras = json!({"features": wall["features"]});
            if rule == "machine.clamping" { extras["jaw_cap_z"] = json!(floor); }
            let d = trace.warn(rule, &format!("{setup_id}:{}", wall["wall"].as_str().unwrap_or("")),
                format!("outside wall limited to Z{}: {why}; its finishing allowance below this level is left uncut{need}", show(floor)), extras);
            because(wall, [d]);
        }
        for op in ops.iter_mut().filter(|o| o["kind"] == "flats") {
            if let Some(levels) = op["levels"].as_array_mut() {
                let before = levels.len();
                levels.retain(|z| z.as_f64().is_some_and(|z| z >= floor));
                if levels.len() != before {
                    let mut extras = json!({"features": op["features"]});
                    if rule == "machine.clamping" { extras["jaw_cap_z"] = json!(floor); }
                    let d = trace.warn(rule, &format!("{setup_id}:flats"), format!("flats below Z{} left out: {why}", show(floor)), extras);
                    because(op, [d]);
                }
            }
            op["floor_z"] = json!(floor);
        }
        ops.retain(|o| o["kind"] != "flats" || o["levels"].as_array().is_some_and(|levels| !levels.is_empty()));
        if reach == 0. {
            let index = ops.iter().position(|o| o["kind"] == "roughing").unwrap();
            setup["part_surface"] = ops.remove(index);
        }
    }
    if let Some(covered) = plan["covered_by_roughing"].as_array_mut() {
        for (rule, id, reason) in uncovered {
            if !covered.contains(&json!(id)) { continue; }
            covered.retain(|f| f != &id);
            let skipped = trace.skip(&rule, &id, reason);
            trace.supersede(&id, "covered", &skipped);
        }
    }
    Ok(())
}

/// Why this outside wall cannot be finished, if it cannot. A contour knows only its outline: it has
/// to follow a roughing that cleared the stock beside the wall, with room for the cutter and for
/// the allowance roughing leaves on whatever stands opposite.
fn wall_refusal(setup: &Value, op: &Value, library: &Library) -> Result<Option<(&'static str, String)>, String> {
    let rough = setup["operations"].as_array().and_then(|ops| ops.iter().find(|o| o["kind"] == "roughing"));
    let Some(rough) = rough else {
        return Ok(Some(("feasibility.wall.needs_roughing", "no roughing has cleared the stock beside the wall, so it is not finished".into())));
    };
    if op["on_flat"] == true && !setup["operations"].as_array().is_some_and(|ops| ops.iter().any(|o| o["kind"] == "flats")) {
        return Ok(Some(("feasibility.wall.needs_flat", "the flat this wall stands on is not cut, so it is not finished".into())));
    }
    let Some(mill) = library.clearing("roughing")? else {
        return Ok(Some(("feasibility.wall.needs_mill", format!("no end mill to finish the wall with: needs {}", crate::needed::for_operation(op, setup["stock"]["max"][2].as_f64().ok_or("bad stock")?, library, None)?.unwrap()))));
    };
    if num(op, "floor_z")? < num(rough, "floor_z")? {
        if setup["clamping"]["jaw_top_z"].as_f64().is_some_and(|z| num(op, "floor_z").is_ok_and(|floor| floor < z + setup["clamping"]["clearance_mm"].as_f64().unwrap())) {
            return Ok(Some(("machine.clamping", "this wall band is below the jaw clearance cap; left for a later setup".into())));
        }
        return Ok(Some((REACH_RULE, format!("{}; none of this wall band remains above the cleared stock", reach_reason(rough, mill)?))));
    }
    let radial = library.data.get("radial_allowance_mm").and_then(Value::as_f64).unwrap_or(0.3).max(2. * num(rough, "surface_error_mm")?);
    let needed = num(mill, "diameter")? + 2. * radial;
    let Some(room) = op["room"].as_f64().filter(|room| *room < needed) else { return Ok(None); };
    let mut why = format!("{} mm beside the wall is too narrow for {} (it needs {}), so it keeps its roughing allowance", show(room), id(mill), show(r(needed)));
    let maximum = ((room - 2. * radial) * 100.).floor() / 100.;
    if maximum > 0. {
        let top = setup["stock"]["max"][2].as_f64().ok_or("bad stock")?;
        let need = crate::needed::for_operation(op, top, library, None)?.unwrap();
        why.push_str(&format!(": needs {need}, diameter at most {maximum:.2} mm to leave room for roughing allowance"));
    }
    Ok(Some(("feasibility.wall.room", why)))
}

/// Why this V-channel cannot be cut, if it cannot.
fn v_channel_refusal(setup: &Value, op: &Value, library: &Library) -> Result<Option<(&'static str, String)>, String> {
    if op["on_flat"] == true {
        let rough = setup["operations"].as_array().and_then(|ops| ops.iter().find(|o| o["kind"] == "roughing"));
        if let Some(rough) = rough.filter(|rough| op["top_z"].as_f64().zip(rough["floor_z"].as_f64()).is_some_and(|(top, floor)| top < floor))
            && let Some(mill) = library.clearing("roughing")? {
            return Ok(Some((REACH_RULE, format!("the flat it is cut in is below the cleared stock; {}", reach_reason(rough, mill)?))));
        }
    }
    if op["on_flat"] == true && !setup["operations"].as_array().is_some_and(|ops| ops.iter().any(|o| o["kind"] == "flats")) {
        return Ok(Some(("feasibility.v_channel.needs_flat", "the flat it is cut in has not been cut, so it is not cut".into())));
    }
    let Some(bit) = library.v_bit(num(op, "angle")?) else {
        return Ok(Some(("feasibility.v_channel.needs_bit", format!("no sharp V-bit of {}° or less: needs {}", show(num(op, "angle")?), crate::needed::for_operation(op, setup["stock"]["max"][2].as_f64().ok_or("bad stock")?, library, None)?.unwrap()))));
    };
    if bit.get("stepdown_mm").and_then(Value::as_f64).is_none() {
        return Ok(Some(("feasibility.v_channel.needs_stepdown", format!("{} has no stepdown_mm", id(bit)))));
    }
    let clear = num(bit, "diameter")? / 2. - num(op, "width")? / 2.;
    let Some(room) = op["room"].as_f64().filter(|room| *room < clear) else { return Ok(None); };
    let mut why = format!("{} mm beside it is too narrow for {}", show(room), id(bit));
    let maximum = ((num(op, "width")? + 2. * room) * 100.).floor() / 100.;
    if maximum > 0. {
        why.push_str(&format!(": needs {}, diameter at most {maximum:.2} mm", library.needed("v_bit", &[("angle", num(op, "angle")?)])?));
    }
    Ok(Some(("feasibility.v_channel.room", why)))
}

/// Takes out of the plan what can no longer be cut: walls and flats with nothing to follow, and an
/// outside chamfer whose wall is not at size, whose rounds go back to not being cut.
pub fn settle(plan: &mut Value, library_data: &Value) -> Result<(), String> {
    let mut trace = Trace::take(plan);
    let result = settle_with(plan, library_data, &mut trace);
    // Whatever happened, the plan keeps what it had said and decided.
    trace.store(plan);
    result
}

fn settle_with(plan: &mut Value, library_data: &Value, trace: &mut Trace) -> Result<(), String> {
    let library = Library::new(library_data)?;
    cap_roughing(plan, &library, trace)?;
    for setup in plan["setups"].as_array_mut().ok_or("plan has no setups")? {
        let setup_id = setup["id"].as_str().unwrap_or("").to_string();
        if let Some(cap) = setup["clamping"]["jaw_top_z"].as_f64().map(|z| r(z + setup["clamping"]["clearance_mm"].as_f64().unwrap())) {
            let axis = usize::from(setup["clamping"]["faces"][0] == "stock.y.min");
            let low = setup["stock"]["min"][axis].as_f64().unwrap();
            let high = setup["stock"]["max"][axis].as_f64().unwrap();
            let ops = setup["operations"].as_array_mut().ok_or("setup has no operations")?;
            ops.retain_mut(|op| {
                let kind = op["kind"].as_str().unwrap_or("").to_string();
                let floor = match kind.as_str() {
                    "slot" | "notch" => op["floor_z"].as_f64(),
                    "v_channel" => op["root_z"].as_f64(),
                    "edge_chamfer" if op["boundaries"] != "holes" => {
                        op["width"].as_f64().and_then(|width| library.chamfer_mill(width).and_then(|tool| {
                            let half = (tool["angle"].as_f64()? / 2.).to_radians().tan();
                            Some(op["top_z"].as_f64()? - (width + crate::resources::TIP_CLEARANCE) / half)
                        }))
                    }
                    _ => None,
                };
                if floor.is_none_or(|z| z >= cap) { return true; }
                let edge = if kind == "edge_chamfer" { true } else if kind == "notch" {
                    op["outer"].as_array().into_iter().flatten().filter_map(|p| p[axis].as_f64()).any(|v| v <= low || v >= high)
                } else {
                    let half = op["width"].as_f64().unwrap_or(0.) / 2.;
                    ["a", "b"].iter().filter_map(|k| op[k][axis].as_f64()).any(|v| v - half <= low || v + half >= high)
                };
                if !edge { return true; }
                let reason = format!("{kind} material below Z{} jaw clearance cap left for a later setup", show(cap));
                let d = trace.warn("machine.clamping", &setup_id, reason.clone(), json!({"features": op["features"], "jaw_cap_z": cap}));
                for feature in op["features"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    trace.skip("machine.clamping", feature, reason.clone());
                }
                if kind == "v_channel" || kind == "edge_chamfer" || op["top_z"].as_f64().is_some_and(|z| z <= cap) { return false; }
                op["floor_z"] = json!(cap);
                // It no longer breaks through: job generation must not extend below the cap.
                op["bottom"] = json!("blind");
                because(op, [d]);
                true
            });
        }
        let view = setup.clone();
        let ops = setup["operations"].as_array_mut().ok_or("setup has no operations")?;
        let mut refused = vec![];
        for op in ops.iter().filter(|o| o["kind"] == "wall_finish") {
            if let Some((rule, why)) = wall_refusal(&view, op, &library)? {
                let wall = format!("{setup_id}:{}", op["wall"].as_str().unwrap_or(""));
                trace.warn(rule, &wall, format!("setup {setup_id}: outside wall above Z{} left out: {why}", show(num(op, "floor_z")?)), json!({"features": op["features"]}));
                refused.push(op["wall"].clone());
            }
        }
        let mut no_channel = vec![];
        for (n, op) in ops.iter().enumerate().filter(|(_, o)| o["kind"] == "v_channel") {
            if let Some((rule, why)) = v_channel_refusal(&view, op, &library)? {
                trace.warn(rule, &format!("{setup_id}:v_channel"), format!("setup {setup_id}: V-channel left out: {why}"), json!({"features": op["features"]}));
                for id in op["features"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    trace.skip(rule, id, format!("V-channel left out: {why}"));
                }
                no_channel.push(n);
            }
        }
        for n in no_channel.into_iter().rev() {
            ops.remove(n);
        }
        let rough = ops.iter().any(|o| o["kind"] == "roughing");
        if !rough && ops.iter().any(|o| o["kind"] == "flats") {
            trace.decide("feasibility.flats.need_roughing", &format!("{setup_id}:flats"), "removed", "flats follow roughing, which is not cut in this setup", Value::Null);
        }
        ops.retain(|o| !(o["kind"] == "wall_finish" && refused.contains(&o["wall"])) && (rough || o["kind"] != "flats"));
        let walls: Vec<Value> = ops.iter().filter(|o| o["kind"] == "wall_finish").map(|o| o["wall"].clone()).collect();
        ops.retain_mut(|op| {
            if op["kind"] != "edge_chamfer" || op["needs_wall"].is_null() || walls.contains(&op["needs_wall"]) {
                return true;
            }
            let lost = op["wall_features"].as_array().cloned().unwrap_or_default();
            for id in lost.iter().filter_map(Value::as_str) {
                let edge = op["radius"].as_f64().map_or("edge chamfer".into(), |radius| format!("r{} edge round", show(radius)));
                let reason = if op["radius"].is_number() {
                    format!("{edge}: its wall keeps its roughing allowance, so it is not chamfered")
                } else {
                    format!("{edge}: its wall keeps its roughing allowance, so the edge stays sharp (chamfer it by hand); leg {} mm", show(op["width"].as_f64().unwrap_or(0.)))
                };
                trace.skip("feasibility.chamfer.needs_wall", id, reason);
            }
            if op["boundaries"] != "all" {
                return false;
            }
            op["boundaries"] = json!("holes");
            op["needs_wall"] = Value::Null;
            if let Some(features) = op["features"].as_array_mut() {
                features.retain(|f| !lost.contains(f));
            }
            true
        });
    }
    Ok(())
}

/// Names the tool that cuts each operation (`tool`), or says why none can (`tool_note`; null when
/// nothing needs saying). An operation kind this does not know is left without either.
pub fn resolve(plan: &mut Value, library_data: &Value) -> Result<(), String> {
    let mut trace = Trace::take(plan);
    let result = resolve_with(plan, library_data, &mut trace);
    trace.store(plan);
    result
}

fn resolve_with(plan: &mut Value, library_data: &Value, trace: &mut Trace) -> Result<(), String> {
    let library = Library::new(library_data)?;
    for (tool, why) in &library.too_big {
        trace.decide("machine.spindle.tool_size", id(tool), "removed", why, Value::Null);
    }
    let group = material(plan, library_data, trace);
    let plan_material = plan["material"]["used"].as_str().map(String::from);
    for setup in plan["setups"].as_array_mut().ok_or("plan has no setups")? {
        let view = setup.clone();
        let setup_id = view["id"].as_str().unwrap_or("").to_string();
        let top = view["stock"]["max"][2].as_f64().ok_or("bad stock")?;
        let has_rough = view["operations"].as_array().is_some_and(|ops| ops.iter().any(|o| o["kind"] == "roughing"));
        for (index, op) in setup["operations"].as_array_mut().ok_or("setup has no operations")?.iter_mut().enumerate() {
            let kind = op["kind"].as_str().unwrap_or("").to_string();
            let features = || op["features"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
            // The tool, or why there is none (None: nothing to say); the rule; the candidates
            // turned down, and why.
            let mut considered: Vec<Value> = vec![];
            let (mut chosen, rule): (Result<&Value, Option<String>>, &str) = match kind.as_str() {
                "facing" | "roughing" => {
                    if library.data.get(format!("{kind}_tool")).and_then(Value::as_str).is_none_or(str::is_empty) {
                        considered = library.considered("largest_end_mill", &[]);
                    }
                    (library.clearing(&kind)?.ok_or_else(|| Some(format!("no end mill to {} with", if kind == "facing" { "face" } else { "rough" }))), "tool.clearing")
                }
                "flats" => match (has_rough, library.clearing("roughing")?) {
                    (true, Some(s)) => (Ok(s), "tool.flats.follow_roughing"),
                    (true, None) => (Err(Some(format!("left out: it follows roughing, with the same end mill: needs {}", crate::needed::for_operation(op, top, &library, None)?.unwrap()))), "tool.flats.follow_roughing"),
                    _ => (Err(Some("left out: it follows roughing, with the same end mill".into())), "tool.flats.follow_roughing"),
                },
                "spot_drill" => {
                    let selected = op["spot_tool"].as_str().and_then(|name| library.data["tools"].as_array()?.iter().find(|t| id(t) == name));
                    let need = crate::needed::for_operation(op, top, &library, None)?.ok_or("no spot-drill query")?;
                    (selected.ok_or_else(|| Some(format!("no usable spot drill: needs {need}"))), "tool.spot_drill")
                }
                "holes" => {
                    let d = num(op, "diameter")?;
                    let depth = library.drill_depth(op, top)?;
                    let drilled = library.drill(d, depth);
                    considered = library.considered("drill", &[("diameter", d), ("depth", depth)]);
                    if drilled.is_none() {
                        // Bored: the end mills decided it, so they come first.
                        let drills = std::mem::take(&mut considered);
                        considered = library.considered("bore_mill", &[("diameter", d)]);
                        considered.extend(drills);
                    }
                    let tool = drilled.or_else(|| library.bore_mill(d)).or_else(|| library.deferred_drill(d, depth)).ok_or_else(|| Some(format!("no drill of ⌀{} and no end mill small enough to bore it ({})", show(d), features())));
                    (tool, "tool.hole.drill_or_bore")
                }
                "counterbore" => {
                    let d = num(op, "diameter")?;
                    considered = library.considered("bore_mill", &[("diameter", d)]);
                    (library.bore_mill(d).ok_or_else(|| Some(format!("no end mill small enough for the ⌀{} counterbore", show(d)))), "tool.counterbore.bore_mill")
                }
                "thread" => {
                    let (tap, taps) = library.tap(op, top, library.thread_open_z(op, &view, top)?, group.as_deref())?;
                    let (mill, why) = library.thread_mill(op, top)?;
                    considered = why.iter().map(|(candidate, text)| {
                        let rejected = text.strip_prefix(&format!("{candidate}: ")).unwrap_or(text);
                        json!({"candidate": candidate, "rejected": rejected})
                    }).collect();
                    considered.extend(taps);
                    if tap.is_some() && let Some(mill) = mill {
                        considered.push(json!({"candidate": id(mill), "rejected": "qualifies, but matching small-thread tap in ISO N material is preferred"}));
                    }
                    let tool = tap.or(mill).ok_or_else(|| {
                        let texts: Vec<&str> = why.iter().map(|(_, t)| t.as_str()).collect();
                        let reasons = if texts.is_empty() { String::new() } else { format!(" ({})", texts.join("; ")) };
                        Some(format!("{} ({}): no thread mill can cut it{reasons}", op["spec"].as_str().unwrap_or(""), features()))
                    });
                    (tool, if library.has_taps() { "tool.thread.tap_or_mill" } else { "tool.thread_mill" })
                }
                "slot" => {
                    let width = num(op, "width")?;
                    considered = library.considered("bore_mill", &[("diameter", width)]);
                    (library.bore_mill(width).ok_or_else(|| Some(format!("no end mill small enough for the {} mm slot ({})", show(width), features()))), "tool.slot.bore_mill")
                }
                "notch" => {
                    let width = num(op, "width")?;
                    let depth = num(op, "top_z")? - num(op, "floor_z")?;
                    considered = library.considered("pocket_mill", &[("width", width), ("depth", depth)]);
                    let tool = library.pocket_mill(width, depth).or_else(|| library.deferred_pocket_mill(width, depth))
                        .ok_or_else(|| Some(format!("no end mill both narrower than the {} mm width and long enough ({})", show(width), features())));
                    (tool, "tool.notch.pocket_mill")
                }
                "v_channel" => match v_channel_refusal(&view, op, &library)? {
                    Some((rule, why)) => (Err(Some(why)), rule),
                    None => {
                        considered = library.considered("v_bit", &[("angle", num(op, "angle")?)]);
                        (library.v_bit(num(op, "angle")?).ok_or(None), "tool.v_channel.v_bit")
                    }
                },
                "wall_finish" => match wall_refusal(&view, op, &library)? {
                    Some((rule, why)) => (Err(Some(why)), rule),
                    None => (library.clearing("roughing")?.ok_or(None), "tool.wall.roughing_mill"),
                },
                "edge_chamfer" => {
                    let width = num(op, "width")?;
                    let maximum = op["maximum_diameter"].as_f64().unwrap_or(f64::INFINITY);
                    let room = op["room"].as_f64().unwrap_or(f64::INFINITY);
                    let depth = top - num(op, "top_z")? + width + crate::resources::TIP_CLEARANCE;
                    considered = library.considered("chamfer_mill", &[("width", width), ("maximum_diameter", maximum), ("depth", depth), ("room", room)]);
                    (library.chamfer_mill_in(width, maximum, depth, room).ok_or_else(|| Some(format!("no 90° chamfer mill covers the whole {} mm chamfer with tip clearance, loop clearance and cutting-length reach; width passes cannot extend its cone", show(width)))), "tool.edge_chamfer.chamfer_mill")
                }
                _ => continue,
            };
            let subject = format!("{setup_id}:{kind}");
            let operation = format!("{setup_id}-{}-{kind}", index + 1);
            let mut supplied = None;
            let supplied_rule = if rule == "tool.thread.tap_or_mill" && trace.supplied_for(rule, &subject, Some(&operation)).is_none() && trace.supplied_for("tool.thread_mill", &subject, Some(&operation)).is_some() { "tool.thread_mill" } else { rule };
            if view["operations"].as_array().unwrap().iter().filter(|o| o["kind"] == kind).count() > 1
                && let Some(n) = trace.supplied_for(supplied_rule, &subject, None) {
                trace.refuse(n, "subject matches more than one operation; name operation explicitly");
            }
            if let Some(n) = trace.supplied_for(supplied_rule, &subject, Some(&operation))
                && crate::supplied::supported(rule) {
                    match library.supplied_thread_tool(op, top, &trace.supplied[n]["choice"], &view, group.as_deref()) {
                        Ok((tool, candidates)) => {
                            considered = candidates;
                            chosen = Ok(tool);
                            supplied = Some(n);
                        }
                        Err(reason) => trace.refuse(n, &reason),
                    }
            }
            let (tool, note, decided) = match chosen {
                Ok(tool) => {
                    let deferred = considered.iter().any(|c| c["candidate"] == id(tool) && c["rejected"].as_str().is_some_and(|r| r.contains("flutes")));
                    let says = if deferred { format!("{} selected; reach is left to camlib's check", id(tool)) } else if tool["type"] == "tap" { format!("{} taps the small metric thread in ISO N material: matching pitch, hand and pre-drill; chamfer, one-pitch bottom clearance and reach fit", id(tool)) } else { format!("{} cuts it", id(tool)) };
                    let decided = trace.decide(rule, &subject, "chosen", &says, json!({"chosen": id(tool), "considered": considered}));
                    (json!(id(tool)), Value::Null, decided)
                }
                Err(note) => {
                    let note = match note {
                        Some(text) if !text.contains(": needs ") && matches!(rule, "tool.clearing" | "tool.hole.drill_or_bore" | "tool.counterbore.bore_mill" | "tool.thread_mill" | "tool.thread.tap_or_mill" | "tool.slot.bore_mill" | "tool.notch.pocket_mill" | "tool.edge_chamfer.chamfer_mill" | "feasibility.v_channel.needs_bit" | "feasibility.wall.needs_mill") => {
                            Some(match crate::needed::for_operation_with_context(op, top, &library, None, &view, group.as_deref())? {
                                Some(need) => format!("{text}: needs {need}"),
                                None => text,
                            })
                        }
                        other => other,
                    };
                    let says = note.clone().unwrap_or_else(|| "no tool".into());
                    let decided = trace.decide(rule, &subject, "none", &says, json!({"chosen": null, "considered": considered}));
                    (Value::Null, json!(note), decided)
                }
            };
            if let Some(n) = supplied { trace.accept(n, &decided); }
            let tool_type = tool.as_str().and_then(|t| library.data["tools"].as_array()?.iter().find(|x| x["id"] == t)?["type"].as_str());
            if let Some(names) = crate::reference::iso14649(op, tool_type) {
                op["iso14649"] = names;
            }
            let mut why = vec![decided];
            // Feature warnings made before grouping belong to each operation cutting that feature.
            why.extend(trace.decisions.iter().filter(|d| d["outcome"] == "warning" && d.get("superseded_by").is_none()
                && !op["because"].as_array().is_some_and(|ids| ids.contains(&d["id"]))
                && d["features"].as_array().is_some_and(|features| features.contains(&d["subject"]) && features.iter().any(|f| op["features"].as_array().is_some_and(|own| own.contains(f)))))
                .filter_map(|d| d["id"].as_str().map(String::from)));
            if let Some(name) = tool.as_str() {
                let spec = library.data["tools"].as_array().and_then(|t| t.iter().find(|x| x["id"] == name)).ok_or("a chosen tool is not in the library")?;
                // No dog-bone relief: neither analysis nor answers can request it today.
                if kind == "notch" {
                    let model = op["corner_radius"].as_f64().unwrap_or(0.);
                    let radius = num(spec, "diameter")? / 2.;
                    if radius > model + 1e-6 {
                        let corners = op["inside_corners"].as_u64().unwrap_or(4);
                        let shape = if model == 0. { "square inside corners" } else { "inside corners" };
                        let model_note = if model == 0. { "the model has them sharp".to_string() } else { format!("the model has r{}", warning_number(model)) };
                        let feature_names = format!("setup {setup_id}: {} mm notch", warning_number(num(op, "width")?));
                        why.push(trace.warn("process.notch.corner_radius", &subject, format!("{}: its {corners} {shape} are left r{} by {name} ({model_note})", feature_names, warning_number(radius)), json!({"features": op["features"], "radius": radius, "model_radius": model, "corners": corners})));
                    }
                }
                let (mut technology, source) = technology(spec, group.as_deref());
                if spec["type"] == "tap" { technology.as_object_mut().unwrap().remove("feed_mm_min"); }
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
                if kind == "holes" && spec["type"] == "drill" {
                    let depth = library.drill_depth(op, top)?;
                    let diameter = num(spec, "diameter")?;
                    let cycle = crate::reference::drilling(depth, diameter, spec);
                    if let Some(ratio) = crate::reference::deep_drilling_ratio(depth, diameter) {
                        why.push(trace.warn("process.hole.peck_by_depth", &subject, format!(
                            "setup {setup_id}: {} mm holes, depth/D {ratio:.1}: deep-hole drilling (pilot plus deep-hole drill) is not planned, so it is drilled with full-retract pecks",
                            warning_number(diameter)), json!({"features": op["features"], "depth_diameter_ratio": ratio})));
                    }
                    why.push(trace.decide("process.hole.peck_by_depth", &subject, "chosen",
                        "Drilling cycle from stock-top depth / drill diameter; positive tool peck_mm caps the increment",
                        json!({"chosen": cycle})));
                    technology.as_object_mut().unwrap().extend(cycle.as_object().unwrap().clone());
                }
                if kind == "wall_finish" {
                    let depth = (top - num(op, "floor_z")?).max(0.);
                    let flute = num(spec, "flute_length")?;
                    if depth > 0. {
                        let passes = (depth / flute).ceil().max(1.);
                        let stepdown = depth / passes;
                        technology["stepdown_mm"] = json!(stepdown);
                        why.push(trace.decide("process.wall_finish.axial_passes", &subject, "chosen",
                            "Finish the wall in the fewest equal axial passes its flute length allows; roughing stepdown does not limit the finishing allowance cut. Camlib still checks reach and stock contact.",
                            json!({"chosen": {"passes": passes as u64, "stepdown_mm": stepdown, "depth_mm": depth, "flute_length_mm": flute}})));
                    }
                }
                if kind == "roughing" {
                    why.push(crate::roughing::choose(op, &mut technology, spec, group.as_deref(), library_data, &subject, trace)?);
                }
                op["technology"] = technology;
            }
            op["tool"] = tool;
            op["tool_note"] = note;
            because(op, why);
        }
        block_chamfers_without_openings(setup, trace);
    }
    Ok(())
}

/// Resolve every opening first, then gate its mouth chamfer on the actual result.
/// Feature ids survive operation grouping, renumbering and execution exclusions.
/// A combined round-as-chamfer operation is conservatively blocked as a whole.
fn block_chamfers_without_openings(setup: &mut Value, trace: &mut Trace) {
    let view = setup["operations"].as_array().cloned().unwrap_or_default();
    let subject = format!("{}:edge_chamfer", setup["id"].as_str().unwrap_or(""));
    for op in setup["operations"].as_array_mut().into_iter().flatten().filter(|o| o["kind"] == "edge_chamfer") {
        let blocked = op["needs_openings"].as_array().into_iter().flatten().find_map(|feature| {
            let opening = view.iter().find(|o| matches!(o["kind"].as_str(), Some("holes" | "slot"))
                && o["features"].as_array().is_some_and(|f| f.contains(feature)));
            if opening.is_some_and(|o| o["tool"].is_string()) { return None; }
            let why = opening.and_then(|o| o["tool_note"].as_str())
                .or_else(|| trace.skipped.iter().find(|s| s["feature"] == *feature).and_then(|s| s["reason"].as_str()))
                .unwrap_or("its hole or slot operation was left out");
            Some((feature.clone(), why.to_owned()))
        });
        let Some((feature, why)) = blocked else { continue; };
        let reason = format!("mouth chamfer: required opening {} is not cut, so the edge stays sharp (chamfer it by hand)", feature.as_str().unwrap_or(""));
        let note = match op["tool_note"].as_str().filter(|_| op["tool"].is_null()) {
            Some(own) => format!("{reason}: {}; {own}", why.split(": needs ").next().unwrap_or(&why)),
            None => format!("{reason}: {why}"),
        };
        let decided = trace.decide("feasibility.chamfer.needs_opening", &subject, "none", &note, json!({"chosen": null, "features": [feature]}));
        for decision in &mut trace.decisions {
            if decision["rule"].as_str().is_some_and(|r| r.starts_with("tool.edge_chamfer.chamfer_mill@"))
                && op["because"].as_array().is_some_and(|ids| ids.contains(&decision["id"])) {
                decision["superseded_by"] = json!(decided);
            }
        }
        op["tool"] = Value::Null;
        op["tool_note"] = json!(note);
        if let Some(object) = op.as_object_mut() { object.remove("technology"); }
        because(op, [decided]);
    }
}

/// The cutting data a tool's operations use: its own, with what it states for the material's ISO
/// 513 group (`cutting_data`) in their place. And where they came from: `tool`, or
/// `cutting_data.<group>`.
fn technology(tool: &Value, group: Option<&str>) -> (Value, String) {
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
    (Value::Object(out), source)
}

/// The material the part is cut from: Specify's answer, unless the resources override it, and its
/// ISO 513 group. Writes `material` into the plan and returns the group.
fn material(plan: &mut Value, library: &Value, trace: &mut Trace) -> Option<String> {
    let specified = plan["material"]["specified"].as_str().map(String::from);
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
    plan["material"] = json!({"specified": specified, "override": over, "used": used, "iso513": group});
    group
}

/// Millimetres in warning prose, rounded without trailing zeroes.
fn warning_number(value: f64) -> String {
    crate::plan::r(value).to_string()
}
