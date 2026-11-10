//! The camlib adapter: a plan whose operations name their tools (`capp::finish`) + the tool
//! library -> camlib jobs, how each tool cuts and how deep. A job is camlib's manifest, plain
//! JSON, one per setup.
use capp::plan::{PLAN_FORMAT, r, show};
use capp::resources::{TIP_CLEARANCE, DRILLING_TOLERANCE, EXIT_MARGIN, drill_tip_length, id, num, tap_through_travel};
use serde_json::{Value, json};
use std::collections::HashMap;

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
    let drill = spec["type"] == "drill" || spec["type"] == "spot_drill";
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
        let angle = num(spec, "angle")?.to_radians();
        let tip = num(spec, "tip_diameter")?;
        let flute_length = num(spec, "flute_length")?;
        let cone_height = (diameter / 2. - tip / 2.) / (angle / 2.).tan();
        // Match camlib's cone calculation when nominal equality differs only by
        // floating-point roundoff. Keep genuinely short flutes for camlib to reject.
        let tolerance = 16. * f64::EPSILON * flute_length.abs().max(1.);
        let cutting_length = if cone_height.is_finite() && (cone_height - flute_length).abs() <= tolerance {
            flute_length.max(cone_height)
        } else {
            flute_length
        };
        return Ok(json!({
            "id": name,
            "cutter": {
                "id": name, "name": name,
                "shape": {"kind": "chamfer", "included_angle_rad": angle, "tip_diameter_mm": tip},
                "diameter_mm": diameter, "cutting_length_mm": cutting_length, "overall_length_mm": num(spec, "overall_length")?,
                "shank_diameter_mm": spec.get("shank_diameter").and_then(Value::as_f64).unwrap_or(diameter),
                "flutes": 3, "center_cutting": true, "material": "carbide", "neck": null,
            },
            "stickout_mm": stickout,
            "gauge_length_mm": gauge,
            "holders": [{"z_min_mm": stickout, "z_max_mm": gauge, "radius_min_mm": radius, "radius_max_mm": radius}],
        }));
    }
    let tap = spec["type"] == "tap";
    let cutter_diameter = if tap { capp::reference::tap_drill(diameter, num(spec, "pitch")?, spec["tap_type"] == "form").ok_or("no tap drill reference")? } else { diameter };
    let shape = if drill {
        json!({"kind": "drill", "point_angle_rad": num(spec, "point_angle")?.to_radians()})
    } else {
        json!({"kind": "flat"})
    };
    Ok(json!({
        "id": name,
        "cutter": {
            "id": name, "name": name, "shape": shape, "diameter_mm": cutter_diameter,
            "cutting_length_mm": num(spec, if tap { "thread_length" } else { "flute_length" })?, "overall_length_mm": num(spec, "overall_length")?,
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

#[derive(Clone)]
pub struct Job {
    /// Export the finished stock only when a successor needs it.
    pub return_stock: bool,
    /// Source-frame part bounds after a declared release/cut-free step.
    pub crop_stock_to: Option<[[f64; 3]; 2]>,
    pub setup: String,
    /// camlib's manifest.
    pub manifest: Value,
    /// Real setup stock bottom; the camlib simulation may include tap backing below it.
    pub stock_bottom_mm: f64,
    pub notes: Vec<String>,
    /// Pocket mouths that preceding roughing may have exposed. Camlib must admit each.
    pub local_pocket_tops: HashMap<String, f64>,
}

/// One camlib job for each setup of a plan whose operations name their tools (`capp::resolve`).
/// The operation kinds a plan holds, each of which `capp::resolve` gives a tool or a note.
const KNOWN: [&str; 12] = ["facing", "roughing", "flats", "spot_drill", "holes", "counterbore", "thread", "slot", "notch", "v_channel", "wall_finish", "edge_chamfer"];

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
        let mut stock_lo = setup["stock"]["min"].clone();
        let mut tap_travel: HashMap<String, f64> = HashMap::new();
        for op in setup["operations"].as_array().into_iter().flatten().filter(|o| o["kind"] == "thread" && o["through"] == true) {
            if let Some(tap) = tools.iter().find(|t| t["id"] == op["tool"] && t["type"] == "tap") {
                let extra = tap_through_travel(tap, num(op, "pitch")?)?;
                for feature in op["features"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    tap_travel.insert(feature.to_owned(), extra);
                }
                let floor = num(op, "bottom_z")? - extra;
                stock_lo[2] = json!(stock_lo[2].as_f64().ok_or("bad stock")?.min(floor));
            }
        }
        // camlib requires the tapping tip plus clearance within StockSpec. Model the extra
        // through travel as conservative solid backing, drilled away before the tap enters.
        // This lower bound belongs only to simulation: every machining limit and material
        // comparison still uses the setup's real stock bottom.
        let (lo, hi) = (&stock_lo, &setup["stock"]["max"]);
        let bound = |v: &Value, i: usize| v[i].as_f64().ok_or("bad stock");
        let top = bound(hi, 2)?;
        let stock_bottom = bound(&setup["stock"]["min"], 2)?;
        let breakthrough = if setup["flip_role"] == "sacrificial_grip" {
            breakthrough.min(capp::rules::workholding()["flip_breakthrough_mm"].as_f64().unwrap())
        } else { breakthrough };
        let clearance = top + clearance_above;
        let retract = (top + retract_above).min(clearance);

        let mut local_pocket_tops = HashMap::new();
        let (mut operations, mut used, mut notes): (Vec<Value>, Vec<&Value>, Vec<String>) = (vec![], vec![], vec![]);
        // How deep each hole is open at its full diameter, once cut: a thread must not go lower.
        let mut full_diameter_z: HashMap<String, f64> = HashMap::new();
        let mut drill_tip_z: HashMap<String, f64> = HashMap::new();
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
                        if let Some(margin) = setup["flip_seating"]["residual_margin_mm"].as_f64() {
                            out["edge_overlap_mm"] = json!(margin);
                        }
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
                        if setup["flip_role"] == "sacrificial_grip" && num(op, "web_mm")? == 0. {
                            notes.push(format!("{op_id}: clears {} mm from the walls to the part bottom; sacrificial bottom stock retains the grip and is faced off in bottom setup", show(radial)));
                        } else {
                            let separation = crate::setup_stock::breakout(setup, &plan["part"]);
                            let left = match separation.as_ref() {
                                Some(step) if step["kind"] == "cut_free" => format!("leaves a {} mm band of uncut walls below Z{}", show(num(op,"web_mm")?), show(num(op,"floor_z")?)),
                                Some(_) => format!("leaves a {} mm web below the part bottom",show(num(op,"web_mm")?)),
                                None => format!("leaves stock below Z{}",show(num(op,"floor_z")?)),
                            };
                            notes.push(format!("{op_id}: clears {} mm from the walls and {left}; side cutting respects the setup clamping cap when vise jaw dimensions are supplied; stock below the cap is left for a later setup", show(radial)));
                        }
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
                "spot_drill" => {
                    let spec = tool_or_skip!(tool, notes);
                    let cut = cutting_data(spec, op);
                    // A plan group can contain different mouth heights and spot diameters.
                    // camlib has one absolute tip Z per drilling operation.
                    let mut depths: Vec<(f64, Vec<Value>)> = vec![];
                    for spot in op["spots"].as_array().ok_or("spot operation has no spots")? {
                        let z = num(spot, "top_z")? - num(spot, "depth")?;
                        if let Some((_, centers)) = depths.iter_mut().find(|(depth, _)| *depth == z) {
                            centers.push(spot["center"].clone());
                        } else {
                            depths.push((z, vec![spot["center"].clone()]));
                        }
                    }
                    for (i, (z, centers)) in depths.into_iter().enumerate() {
                        let mut out = json!({
                            "kind":"drilling", "id":format!("{op_id}-{}", i + 1), "tool":id(spec),
                            "centers_mm":centers, "depth_z_mm":z, "depth_reference":"tip",
                            "peck_mm":null, "peck_mode":"full_retract", "dwell_s":0.2,
                            "retract_z_mm":retract, "chip_break_lift_mm":1.0, "reentry_clearance_mm":0.5,
                            "allow_breakthrough_mm":0., "clearance_z_mm":clearance,
                            "retract_feed_mm_min":num(&cut, "feed_mm_min")?,
                        });
                        cutting(&mut out, &cut, false)?;
                        operations.push(out);
                    }
                    if !used.iter().any(|t| id(t) == id(spec)) { used.push(spec); }
                    continue;
                }
                "holes" => {
                    let d = num(op, "diameter")?;
                    let breakthrough = features_of(op).iter().filter_map(|f| tap_travel.get(f)).copied().fold(breakthrough, f64::max);
                    let chosen = tool_or_skip!(tool, notes);
                    if chosen["type"] == "drill" {
                        let drill = chosen;
                        spec = drill;
                        let drill_cut = cutting_data(drill, op);
                        let drill = &drill_cut;
                        let (depth, reference, allow) = drill_depth(op, drill, breakthrough, stock_bottom, &op_id, &mut notes)?;
                        let open_z = if reference == "full_diameter" { depth } else { depth + drill_tip_length(drill)? };
                        full_diameter_z.extend(features_of(op).into_iter().map(|f| (f, open_z)));
                        let tip_z = if reference == "full_diameter" { depth - drill_tip_length(drill)? } else { depth };
                        drill_tip_z.extend(features_of(op).into_iter().map(|f| (f, tip_z)));
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
                    if mill["type"] == "tap" {
                        let tip = bottom_z - num(mill, "chamfer_pitches")? * pitch;
                        if tip - pitch < open_z - 1e-6 {
                            notes.push(format!("{op_id}: {thread}: tap tip Z{tip:.3} plus {pitch:.3} mm bottom clearance needs full diameter to Z{:.3}; actual drilled floor Z{open_z:.3}; not tapped", tip - pitch));
                            continue;
                        }
                        spec = mill;
                        let tap = cutting_data(mill, op);
                        let out = json!({
                            "kind": "tapping", "id": op_id, "tool": id(mill), "centers_mm": op["centers"],
                            "pitch_mm": pitch, "hand": mill.get("hand").and_then(Value::as_str).unwrap_or("right"),
                            "depth_z_mm": tip, "bottom_clearance_mm": pitch,
                            "clearance_z_mm": clearance, "retract_z_mm": retract, "feed_z_mm": retract,
                            "tolerance_mm": DRILLING_TOLERANCE, "spindle_rpm": num(&tap, "spindle_rpm")?,
                            "work_offset": 1, "tool_number": mill["number"],
                            "length_register": mill.get("length_register").unwrap_or(&mill["number"]), "coolant": tap["coolant"],
                        });
                        // Tapping derives synchronized feed from pitch × rpm; no feed field.
                        notes.push(format!("{op_id}: {thread}: tapped {} (pitch {}, {} hand), full thread {} mm, tip Z{} with {}; check with a gauge", op["spec"].as_str().unwrap_or(""), show(pitch), mill.get("hand").and_then(Value::as_str).unwrap_or("right"), show(num(op, "top_z")? - bottom_z), show(tip), id(mill)));
                        if op["through"] == true {
                            // Transform the part's model-space box into this setup's Z axis.
                            let part_bottom = (0..3).try_fold(0., |z, i| {
                                let direction = bound(&setup["rotation"][2], i)?;
                                let edge = if direction < 0. { "max" } else { "min" };
                                Ok::<_, String>(z + direction * (bound(&plan["part"][edge], i)? - bound(&setup["origin_model"], i)?))
                            })?;
                            let below = |z: f64| format!("{:.2}", ((part_bottom - z).max(0.) * 100. - 1e-6).ceil() / 100.).trim_end_matches('0').trim_end_matches('.').to_string();
                            let drill_tip = features.iter().filter_map(|f| drill_tip_z.get(f)).copied().reduce(f64::min);
                            if let Some(drill_tip) = drill_tip {
                                notes.push(format!("{op_id}: the tap tip goes {} mm and its drill tip {} mm below the part's bottom (drill full diameter {} mm below): keep the hole clear of parallels", below(tip), below(drill_tip), below(open_z)));
                            }
                        }
                        out
                    } else {
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
                        // camlib retains CCW internal climb milling and reverses Z feed
                        // for a left-hand helix (top-down instead of bottom-up).
                        let right_hand = op["hand"] != "left";
                        let hand = if right_hand { "right" } else { "left" };
                        let mut out = json!({
                            "kind": "thread_mill", "id": op_id, "tool": id(mill), "centers_mm": op["centers"], "internal": true,
                            "diameter_mm": major, "start_diameter_mm": start, "pitch_mm": pitch, "right_hand": right_hand, "climb": true,
                            "top_z_mm": num(op, "top_z")?, "bottom_z_mm": bottom_z,
                            "radial_passes": mill.get("radial_passes").and_then(Value::as_u64).unwrap_or(RADIAL_PASSES),
                            "spring_passes": mill.get("spring_passes").and_then(Value::as_u64).unwrap_or(SPRING_PASSES),
                            "clearance_z_mm": clearance, "retract_z_mm": retract, "feed_z_mm": retract,
                        });
                        cutting(&mut out, mill, true)?;
                        notes.push(format!("{op_id}: {thread}: thread milled to the ⌀{} major diameter (pitch {}, {hand} hand) with {}; check the fit with a gauge or screw before taking the part off", show(major), show(pitch), id(mill)));
                        out
                    }
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
                    // A prior roughing pass may have exposed the local pocket mouth.
                    // Camlib scans the remaining stock and proves assembly clearance before
                    // admitting this lower top; this declaration is not a clearance verdict.
                    let local_top = op["wall_top"].as_f64().filter(|z| operations.iter().any(|o|
                        o["kind"] == "roughing" && o["floor_z_mm"].as_f64().is_some_and(|floor| floor <= *z)));
                    if let Some(z) = local_top { local_pocket_tops.insert(op_id.clone(), z); }
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
                        "clearance_z_mm": clearance, "tip_clearance_mm": TIP_CLEARANCE, "width_step_mm": width.min(1.0),
                        "boundary_tolerance_mm": 0.01, "reverse": false, "lead_in_mm": 0.0, "lead_out_mm": 0.0,
                        "retract_z_mm": retract, "feed_z_mm": retract,
                    });
                    cutting(&mut out, mill, true)?;
                    if op["radius"].is_number() { notes.push(format!("{op_id}: the model's r{} top edges are cut as {} mm 45° chamfers meeting the round at its middle, not as the round; the bottom edges are not cut", show(num(op, "radius")?), show(width))); }
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
        for op in &mut operations {
            let gauge = used.iter().zip(&gauges).find(|(t, _)| t["id"] == op["tool"]).map(|(_, g)| *g).ok_or("operation tool has no gauge length")?;
            let tip = gauge_z - gauge;
            let clearance = num(op, "clearance_z_mm")?;
            // Canonicalize the same intended plane after gauge subtraction.
            // Fractional carried tops can otherwise introduce a sub-nanometre
            // lift that camlib correctly refuses as a zero-length line.
            let roundoff = 8. * f64::EPSILON * gauge_z.abs().max(gauge.abs()).max(clearance.abs()).max(1.);
            if (setup["flip_seating"].is_object() && op["kind"] == "facing") || (tip-clearance).abs() <= roundoff {
                op["clearance_z_mm"] = json!(tip);
            }
        }
        // The whole part is known only where roughing is planned, whether or not camlib accepted it.
        // A thread cuts past the bore the mesh shows, so the stock is compared just before threads.
        let part = setup["operations"].as_array().and_then(|ops| ops.iter().find(|o| o["kind"] == "roughing")).or_else(|| setup.get("part_surface"));
        // Tapping uses the pre-drill diameter and retains thread flanks; unlike thread
        // milling it does not cut beyond the modelled bore, so comparison stays enabled.
        let first_thread = operations.iter().position(|o| o["kind"] == "thread_mill");
        let material = match part.filter(|_| first_thread != Some(0)) {
            Some(rough) => {
                let z = rough["positions"].as_array().ok_or("roughing has no positions")?.iter().skip(2).step_by(3).filter_map(Value::as_f64);
                // The stock can go below the part: everything above the base is what is compared.
                let base = z.fold(stock_bottom, f64::min);
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
        let scene = if let Some(clamping) = setup.get("clamping") {
            let bodies = ["jaws", "parallels"].into_iter().flat_map(|key| clamping[key].as_array().into_iter().flatten()).map(|body| {
                let (lo, hi) = (&body["min"], &body["max"]);
                json!({"id": body["id"], "role": "fixture", "z_min_mm": lo[2], "z_max_mm": hi[2], "z_error_mm": 0.,
                    "footprint": {"version": 1, "units": "mm", "name": body["id"], "boundary_error_mm": 0.,
                        "components": [{"outer": {"vertices": [[lo[0], lo[1], 0.], [hi[0], lo[1], 0.], [hi[0], hi[1], 0.], [lo[0], hi[1], 0.]]}, "holes": []}]}})
            }).collect::<Vec<_>>();
            json!({"scene": {"version": 1, "units": "mm", "id": format!("{setup_id}-workholding"), "revision": "1",
                "work_to_machine": camlib::motion::WorkFrame::identity(), "bodies": bodies, "machine": null}})
        } else { Value::Null };
        let manifest = json!({
            "version": 1,
            "run": {"id": format!("step2cam-{setup_id}"), "revision": "1"},
            "stock": {"version": 1, "units": "mm", "min_mm": lo, "max_mm": hi, "cell_size_mm": cell, "max_cells": u32::MAX},
            "tools": used.iter().map(|t| assembly(t)).collect::<Result<Vec<_>, _>>()?,
            "initial_position_mm": [change_xy[0], change_xy[1], gauge_z - first_gauge],
            "sweep": {"tolerance_mm": 1e-5, "chord_error_mm": 0.002, "max_tip_step_mm": 0.1, "max_segments": 100000,
                      "max_scratch_cells": u32::MAX, "max_changed_cells": u32::MAX, "max_cell_visits": 100000000},
            "operations": operations,
            "verification": {"scene": scene, "material": material, "stock_contact": {
                "tolerance_mm": 1e-5, "spatial_resolution_mm": 0.001, "max_nodes": 200000, "max_cell_reads": 2000000, "max_findings": 128}},
            "tool_change": {"gauge_z_mm": gauge_z, "xy_mm": change_xy},
            "rapid_retracts": true,
        });
        jobs.push(Job { return_stock: false, crop_stock_to: None, setup: setup_id.to_string(), manifest, stock_bottom_mm: stock_bottom, notes, local_pocket_tops });
    }
    Ok((jobs, log))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chamfer_cone_height_accepts_nominal_equality_without_extending_short_flutes() {
        let mut spec = json!({
            "id": "cm12", "type": "chamfer_mill", "diameter": 12.,
            "tip_diameter": 2.5, "angle": 90., "flute_length": 4.75,
            "overall_length": 70., "stickout": 15.,
            "holder_length": 76.2, "holder_diameter": 50.,
        });
        let cutter = |spec: &Value| -> camlib::tool::Cutter {
            serde_json::from_value(assembly(spec).unwrap()["cutter"].clone()).unwrap()
        };
        let equal = cutter(&spec);
        assert!(equal.prepare().is_ok(), "nominal cone-height flutes must be accepted");
        assert_eq!(spec["flute_length"], 4.75, "catalogue input must remain unchanged");

        spec["flute_length"] = json!(4.74);
        let short = cutter(&spec);
        assert_eq!(short.cutting_length_mm, 4.74);
        assert_eq!(short.prepare().err().unwrap(), "cutting length does not contain the complete tool tip geometry");

        spec["flute_length"] = json!(6.);
        let longer = cutter(&spec);
        assert_eq!(longer.cutting_length_mm, 6.);
        assert!(longer.prepare().is_ok());
    }
}
