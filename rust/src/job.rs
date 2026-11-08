//! The camlib adapter: a plan whose operations name their tools (`capp::finish`) + the tool
//! library -> camlib jobs, how each tool cuts and how deep. A job is camlib's manifest, plain
//! JSON, one per setup.
use capp::plan::{PLAN_FORMAT, r, show};
use capp::resources::{TIP_CLEARANCE, DRILLING_TOLERANCE, drill_tip_length, id, num};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Kept between a drill tip and material found beyond a through hole's exit (mm).
const EXIT_MARGIN: f64 = 0.2;
/// Twice a triangle's area (mm²) at or below which camlib's surface mesh calls it degenerate, at
/// the tolerance of 1e-5 mm that the job gives it.
const NO_AREA: f64 = 1e-10;
/// Thread milling passes, unless the tool says otherwise: the thread depth in two radial steps,
/// then one spring pass at full depth.
const RADIAL_PASSES: u64 = 2;
const SPRING_PASSES: u64 = 1;
/// How much work camlib may spend comparing the stock with the part, its most. A check that runs out refuses
/// the program. camlib's default runs out on the fine grid a thread mill needs.
const COMPARISON_BUDGET: u64 = 1_000_000_000;
const COMPARISON_NODES: u64 = 10_000_000;
/// camlib's most: a gouge should get a location even among the cells of material left.
const COMPARISON_FINDINGS: usize = 4096;

/// camlib proves a thread mill's neck clear of the hole wall beside the groove from its stock
/// grid: a cell up to its diagonal away can still count as wall. The grid is made fine enough for
/// the neck's clearance, with this much in hand...
const NECK_CELL_SAFETY: f64 = 1.25;
/// ...but no finer than a stock grid of this many cells.
const MAX_STOCK_CELLS: f64 = 16_000_000.;
fn assembly(spec: &Value) -> Result<Value, String> {
    let name = id(spec);
    let (diameter, stickout) = (num(spec, "diameter")?, num(spec, "stickout")?);
    let gauge = stickout + num(spec, "holder_length")?;
    let radius = num(spec, "holder_diameter")? / 2.;
    let drill = spec["type"] == "drill";
    if spec["type"] == "thread_mill" {
        return Ok(json!({
            "id": name,
            "cutter": {
                "id": name, "name": name, "shape": {"kind": "flat"}, "diameter_mm": diameter,
                "cutting_length_mm": num(spec, "tooth_height")?, "overall_length_mm": num(spec, "overall_length")?,
                "shank_diameter_mm": spec.get("shank_diameter").and_then(Value::as_f64).unwrap_or(diameter),
                "flutes": 3, "center_cutting": false, "material": "carbide",
                "neck": {"diameter_mm": num(spec, "neck_diameter")?, "length_mm": num(spec, "neck_length")?},
            },
            "stickout_mm": stickout,
            "gauge_length_mm": gauge,
            "holders": [{"z_min_mm": stickout, "z_max_mm": gauge, "radius_min_mm": radius, "radius_max_mm": radius}],
        }));
    }
    if spec["type"] == "chamfer_mill" {
        return Ok(json!({
            "id": name,
            "cutter": {
                "id": name, "name": name,
                "shape": {"kind": "chamfer", "included_angle_rad": num(spec, "angle")?.to_radians(), "tip_diameter_mm": num(spec, "tip_diameter")?},
                "diameter_mm": diameter, "cutting_length_mm": num(spec, "flute_length")?, "overall_length_mm": num(spec, "overall_length")?,
                "shank_diameter_mm": spec.get("shank_diameter").and_then(Value::as_f64).unwrap_or(diameter),
                "flutes": 3, "center_cutting": true, "material": "carbide", "neck": null,
            },
            "stickout_mm": stickout,
            "gauge_length_mm": gauge,
            "holders": [{"z_min_mm": stickout, "z_max_mm": gauge, "radius_min_mm": radius, "radius_max_mm": radius}],
        }));
    }
    let shape = if drill {
        json!({"kind": "drill", "point_angle_rad": num(spec, "point_angle")?.to_radians()})
    } else {
        json!({"kind": "flat"})
    };
    Ok(json!({
        "id": name,
        "cutter": {
            "id": name, "name": name, "shape": shape, "diameter_mm": diameter,
            "cutting_length_mm": num(spec, "flute_length")?, "overall_length_mm": num(spec, "overall_length")?,
            "shank_diameter_mm": spec.get("shank_diameter").and_then(Value::as_f64).unwrap_or(diameter),
            "flutes": if drill { 2 } else { 3 }, "center_cutting": true, "material": "carbide", "neck": null,
        },
        "stickout_mm": stickout,
        "gauge_length_mm": gauge,
        "holders": [{"z_min_mm": stickout, "z_max_mm": gauge, "radius_min_mm": radius, "radius_max_mm": radius}],
    }))
}

/// The tool as this operation cuts with it: its own fields, with the cutting data capp chose for
/// the operation (`technology`: feed, speed, plunge, stepdown, stepover) in their place.
fn cutting_data(tool: &Value, op: &Value) -> Value {
    let mut out = tool.clone();
    for (key, value) in op["technology"].as_object().into_iter().flatten() {
        if key != "source" {
            out[key.as_str()] = value.clone();
        }
    }
    out
}

/// The cutting data every operation carries.
fn cutting(op: &mut Value, spec: &Value, plunge: bool) -> Result<(), String> {
    let number = spec["number"].clone();
    op["feed_mm_min"] = json!(num(spec, "feed_mm_min")?);
    op["tolerance_mm"] = json!(DRILLING_TOLERANCE);
    op["spindle_rpm"] = json!(num(spec, "spindle_rpm")?);
    op["work_offset"] = json!(1);
    op["tool_number"] = number.clone();
    op["length_register"] = spec.get("length_register").cloned().unwrap_or(number);
    op["coolant"] = spec["coolant"].clone();
    if plunge {
        op["plunge_mm_min"] = json!(num(spec, "plunge_mm_min")?);
    }
    Ok(())
}

/// (depth_z, depth_reference, allow_breakthrough) for drilling one hole group.
fn drill_depth(op: &Value, drill: &Value, breakthrough: f64, stock_bottom: f64, op_id: &str, notes: &mut Vec<String>) -> Result<(f64, &'static str, f64), String> {
    let (d, bottom_z) = (num(op, "diameter")?, num(op, "bottom_z")?);
    let tip = drill_tip_length(drill)?;
    let kind = op["bottom"].as_str().unwrap_or("unknown");
    if kind == "through" {
        let want = breakthrough;
        let (depth, reference, tip_z);
        match op.get("exit_clear_mm").and_then(Value::as_f64) {
            None => (depth, reference, tip_z) = (bottom_z - want, "full_diameter", bottom_z - want - tip),
            Some(clear) => {
                let room = clear - EXIT_MARGIN; // how far the tip may go past the exit
                if room >= tip {
                    let through = want.min(room - tip);
                    (depth, reference, tip_z) = (bottom_z - through, "full_diameter", bottom_z - through - tip);
                    if through < want {
                        notes.push(format!("{op_id}: part material {} mm beyond the exit: drilled only {} mm past it at full diameter", show(clear), show(r(through))));
                    }
                } else {
                    let z = bottom_z - room.max(0.);
                    (depth, reference, tip_z) = (z, "tip", z);
                    notes.push(format!("{op_id}: part material {} mm beyond the exit: the ⌀{} drill cannot reach full diameter at the exit without cutting it; finish it another way", show(clear), show(d)));
                }
            }
        }
        let mut allow = 0.;
        if tip_z < stock_bottom - 1e-9 {
            notes.push(format!("{op_id}: the drill tip goes {} mm below the stock bottom: support the part clear of the holes (spoilboard, parallels)", show(r(stock_bottom - tip_z))));
            allow = stock_bottom - tip_z + 0.01;
        }
        return Ok((depth, reference, allow));
    }
    if kind == "drill_point" {
        // quiddity's depth ends where the cone starts: full diameter there.
        notes.push(format!("{op_id}: ⌀{} drill-point holes: full diameter at the model's depth; the point ends {} mm deeper, at the {}° drill's angle (the model's cone angle is not read)", show(d), show(r(tip)), drill["point_angle"]));
        return Ok((bottom_z, "full_diameter", 0.));
    }
    if kind == "flat" {
        notes.push(format!("{op_id}: ⌀{} flat-bottomed holes: the drill's tip stops at the bottom, so full diameter ends {} mm short of it and a point is left", show(d), show(r(tip))));
    } else {
        notes.push(format!("{op_id}: ⌀{} holes with an unknown bottom: the drill's tip stops at the model's depth", show(d)));
    }
    Ok((bottom_z, "tip", 0.))
}

/// The Bore floor for a hole group: through holes go past the exit as far as the stock (camlib
/// bores no lower) and any part material beyond allow.
fn bore_floor(op: &Value, breakthrough: f64, stock_bottom: f64, op_id: &str, notes: &mut Vec<String>) -> Result<f64, String> {
    let bottom_z = num(op, "bottom_z")?;
    if op["bottom"] != "through" {
        return Ok(bottom_z);
    }
    let mut past = breakthrough;
    if let Some(clear) = op.get("exit_clear_mm").and_then(Value::as_f64) {
        past = past.min((clear - EXIT_MARGIN).max(0.));
    }
    let floor = stock_bottom.max(bottom_z - past);
    if floor > bottom_z - 0.05 {
        notes.push(format!("{op_id}: ⌀{} through holes are bored to the exit and no further (the stock or the part ends there): expect a skin or burr to clean up", show(num(op, "diameter")?)));
    }
    Ok(floor)
}

fn bore(op_id: &str, spec: &Value, centers: &Value, diameter: f64, floor: f64, clearance: f64, retract: f64) -> Result<Value, String> {
    let mut out = json!({
        "kind": "bore", "id": op_id, "tool": id(spec), "centers_mm": centers, "diameter_mm": diameter,
        "floor_z_mm": floor, "stepdown_mm": num(spec, "stepdown_mm")?,
        "radial_stepover_mm": num(spec, "stepover_mm")?.min(num(spec, "diameter")? / 2.),
        "radial_allowance_mm": 0.0, "bottom_allowance_mm": 0.0, "boundary_tolerance_mm": 0.01, "clockwise": true,
        "entry_feed_mm_min": num(spec, "plunge_mm_min")?, "clearance_z_mm": clearance,
        "retract_feed_mm_min": spec.get("retract_feed_mm_min").and_then(Value::as_f64).unwrap_or(num(spec, "feed_mm_min")?),
        "retract_z_mm": retract, "feed_z_mm": retract,
    });
    cutting(&mut out, spec, false)?;
    Ok(out)
}

/// A slot's outline, counter-clockwise: its two straight sides and its round ends.
fn obround(a: [f64; 2], b: [f64; 2], radius: f64) -> Value {
    let length = (b[0] - a[0]).hypot(b[1] - a[1]);
    let (ux, uy) = ((b[0] - a[0]) / length, (b[1] - a[1]) / length);
    let (nx, ny) = (-uy * radius, ux * radius);
    json!([
        [a[0] - nx, a[1] - ny, 0.0],
        [b[0] - nx, b[1] - ny, 1.0],
        [b[0] + nx, b[1] + ny, 0.0],
        [a[0] + nx, a[1] + ny, 1.0],
    ])
}

fn region(name: &str, outer: &Value, holes: &Value) -> Value {
    let holes: Vec<Value> = holes.as_array().into_iter().flatten().map(|h| json!({"vertices": h})).collect();
    json!({
        "version": 1, "units": "mm", "name": name, "boundary_error_mm": 0.0,
        "components": [{"outer": {"vertices": outer}, "holes": holes}],
        "source": {"plan": PLAN_FORMAT},
    })
}

fn point2(v: &Value) -> Result<[f64; 2], String> {
    Ok([v[0].as_f64().ok_or("bad point")?, v[1].as_f64().ok_or("bad point")?])
}

/// camlib's surface mesh: triangles as they are up to 1024, then points shared by exact value.
fn surface(op: &Value) -> Result<Value, String> {
    let p: Vec<f64> = op["positions"].as_array().ok_or("roughing has no positions")?.iter().filter_map(Value::as_f64).collect();
    let idx: Vec<usize> = op["indices"].as_array().ok_or("roughing has no indices")?.iter().filter_map(|v| v.as_u64().map(|x| x as usize)).collect();
    let mut out = json!({
        "version": 1, "units": "mm", "surface_error_mm": num(op, "surface_error_mm")?,
        "source": {"plan": PLAN_FORMAT},
    });
    let point = |i: usize| [p[3 * i], p[3 * i + 1], p[3 * i + 2]];
    // A triangle with no area has no surface, and camlib refuses a mesh that has one.
    let has_area = |t: &&[usize]| {
        let (a, b, c) = (point(t[0]), point(t[1]), point(t[2]));
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() > NO_AREA
    };
    let triangles: Vec<&[usize]> = idx.chunks_exact(3).filter(|t| has_area(t)).collect();
    if triangles.len() < 1024 {
        out["triangles"] = json!(triangles.iter().map(|t| [point(t[0]), point(t[1]), point(t[2])]).collect::<Vec<_>>());
    } else {
        let mut lookup = std::collections::HashMap::new();
        let (mut vertices, mut indices) = (vec![], vec![]);
        for t in triangles {
            let mut row = [0usize; 3];
            for (k, i) in t.iter().enumerate() {
                let q = point(*i);
                row[k] = *lookup.entry(q.map(f64::to_bits)).or_insert_with(|| {
                    vertices.push(q);
                    vertices.len() - 1
                });
            }
            indices.push(row);
        }
        out["vertices"] = json!(vertices);
        out["indices"] = json!(indices);
    }
    Ok(out)
}

pub struct Job {
    pub setup: String,
    /// camlib's manifest.
    pub manifest: Value,
    pub notes: Vec<String>,
}

/// One camlib job for each setup of a plan whose operations name their tools (`capp::resolve`).
/// The operation kinds a plan holds, each of which `capp::resolve` gives a tool or a note.
const KNOWN: [&str; 11] = ["facing", "roughing", "flats", "holes", "counterbore", "thread", "slot", "notch", "v_channel", "wall_finish", "edge_chamfer"];

/// The operation's tool, or else its note in the log and on to the next operation.
macro_rules! tool_or_skip {
    ($tool:expr, $notes:expr) => {
        match $tool {
            Ok(spec) => spec,
            Err(note) => {
                if let Some(note) = note {
                    $notes.push(note);
                }
                continue;
            }
        }
    };
}

pub fn jobs(plan: &Value, library_data: &Value) -> Result<(Vec<Job>, Vec<String>), String> {
    if plan["format"] != PLAN_FORMAT {
        return Err("not a step2cam plan".into());
    }
    let tools: Vec<&Value> = library_data.get("tools").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let breakthrough = library_data.get("breakthrough_mm").and_then(Value::as_f64).unwrap_or(1.);
    let clearance_above = library_data.get("clearance_mm").and_then(Value::as_f64).unwrap_or(10.);
    let retract_above = library_data.get("retract_mm").and_then(Value::as_f64).unwrap_or(2.);
    let (mut jobs, mut log) = (vec![], vec![]);
    for setup in plan["setups"].as_array().ok_or("plan has no setups")? {
        let setup_id = setup["id"].as_str().unwrap_or("");
        let (lo, hi) = (&setup["stock"]["min"], &setup["stock"]["max"]);
        let bound = |v: &Value, i: usize| v[i].as_f64().ok_or("bad stock");
        let top = bound(hi, 2)?;
        let stock_bottom = bound(lo, 2)?;
        let clearance = top + clearance_above;
        let retract = (top + retract_above).min(clearance);

        let (mut operations, mut used, mut notes): (Vec<Value>, Vec<&Value>, Vec<String>) = (vec![], vec![], vec![]);
        // How deep each hole is open at its full diameter, once cut: a thread must not go lower.
        let mut full_diameter_z: HashMap<String, f64> = HashMap::new();
        // The coarsest stock grid the operations can be checked on, if one needs a finer one.
        let mut needs_cell: Option<f64> = None;
        let features_of = |op: &Value| -> Vec<String> { op["features"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect() };
        for (n, op) in setup["operations"].as_array().ok_or("setup has no operations")?.iter().enumerate() {
            let kind = op["kind"].as_str().unwrap_or("");
            let op_id = format!("{setup_id}-{}-{kind}", n + 1);
            if KNOWN.contains(&kind) && op.get("tool").is_none() {
                return Err(format!("{op_id} names no tool: resolve the plan's tools with capp::finish first"));
            }
            // The tool capp chose, or why there is none (said at the point the operation is cut).
            let tool = match op["tool"].as_str() {
                Some(name) => Ok(*tools.iter().find(|t| id(t) == name).ok_or_else(|| format!("{op_id}: no tool {name:?} in the library"))?),
                None => Err(op["tool_note"].as_str().map(|n| format!("{op_id}: {n}"))),
            };
            let spec: &Value;
            let built = match kind {
                "facing" | "roughing" => {
                    let s = tool_or_skip!(tool, notes);
                    spec = s;
                    let s_cut = cutting_data(s, op);
                    let s = &s_cut;
                    let stepover = num(s, "stepover_mm")?.min(num(s, "diameter")? / 2.);
                    if kind == "facing" {
                        let mut out = json!({
                            "kind": "facing", "id": op_id, "tool": id(s), "floor_z_mm": num(op, "floor_z")?,
                            "stepdown_mm": num(s, "stepdown_mm")?, "stepover_mm": stepover, "clearance_z_mm": clearance,
                            "axis": "x", "mode": "zigzag", "reverse": false, "entry_air_gap_mm": 1.0, "edge_overlap_mm": null,
                        });
                        cutting(&mut out, s, true)?;
                        out
                    } else {
                        let error = num(op, "surface_error_mm")?;
                        let radial = library_data.get("radial_allowance_mm").and_then(Value::as_f64).unwrap_or(0.3).max(2. * error);
                        let bottom = library_data.get("bottom_allowance_mm").and_then(Value::as_f64).unwrap_or(0.2).max(2. * error);
                        let mut out = json!({
                            "kind": "roughing", "omit_empty_cuts": true, "id": op_id, "tool": id(s), "target": surface(op)?,
                            "clearing": {"kind": "raster", "stepover_mm": stepover, "routing": "economical"},
                            "top_z_mm": num(op, "top_z")?, "floor_z_mm": num(op, "floor_z")?,
                            "stepdown_mm": num(s, "stepdown_mm")?, "clearance_z_mm": clearance,
                            "radial_allowance_mm": radial, "bottom_allowance_mm": bottom, "clockwise": true,
                            "entry": {"kind": "stock_edge_or_helix", "angle_rad": 3f64.to_radians()},
                        });
                        cutting(&mut out, s, true)?;
                        notes.push(format!("{op_id}: clears {} mm from the walls and leaves a {} mm web around the part; it cuts the margin where vise jaws on the stock would be, so hold the stock from below or by the web", show(radial), show(num(op, "web_mm")?)));
                        out
                    }
                }
                "flats" => {
                    let rough = setup["operations"].as_array().and_then(|ops| ops.iter().find(|o| o["kind"] == "roughing"));
                    let s = tool_or_skip!(tool, notes);
                    let rough = rough.ok_or("flats with no roughing to follow")?;
                    spec = s;
                    let s_cut = cutting_data(s, op);
                    let s = &s_cut;
                    let stepover = num(s, "stepover_mm")?.min(num(s, "diameter")? / 2.);
                    let mut out = json!({
                        "kind": "flat", "id": op_id, "tool": id(s), "surface": surface(rough)?,
                        "floor_z_mm": num(op, "floor_z")?, "stepover_mm": stepover, "clearance_z_mm": clearance,
                        "retract_z_mm": retract, "feed_z_mm": retract,
                    });
                    cutting(&mut out, s, true)?;
                    out
                }
                "holes" => {
                    let d = num(op, "diameter")?;
                    let chosen = tool_or_skip!(tool, notes);
                    if chosen["type"] == "drill" {
                        let drill = chosen;
                        spec = drill;
                        let drill_cut = cutting_data(drill, op);
                        let drill = &drill_cut;
                        let (depth, reference, allow) = drill_depth(op, drill, breakthrough, stock_bottom, &op_id, &mut notes)?;
                        let open_z = if reference == "full_diameter" { depth } else { depth + drill_tip_length(drill)? };
                        full_diameter_z.extend(features_of(op).into_iter().map(|f| (f, open_z)));
                        let mut out = json!({
                            "kind": "drilling", "id": op_id, "tool": id(drill), "centers_mm": op["centers"], "depth_z_mm": depth,
                            "depth_reference": reference, "peck_mm": op["technology"]["peck_mm"], "peck_mode": op["technology"]["peck_mode"],
                            "retract_z_mm": retract, "chip_break_lift_mm": 1.0, "reentry_clearance_mm": 0.5,
                            "allow_breakthrough_mm": allow, "clearance_z_mm": clearance,
                            "retract_feed_mm_min": drill.get("retract_feed_mm_min").and_then(Value::as_f64).unwrap_or(num(drill, "feed_mm_min")?),
                        });
                        cutting(&mut out, drill, false)?;
                        out
                    } else {
                        let mill = chosen;
                        spec = mill;
                        let mill_cut = cutting_data(mill, op);
                        let mill = &mill_cut;
                        let floor = bore_floor(op, breakthrough, stock_bottom, &op_id, &mut notes)?;
                        full_diameter_z.extend(features_of(op).into_iter().map(|f| (f, floor)));
                        bore(&op_id, mill, &op["centers"], d, floor, clearance, retract)?
                    }
                }
                "counterbore" => {
                    let d = num(op, "diameter")?;
                    let mill = tool_or_skip!(tool, notes);
                    spec = mill;
                    let mill_cut = cutting_data(mill, op);
                    let mill = &mill_cut;
                    bore(&op_id, mill, &op["centers"], d, num(op, "floor_z")?, clearance, retract)?
                }
                "thread" => {
                    let thread = format!("{} ({})", op["spec"].as_str().unwrap_or(""), features_of(op).join(", "));
                    let features = features_of(op);
                    if features.iter().any(|f| !full_diameter_z.contains_key(f)) {
                        notes.push(format!("{op_id}: {thread}: the hole is not cut, so it is not threaded"));
                        continue;
                    }
                    let open_z = features.iter().map(|f| full_diameter_z[f]).fold(f64::MIN, f64::max);
                    let bottom_z = num(op, "bottom_z")?;
                    if bottom_z < open_z - 1e-6 {
                        notes.push(format!("{op_id}: {thread}: the thread would end {} mm below where the hole reaches full diameter; not threaded", show(r(open_z - bottom_z))));
                        continue;
                    }
                    let mill = tool_or_skip!(tool, notes);
                    let (major, start, pitch) = (num(op, "major")?, num(op, "start_diameter")?, num(op, "pitch")?);
                    // The neck's clearance from the drilled wall, on the thread's final path circle.
                    let neck_room = start / 2. - (major / 2. - num(mill, "diameter")? / 2.) - num(mill, "neck_diameter")? / 2.;
                    let cell = neck_room / (2f64.sqrt() * NECK_CELL_SAFETY);
                    let cells = (bound(hi, 0)? - bound(lo, 0)?) / cell * (bound(hi, 1)? - bound(lo, 1)?) / cell;
                    if cells > MAX_STOCK_CELLS {
                        notes.push(format!("{op_id}: {thread}: {}'s neck passes {} mm from the hole wall; proving that needs a {} mm stock grid, too fine for this part (use a thread mill with a narrower neck)", id(mill), show(r(neck_room)), show(r(cell))));
                        continue;
                    }
                    needs_cell = Some(needs_cell.map_or(cell, |c: f64| c.min(cell)));
                    spec = mill;
                    let mill_cut = cutting_data(mill, op);
                    let mill = &mill_cut;
                    let mut out = json!({
                        "kind": "thread_mill", "id": op_id, "tool": id(mill), "centers_mm": op["centers"], "internal": true,
                        "diameter_mm": major, "start_diameter_mm": start, "pitch_mm": pitch, "right_hand": true, "climb": true,
                        "top_z_mm": num(op, "top_z")?, "bottom_z_mm": bottom_z,
                        "radial_passes": mill.get("radial_passes").and_then(Value::as_u64).unwrap_or(RADIAL_PASSES),
                        "spring_passes": mill.get("spring_passes").and_then(Value::as_u64).unwrap_or(SPRING_PASSES),
                        "clearance_z_mm": clearance, "retract_z_mm": retract, "feed_z_mm": retract,
                    });
                    cutting(&mut out, mill, true)?;
                    notes.push(format!("{op_id}: {thread}: thread milled to the ⌀{} major diameter (pitch {}, right hand) with {}; check the fit with a gauge or screw before taking the part off", show(major), show(pitch), id(mill)));
                    out
                }
                "slot" => {
                    let width = num(op, "width")?;
                    let mill = tool_or_skip!(tool, notes);
                    spec = mill;
                    let mill_cut = cutting_data(mill, op);
                    let mill = &mill_cut;
                    let mut floor = num(op, "floor_z")?;
                    if op["bottom"] == "through" {
                        let exit = floor;
                        floor = stock_bottom.max(exit - breakthrough);
                        if floor > exit - 0.05 {
                            notes.push(format!("{op_id}: the {} mm slot is cut to the exit and no further (the stock ends there): expect a skin or burr to clean up", show(width)));
                        }
                    }
                    let mut out = json!({
                        "kind": "pocket", "id": op_id, "tool": id(mill),
                        "region": {
                            "version": 1, "units": "mm", "name": op_id, "boundary_error_mm": 0.0,
                            "components": [{"outer": {"vertices": obround(point2(&op["a"])?, point2(&op["b"])?, width / 2.)}, "holes": []}],
                            "source": {"plan": PLAN_FORMAT},
                        },
                        "top_z_mm": num(op, "top_z")?, "floor_z_mm": floor,
                        "stepdown_mm": num(mill, "stepdown_mm")?, "stepover_mm": num(mill, "stepover_mm")?.min(num(mill, "diameter")? / 2.),
                        "clearance_z_mm": clearance, "radial_allowance_mm": 0.0, "bottom_allowance_mm": 0.0,
                        "entry": {"kind": "helix", "angle_rad": 3f64.to_radians()},
                        "retract_z_mm": retract, "feed_z_mm": retract,
                    });
                    cutting(&mut out, mill, true)?;
                    out
                }
                "notch" => {
                    let mill = tool_or_skip!(tool, notes);
                    spec = mill;
                    let mill_cut = cutting_data(mill, op);
                    let mill = &mill_cut;
                    let mut floor = num(op, "floor_z")?;
                    if op["bottom"] == "through" {
                        floor = stock_bottom.max(floor - breakthrough);
                    }
                    let mut out = json!({
                        "kind": "pocket", "id": op_id, "tool": id(mill), "region": region(&op_id, &op["outer"], &json!([])),
                        "top_z_mm": num(op, "top_z")?, "floor_z_mm": floor,
                        "stepdown_mm": num(mill, "stepdown_mm")?, "stepover_mm": num(mill, "stepover_mm")?.min(num(mill, "diameter")? / 2.),
                        "clearance_z_mm": clearance, "radial_allowance_mm": 0.0, "bottom_allowance_mm": 0.0,
                        "entry": {"kind": "helix", "angle_rad": 3f64.to_radians()},
                        "retract_z_mm": retract, "feed_z_mm": retract,
                    });
                    cutting(&mut out, mill, true)?;
                    out
                }
                "v_channel" => {
                    let bit = tool_or_skip!(tool, notes);
                    spec = bit;
                    let bit_cut = cutting_data(bit, op);
                    let bit = &bit_cut;
                    let (half, depth) = ((num(bit, "angle")? / 2.).to_radians().tan(), num(op, "depth")?);
                    let left = num(op, "width")? / 2. - half * depth;
                    if left > 0.005 {
                        notes.push(format!("{op_id}: {} is a {}° bit in a {}° channel: its walls keep up to {} mm at the top", id(bit), show(num(bit, "angle")?), show(num(op, "angle")?), show(r(left))));
                    }
                    let a = point2(&op["a"])?;
                    let mut out = json!({
                        "kind": "trace", "id": op_id, "tool": id(bit),
                        "paths": [{"start_mm": a, "segments": [{"kind": "line", "end_mm": point2(&op["b"])?}]}],
                        "top_z_mm": num(op, "top_z")?, "depth_z_mm": num(op, "root_z")?, "stepdown_mm": num(bit, "stepdown_mm")?,
                        "clearance_z_mm": clearance, "retract_z_mm": retract, "feed_z_mm": retract,
                    });
                    cutting(&mut out, bit, true)?;
                    out
                }
                "wall_finish" => {
                    let mill = tool_or_skip!(tool, notes);
                    spec = mill;
                    let mill_cut = cutting_data(mill, op);
                    let mill = &mill_cut;
                    let mut out = json!({
                        "kind": "contour", "id": op_id, "tool": id(mill), "region": region(&op_id, &op["outer"], &json!([])),
                        "boundaries": "outer", "floor_z_mm": num(op, "floor_z")?, "stepdown_mm": num(mill, "stepdown_mm")?,
                        "clearance_z_mm": clearance, "radial_allowance_mm": 0.0, "bottom_allowance_mm": 0.0,
                        "boundary_tolerance_mm": 0.01, "reverse": true, "lead_in_mm": 0.0, "lead_out_mm": 0.0,
                        "retract_z_mm": retract, "feed_z_mm": retract,
                    });
                    cutting(&mut out, mill, true)?;
                    out
                }
                "edge_chamfer" => {
                    let width = num(op, "width")?;
                    let mill = tool_or_skip!(tool, notes);
                    spec = mill;
                    let mill_cut = cutting_data(mill, op);
                    let mill = &mill_cut;
                    let mut out = json!({
                        "kind": "chamfer", "id": op_id, "tool": id(mill), "region": region(&op_id, &op["outer"], &op["holes"]),
                        "boundaries": op["boundaries"], "top_z_mm": num(op, "top_z")?, "width_mm": width,
                        "clearance_z_mm": clearance, "tip_clearance_mm": TIP_CLEARANCE, "width_step_mm": null,
                        "boundary_tolerance_mm": 0.01, "reverse": false, "lead_in_mm": 0.0, "lead_out_mm": 0.0,
                        "retract_z_mm": retract, "feed_z_mm": retract,
                    });
                    cutting(&mut out, mill, true)?;
                    notes.push(format!("{op_id}: the model's r{} top edges are cut as {} mm 45° chamfers meeting the round at its middle, not as the round; the bottom edges are not cut", show(num(op, "radius")?), show(width)));
                    out
                }
                other => {
                    notes.push(format!("{op_id}: unknown operation kind {other:?}"));
                    continue;
                }
            };
            operations.push(built);
            if !used.iter().any(|u| id(u) == id(spec)) {
                used.push(spec);
            }
        }
        if operations.is_empty() {
            log.extend(notes);
            log.push(format!("setup {setup_id}: nothing to cut"));
            continue;
        }
        let (x0, y0) = (bound(lo, 0)?, bound(lo, 1)?);
        let (x1, y1) = (bound(hi, 0)?, bound(hi, 1)?);
        let cell = match library_data.get("cell_size_mm").and_then(Value::as_f64).filter(|c| *c != 0.) {
            Some(c) => c,
            None => format!("{:.2}", (x1 - x0).max(y1 - y0) / 400.).parse::<f64>().unwrap_or(0.1).max(0.1),
        };
        let cell = match needs_cell {
            Some(needed) if needed < cell => {
                // Rounded down to a whole micrometre, so the grid is never coarser than needed.
                let fine = (needed * 1000.).floor() / 1000.;
                notes.push(format!("setup {setup_id}: stock grid {} mm, fine enough to prove the thread mill's neck clear of the hole wall", show(fine)));
                fine
            }
            _ => cell,
        };
        // Where the machine leaves each new tool: the spindle's gauge line at `gauge_z` over X Y.
        let gauges = used.iter().map(|t| Ok(num(t, "stickout")? + num(t, "holder_length")?)).collect::<Result<Vec<f64>, String>>()?;
        let lowest = clearance + gauges.iter().copied().fold(f64::MIN, f64::max);
        let gauge_z = match library_data.get("tool_change_gauge_z_mm").and_then(Value::as_f64) {
            None => lowest,
            Some(z) if z < lowest => {
                return Err(format!("tool_change_gauge_z_mm {} puts a tool tip below the clearance height in setup {setup_id}: it must be at least {}", show(z), show(r(lowest))));
            }
            Some(z) => z,
        };
        let change_xy = match library_data.get("tool_change_xy_mm").and_then(Value::as_array) {
            Some(xy) if xy.len() == 2 => [xy[0].as_f64().unwrap_or(x0), xy[1].as_f64().unwrap_or(y0)],
            _ => [x0, y0],
        };
        let first = operations[0]["tool"].as_str().unwrap_or("");
        let first_gauge = used.iter().zip(&gauges).find(|(t, _)| id(t) == first).map_or(0., |(_, g)| *g);
        // The whole part is known only where roughing is planned, whether or not camlib accepted it.
        // A thread cuts past the bore the mesh shows, so the stock is compared just before threads.
        let part = setup["operations"].as_array().and_then(|ops| ops.iter().find(|o| o["kind"] == "roughing")).or_else(|| setup.get("part_surface"));
        let first_thread = operations.iter().position(|o| o["kind"] == "thread_mill");
        let material = match part.filter(|_| first_thread != Some(0)) {
            Some(rough) => {
                let z = rough["positions"].as_array().ok_or("roughing has no positions")?.iter().skip(2).step_by(3).filter_map(Value::as_f64);
                // The stock can go below the part: everything above the base is what is compared.
                let base = z.fold(bound(lo, 2)?, f64::min);
                let after = first_thread.map(|i| operations[i - 1]["id"].clone());
                if first_thread.is_some() {
                    notes.push(format!("setup {setup_id}: the finished stock is compared with the part before thread milling, which is not checked this way"));
                }
                json!([{"after_operation": after, "options": {"max_tile_nodes": COMPARISON_NODES, "max_boundary_tests": COMPARISON_BUDGET, "max_findings": COMPARISON_FINDINGS}, "target": {
                    "version": 1, "units": "mm", "id": format!("step2cam-{setup_id}-part"), "revision": "1",
                    "surface": surface(rough)?, "base_z_mm": base,
                    // surface_error_mm bounds the mesh's distance from the part both ways, so the
                    // part lies between the mesh solid eroded and dilated by it. Without saying so
                    // camlib proves no material under the mesh, and cannot find a definite gouge.
                    "source_enclosure": "offset",
                }}])
            }
            None => json!([]),
        };
        let manifest = json!({
            "version": 1,
            "run": {"id": format!("step2cam-{setup_id}"), "revision": "1"},
            "stock": {"version": 1, "units": "mm", "min_mm": lo, "max_mm": hi, "cell_size_mm": cell, "max_cells": u32::MAX},
            "tools": used.iter().map(|t| assembly(t)).collect::<Result<Vec<_>, _>>()?,
            "initial_position_mm": [change_xy[0], change_xy[1], gauge_z - first_gauge],
            "sweep": {"tolerance_mm": 1e-5, "chord_error_mm": 0.002, "max_tip_step_mm": 0.1, "max_segments": 100000,
                      "max_scratch_cells": u32::MAX, "max_changed_cells": u32::MAX, "max_cell_visits": 100000000},
            "operations": operations,
            "verification": {"scene": null, "material": material, "stock_contact": {
                "tolerance_mm": 1e-5, "spatial_resolution_mm": 0.001, "max_nodes": 200000, "max_cell_reads": 2000000, "max_findings": 128}},
            "tool_change": {"gauge_z_mm": gauge_z, "xy_mm": change_xy},
            "rapid_retracts": true,
        });
        jobs.push(Job { setup: setup_id.to_string(), manifest, notes });
    }
    Ok((jobs, log))
}
