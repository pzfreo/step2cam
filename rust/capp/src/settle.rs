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
    settle(plan, library)?;
    for exclusion in exclusions {
        let operation = exclusion["operation"].as_str().ok_or("an exclusion names no operation")?;
        let message = crate::needed::refusal(plan, operation, exclusion["message"].as_str().unwrap_or(""), &Library::new(library)?)?;
        exclude(plan, operation, &message)
            .ok_or_else(|| format!("excluded operation {operation} is not in the plan"))?;
        settle(plan, library)?;
    }
    let mut trace = Trace::take(plan);
    crate::machines::place(plan, library, &mut trace);
    trace.store(plan);
    resolve(plan, library)?;
    let mut trace = Trace::take(plan);
    crate::machines::check_changer(plan, library, &mut trace);
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
    trace.warn("execution.refused", operation, format!("{operation} left out: {brief}"));
    // Its own decisions (what made it, its tool and cutting data) no longer stand; those it shares
    // with the rest of the setup (the setup, the order, where its holes are cut) do.
    let own: Vec<&str> = removed["because"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    for d in trace.decisions.iter_mut().filter(|d| d["id"].as_str().is_some_and(|id| own.contains(&id)) && (d["stage"] == "process" || d["stage"] == "tools")) {
        d["superseded_by"] = json!(refused);
    }
    trace.store(plan);
    Some(removed)
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
    for setup in plan["setups"].as_array_mut().ok_or("plan has no setups")? {
        let setup_id = setup["id"].as_str().unwrap_or("").to_string();
        let view = setup.clone();
        let ops = setup["operations"].as_array_mut().ok_or("setup has no operations")?;
        let mut refused = vec![];
        for op in ops.iter().filter(|o| o["kind"] == "wall_finish") {
            if let Some((rule, why)) = wall_refusal(&view, op, &library)? {
                let wall = format!("{setup_id}:{}", op["wall"].as_str().unwrap_or(""));
                trace.warn(rule, &wall, format!("setup {setup_id}: outside wall above Z{} left out: {why}", show(num(op, "floor_z")?)));
                refused.push(op["wall"].clone());
            }
        }
        let mut no_channel = vec![];
        for (n, op) in ops.iter().enumerate().filter(|(_, o)| o["kind"] == "v_channel") {
            if let Some((rule, why)) = v_channel_refusal(&view, op, &library)? {
                trace.warn(rule, &format!("{setup_id}:v_channel"), format!("setup {setup_id}: V-channel left out: {why}"));
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
        for op in setup["operations"].as_array_mut().ok_or("setup has no operations")? {
            let kind = op["kind"].as_str().unwrap_or("").to_string();
            let features = || op["features"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
            // The tool, or why there is none (None: nothing to say); the rule; the candidates
            // turned down, and why.
            let mut considered: Vec<Value> = vec![];
            let (chosen, rule): (Result<&Value, Option<String>>, &str) = match kind.as_str() {
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
                "holes" => {
                    let d = num(op, "diameter")?;
                    let drilled = library.drill(d);
                    considered = library.considered("drill", &[("diameter", d)]);
                    if drilled.is_none() {
                        // Bored: the end mills decided it, so they come first.
                        let drills = std::mem::take(&mut considered);
                        considered = library.considered("bore_mill", &[("diameter", d)]);
                        considered.extend(drills);
                    }
                    let tool = drilled.or_else(|| library.bore_mill(d)).ok_or_else(|| Some(format!("no drill of ⌀{} and no end mill small enough to bore it ({})", show(d), features())));
                    (tool, "tool.hole.drill_or_bore")
                }
                "counterbore" => {
                    let d = num(op, "diameter")?;
                    considered = library.considered("bore_mill", &[("diameter", d)]);
                    (library.bore_mill(d).ok_or_else(|| Some(format!("no end mill small enough for the ⌀{} counterbore", show(d)))), "tool.counterbore.bore_mill")
                }
                "thread" => {
                    let (mill, why) = library.thread_mill(op, top)?;
                    considered = why.iter().map(|(candidate, text)| {
                        let rejected = text.strip_prefix(&format!("{candidate}: ")).unwrap_or(text);
                        json!({"candidate": candidate, "rejected": rejected})
                    }).collect();
                    let tool = mill.ok_or_else(|| {
                        let texts: Vec<&str> = why.iter().map(|(_, t)| t.as_str()).collect();
                        let reasons = if texts.is_empty() { String::new() } else { format!(" ({})", texts.join("; ")) };
                        Some(format!("{} ({}): no thread mill can cut it{reasons}", op["spec"].as_str().unwrap_or(""), features()))
                    });
                    (tool, "tool.thread_mill")
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
                    let tool = library.pocket_mill(width, depth)
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
            let (tool, note, decided) = match chosen {
                Ok(tool) => {
                    let says = format!("{} cuts it", id(tool));
                    let decided = trace.decide(rule, &subject, "chosen", &says, json!({"chosen": id(tool), "considered": considered}));
                    (json!(id(tool)), Value::Null, decided)
                }
                Err(note) => {
                    let note = match note {
                        Some(text) if !text.contains(": needs ") && matches!(rule, "tool.clearing" | "tool.hole.drill_or_bore" | "tool.counterbore.bore_mill" | "tool.thread_mill" | "tool.slot.bore_mill" | "tool.notch.pocket_mill" | "tool.edge_chamfer.chamfer_mill" | "feasibility.v_channel.needs_bit" | "feasibility.wall.needs_mill") => {
                            Some(match crate::needed::for_operation(op, top, &library, None)? {
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
            let tool_type = tool.as_str().and_then(|t| library.data["tools"].as_array()?.iter().find(|x| x["id"] == t)?["type"].as_str());
            if let Some(names) = crate::reference::iso14649(op, tool_type) {
                op["iso14649"] = names;
            }
            let mut why = vec![decided];
            if let Some(name) = tool.as_str() {
                let spec = library.data["tools"].as_array().and_then(|t| t.iter().find(|x| x["id"] == name)).ok_or("a chosen tool is not in the library")?;
                let (mut technology, source) = technology(spec, group.as_deref());
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
                op["technology"] = technology;
            }
            op["tool"] = tool;
            op["tool_note"] = note;
            because(op, why);
        }
    }
    Ok(())
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
