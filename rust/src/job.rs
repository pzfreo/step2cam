//! Cam plan + tool library -> camlib jobs: which tool cuts each operation, and how deep.
//! A job is camlib's manifest, plain JSON, one per setup.
use crate::plan::{PLAN_FORMAT, r, show};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Kept between a drill tip and material found beyond a through hole's exit (mm).
const EXIT_MARGIN: f64 = 0.2;
/// Every tool needs these: no defaults, as holder size and cutting data are specific to the tool,
/// material and machine.
const TOOL_KEYS: [&str; 11] = [
    "id", "type", "number", "diameter", "overall_length", "stickout", "holder_length",
    "holder_diameter", "feed_mm_min", "spindle_rpm", "coolant",
];
/// Twice a triangle's area (mm²) at or below which camlib's surface mesh calls it degenerate, at
/// the tolerance of 1e-5 mm that the job gives it.
const NO_AREA: f64 = 1e-10;
/// Thread milling passes, unless the tool says otherwise: the thread depth in two radial steps,
/// then one spring pass at full depth.
const RADIAL_PASSES: u64 = 2;
const SPRING_PASSES: u64 = 1;
/// camlib proves a thread mill's neck clear of the hole wall beside the groove from its stock
/// grid: a cell up to its diagonal away can still count as wall. The grid is made fine enough for
/// the neck's clearance, with this much in hand...
const NECK_CELL_SAFETY: f64 = 1.25;
/// ...but no finer than a stock grid of this many cells.
const MAX_STOCK_CELLS: f64 = 16_000_000.;
/// How far out along a chamfer mill's cone the chamfer's lower edge is cut (mm), so the flank cuts.
const TIP_CLEARANCE: f64 = 0.5;

fn num(v: &Value, key: &str) -> Result<f64, String> {
    v.get(key).and_then(Value::as_f64).ok_or_else(|| format!("missing number {key}"))
}
fn id(t: &Value) -> &str {
    t["id"].as_str().unwrap_or("")
}

/// The tool library: which tool cuts what.
pub struct Library<'a> {
    data: &'a Value,
    tools: Vec<&'a Value>,
    drill_tolerance: f64,
    breakthrough: f64,
}

impl<'a> Library<'a> {
    pub fn new(data: &'a Value) -> Result<Self, String> {
        let tools: Vec<&Value> = data.get("tools").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
        if tools.is_empty() {
            return Err("the tool library has no tools".into());
        }
        for (n, tool) in tools.iter().enumerate() {
            let name = tool["id"].as_str().map_or_else(|| format!("tools[{n}]"), String::from);
            let kind = tool["type"].as_str().unwrap_or("");
            let extra: &[&str] = match kind {
                "end_mill" => &["flute_length", "plunge_mm_min", "stepdown_mm", "stepover_mm"],
                "drill" => &["flute_length", "point_angle"],
                // A single-profile thread mill: `diameter` is the tooth's, on a narrower neck.
                "thread_mill" => &["tooth_height", "neck_diameter", "neck_length", "plunge_mm_min"],
                "chamfer_mill" => &["flute_length", "angle", "tip_diameter", "plunge_mm_min"],
                other => return Err(format!("tool {name}: type must be one of end_mill, drill, thread_mill, chamfer_mill, not {other:?}")),
            };
            let missing: Vec<&str> = TOOL_KEYS.iter().chain(extra).filter(|k| tool.get(**k).is_none()).copied().collect();
            if !missing.is_empty() {
                return Err(format!("tool {name}: missing {}", missing.join(", ")));
            }
            let shank = tool.get("shank_diameter").and_then(Value::as_f64).unwrap_or(num(tool, "diameter")?);
            if num(tool, "holder_diameter")? < shank {
                return Err(format!("tool {name}: holder_diameter is smaller than the shank"));
            }
            if kind == "thread_mill" && num(tool, "neck_diameter")? >= num(tool, "diameter")? {
                return Err(format!("tool {name}: a thread mill's neck must be narrower than its tooth"));
            }
        }
        for (i, t) in tools.iter().enumerate() {
            if tools[..i].iter().any(|o| id(o) == id(t)) {
                return Err("tool ids must be unique".into());
            }
        }
        Ok(Self {
            data,
            tools,
            drill_tolerance: data.get("drill_tolerance_mm").and_then(Value::as_f64).unwrap_or(0.05),
            breakthrough: data.get("breakthrough_mm").and_then(Value::as_f64).unwrap_or(1.),
        })
    }
    fn end_mills(&self) -> Vec<&'a Value> {
        let mut mills: Vec<&Value> = self.tools.iter().copied().filter(|t| t["type"] == "end_mill").collect();
        mills.sort_by(|a, b| a["diameter"].as_f64().unwrap_or(0.).total_cmp(&b["diameter"].as_f64().unwrap_or(0.)));
        mills
    }
    fn clearing(&self, name: &str) -> Result<Option<&'a Value>, String> {
        if let Some(wanted) = self.data.get(format!("{name}_tool")).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            return self.tools.iter().copied().find(|t| id(t) == wanted).map(Some).ok_or_else(|| format!("no tool {wanted:?} in the library"));
        }
        Ok(self.end_mills().last().copied())
    }
    /// The drill closest to the diameter and never larger, at most `drill_tolerance_mm` under it.
    fn drill(&self, diameter: f64) -> Option<&'a Value> {
        let mut best: Option<&Value> = None;
        for t in self.tools.iter().copied().filter(|t| t["type"] == "drill") {
            let d = t["diameter"].as_f64().unwrap_or(0.);
            let gap = diameter - d;
            if (-1e-9..=self.drill_tolerance).contains(&gap) && best.is_none_or(|b| d > b["diameter"].as_f64().unwrap_or(0.)) {
                best = Some(t);
            }
        }
        best
    }
    /// The largest end mill that fits inside the diameter with room to helix.
    fn bore_mill(&self, diameter: f64) -> Option<&'a Value> {
        self.end_mills().into_iter().rfind(|t| t["diameter"].as_f64().unwrap_or(f64::MAX) < diameter - 0.05)
    }
    /// A 90° chamfer mill wide enough for the width, with the tip clear of the edge.
    fn chamfer_mill(&self, width: f64) -> Option<&'a Value> {
        self.tools.iter().copied().find(|t| {
            t["type"] == "chamfer_mill"
                && t["angle"].as_f64() == Some(90.)
                && t["diameter"].as_f64().unwrap_or(0.) / 2. >= t["tip_diameter"].as_f64().unwrap_or(f64::MAX) / 2. + TIP_CLEARANCE + width
        })
    }
    /// The largest thread mill that can cut this thread: its pitch range (if it states one) takes
    /// the pitch, it fits the drilled hole, its neck clears the crests and it reaches the thread's
    /// bottom. With none, the reasons the others were refused.
    fn thread_mill(&self, op: &Value, stock_top: f64) -> Result<(Option<&'a Value>, Vec<String>), String> {
        let (major, start, pitch) = (num(op, "major")?, num(op, "start_diameter")?, num(op, "pitch")?);
        let (mut best, mut why): (Option<&Value>, Vec<String>) = (None, vec![]);
        for t in self.tools.iter().copied().filter(|t| t["type"] == "thread_mill") {
            let (diameter, neck) = (num(t, "diameter")?, num(t, "neck_diameter")?);
            let room = start / 2. - (major / 2. - diameter / 2.); // from the axis-side path to the drilled wall
            let reach = num(t, "tooth_height")? + num(t, "neck_length")?;
            let below = stock_top - num(op, "bottom_z")?;
            let lowest = t.get("min_pitch_mm").and_then(Value::as_f64).unwrap_or(0.);
            let highest = t.get("max_pitch_mm").and_then(Value::as_f64).unwrap_or(f64::INFINITY);
            if !(lowest <= pitch && pitch <= highest) {
                why.push(format!("{}: pitch {} outside its range", id(t), show(pitch)));
            } else if diameter >= start - 0.05 {
                why.push(format!("{}: ⌀{} does not fit the ⌀{} hole", id(t), t["diameter"], show(start)));
            } else if neck / 2. >= room - 0.01 {
                why.push(format!("{}: its ⌀{} neck would rub the thread crests", id(t), t["neck_diameter"]));
            } else if reach <= below + 0.01 {
                why.push(format!("{}: reaches {} mm, the thread ends {} mm below the stock top", id(t), show(reach), show(r(below))));
            } else if best.is_none_or(|b| diameter > b["diameter"].as_f64().unwrap_or(0.)) {
                best = Some(t);
            }
        }
        Ok((best, why))
    }
}

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

/// The cutting data every operation carries.
fn cutting(op: &mut Value, spec: &Value, plunge: bool) -> Result<(), String> {
    let number = spec["number"].clone();
    op["feed_mm_min"] = json!(num(spec, "feed_mm_min")?);
    op["tolerance_mm"] = json!(1e-5);
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

fn drill_tip_length(spec: &Value) -> Result<f64, String> {
    Ok(num(spec, "diameter")? / 2. / (num(spec, "point_angle")?.to_radians() / 2.).tan())
}

/// (depth_z, depth_reference, allow_breakthrough) for drilling one hole group.
fn drill_depth(op: &Value, drill: &Value, library: &Library, stock_bottom: f64, op_id: &str, notes: &mut Vec<String>) -> Result<(f64, &'static str, f64), String> {
    let (d, bottom_z) = (num(op, "diameter")?, num(op, "bottom_z")?);
    let tip = drill_tip_length(drill)?;
    let kind = op["bottom"].as_str().unwrap_or("unknown");
    if kind == "through" {
        let want = library.breakthrough;
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
fn bore_floor(op: &Value, library: &Library, stock_bottom: f64, op_id: &str, notes: &mut Vec<String>) -> Result<f64, String> {
    let bottom_z = num(op, "bottom_z")?;
    if op["bottom"] != "through" {
        return Ok(bottom_z);
    }
    let mut past = library.breakthrough;
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

/// One camlib job for each setup of a plan that has something it can cut.
pub fn jobs(plan: &Value, library_data: &Value) -> Result<(Vec<Job>, Vec<String>), String> {
    if plan["format"] != PLAN_FORMAT {
        return Err("not a step2cam plan".into());
    }
    let library = Library::new(library_data)?;
    let machine = library_data.get("machine").filter(|m| m.as_object().is_some_and(|o| !o.is_empty()))
        .ok_or("the tool library needs a `machine` (camlib assumes nothing about it)")?;
    if machine["tool_change_position_mode"] != "table" {
        return Err("machine.tool_change_position_mode must be \"table\" (the only mode wired up)".into());
    }
    if machine.get("dialect").and_then(Value::as_str).is_some_and(|d| d != "linuxcnc") {
        return Err("only the linuxcnc dialect is wired up".into());
    }
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
            let spec: &Value;
            let built = match kind {
                "facing" | "roughing" => {
                    let Some(s) = library.clearing(kind)? else {
                        notes.push(format!("{op_id}: no end mill to {} with", if kind == "facing" { "face" } else { "rough" }));
                        continue;
                    };
                    spec = s;
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
                "holes" => {
                    let d = num(op, "diameter")?;
                    if let Some(drill) = library.drill(d) {
                        spec = drill;
                        let (depth, reference, allow) = drill_depth(op, drill, &library, stock_bottom, &op_id, &mut notes)?;
                        let open_z = if reference == "full_diameter" { depth } else { depth + drill_tip_length(drill)? };
                        full_diameter_z.extend(features_of(op).into_iter().map(|f| (f, open_z)));
                        let mut out = json!({
                            "kind": "drilling", "id": op_id, "tool": id(drill), "centers_mm": op["centers"], "depth_z_mm": depth,
                            "depth_reference": reference, "peck_mm": drill.get("peck_mm").filter(|p| truthy_number(p)), "peck_mode": "full_retract",
                            "retract_z_mm": retract, "chip_break_lift_mm": 1.0, "reentry_clearance_mm": 0.5,
                            "allow_breakthrough_mm": allow, "clearance_z_mm": clearance,
                            "retract_feed_mm_min": drill.get("retract_feed_mm_min").and_then(Value::as_f64).unwrap_or(num(drill, "feed_mm_min")?),
                        });
                        cutting(&mut out, drill, false)?;
                        out
                    } else if let Some(mill) = library.bore_mill(d) {
                        spec = mill;
                        let floor = bore_floor(op, &library, stock_bottom, &op_id, &mut notes)?;
                        full_diameter_z.extend(features_of(op).into_iter().map(|f| (f, floor)));
                        bore(&op_id, mill, &op["centers"], d, floor, clearance, retract)?
                    } else {
                        let features: Vec<&str> = op["features"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                        notes.push(format!("{op_id}: no drill of ⌀{} and no end mill small enough to bore it ({})", show(d), features.join(", ")));
                        continue;
                    }
                }
                "counterbore" => {
                    let d = num(op, "diameter")?;
                    let Some(mill) = library.bore_mill(d) else {
                        notes.push(format!("{op_id}: no end mill small enough for the ⌀{} counterbore", show(d)));
                        continue;
                    };
                    spec = mill;
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
                    let (mill, why) = library.thread_mill(op, top)?;
                    let Some(mill) = mill else {
                        let reasons = if why.is_empty() { String::new() } else { format!(" ({})", why.join("; ")) };
                        notes.push(format!("{op_id}: {thread}: no thread mill can cut it{reasons}"));
                        continue;
                    };
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
                    let Some(mill) = library.bore_mill(width) else {
                        notes.push(format!("{op_id}: no end mill small enough for the {} mm slot ({})", show(width), features_of(op).join(", ")));
                        continue;
                    };
                    spec = mill;
                    let mut floor = num(op, "floor_z")?;
                    if op["bottom"] == "through" {
                        let exit = floor;
                        floor = stock_bottom.max(exit - library.breakthrough);
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
                "wall_finish" => {
                    // Only for a chamfer: other walls keep the roughing allowance, as ever.
                    let width_needed = setup["operations"].as_array().into_iter().flatten().find(|o| o["kind"] == "edge_chamfer").map_or(Ok(0.), |o| num(o, "width"))?;
                    if library.chamfer_mill(width_needed).is_none() {
                        notes.push(format!("{op_id}: no chamfer mill for the edges, so the wall keeps its roughing allowance"));
                        continue;
                    }
                    let Some(mill) = library.clearing("roughing")? else {
                        notes.push(format!("{op_id}: no end mill to finish the wall with"));
                        continue;
                    };
                    spec = mill;
                    let mut out = json!({
                        "kind": "contour", "id": op_id, "tool": id(mill), "region": region(&op_id, &op["outer"], &json!([])),
                        "boundaries": "outer", "floor_z_mm": num(op, "floor_z")?, "stepdown_mm": num(mill, "stepdown_mm")?,
                        "clearance_z_mm": clearance, "radial_allowance_mm": 0.0, "bottom_allowance_mm": 0.0,
                        "boundary_tolerance_mm": 0.01, "reverse": false, "lead_in_mm": 0.0, "lead_out_mm": 0.0,
                        "retract_z_mm": retract, "feed_z_mm": retract,
                    });
                    cutting(&mut out, mill, true)?;
                    out
                }
                "edge_chamfer" => {
                    let width = num(op, "width")?;
                    let Some(mill) = library.chamfer_mill(width) else {
                        notes.push(format!("{op_id}: no 90° chamfer mill wide enough for the r{} edges, so they stay sharp", show(num(op, "radius")?)));
                        continue;
                    };
                    spec = mill;
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
            log.push(format!("setup {setup_id}: nothing to post"));
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
        // The whole part is known only where it is roughed: the final stock is compared with it.
        let material = match setup["operations"].as_array().and_then(|ops| ops.iter().find(|o| o["kind"] == "roughing")) {
            Some(rough) => {
                let z = rough["positions"].as_array().ok_or("roughing has no positions")?.iter().skip(2).step_by(3).filter_map(Value::as_f64);
                let base = z.fold(f64::INFINITY, f64::min);
                json!([{"after_operation": null, "target": {
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

fn truthy_number(v: &Value) -> bool {
    v.as_f64().is_some_and(|x| x != 0.)
}
