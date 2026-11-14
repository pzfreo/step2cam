//! specify-core analysis (+ mesh) -> cam plan: what to cut, per setup, independent of tools and
//! machine. Each setup is the part turned so its tool comes down -Z, with X0 Y0 on the minimum
//! corner and Z0 on the top of the stock (the roughing setup) or of the part's box (later ones).
use crate::model;
use crate::trace::Trace;
use serde_json::{Value, json};
use std::cell::OnceCell;
use std::collections::{BTreeMap, HashMap};

mod edges;
mod frame;
mod holes;
mod mesh;
mod recesses;
mod requirements;
mod section;
mod values;
mod walls;

use edges::*;
use frame::*;
use holes::*;
use mesh::*;
use recesses::*;
use requirements::*;
use section::*;
use values::*;
use walls::*;

pub use frame::between_setups;
pub use mesh::disk_meets_for_test;
pub(crate) use mesh::mesh_triangles;
pub use values::{assembly_requirement, display_position, operation_noun, r, show};

pub const PLAN_FORMAT: &str = "specify-cam-plan";

type V3 = [f64; 3];
type Rows = [[f64; 3]; 3];
type Tri = [V3; 3];

/// The directions a tool can come from, by the hole axis it enters along (from the opening into
/// the hole), as rotations (rows) that turn that axis to -Z. Each is a proper rotation.
const SETUPS: [(&str, V3, Rows); 6] = [
    ("top", [0., 0., -1.], [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]),
    ("bottom", [0., 0., 1.], [[1., 0., 0.], [0., -1., 0.], [0., 0., -1.]]),
    ("front", [0., 1., 0.], [[1., 0., 0.], [0., 0., 1.], [0., -1., 0.]]),
    ("back", [0., -1., 0.], [[1., 0., 0.], [0., 0., -1.], [0., 1., 0.]]),
    ("left", [1., 0., 0.], [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]]),
    ("right", [-1., 0., 0.], [[0., 0., -1.], [0., 1., 0.], [1., 0., 0.]]),
];
/// An axis counts as along a setup direction within this (cosine); oblique holes are skipped.
const AXIS_TOLERANCE: f64 = 1e-6;
/// Mesh points this close to a level count as on it (mm).
const LEVEL_EPS: f64 = 0.01;
/// camlib refuses a roughing mesh whose stated error is larger than this (mm).
const CAMLIB_MAX_SURFACE_ERROR: f64 = 0.1;
/// A direction counts as along an axis (as upright, in a setup's rows) when its cosine with that
/// axis is beyond this: within about 8°.
const ALONG_AXIS: f64 = 0.99;
/// Points closer than this are the same point (mm).
const SAME_POINT: f64 = 1e-7;
/// Just above a level, so that the level's own faces do not count as standing on it (mm).
const ABOVE_FLOOR: f64 = 1e-3;
/// The most coordinate rounding moves a mesh point: 1e-4 mm on each axis, times √3.
const ROUNDING_ERROR: f64 = 1e-4 * 1.7320508075688772;

/// Stock and setup choices.
#[derive(Clone, Debug)]
pub struct Options {
    pub margin: f64,
    pub top_stock: f64,
    pub bottom_stock: f64,
    pub skin: f64,
    /// Maximum transverse bar width accepted by the selected vise.
    pub stock_width_limit: Option<f64>,
    pub rough_from: String,
}

impl Default for Options {
    fn default() -> Self {
        Self { margin: 2., top_stock: 1., bottom_stock: 0., skin: 0.5, stock_width_limit: None, rough_from: "top".into() }
    }
}

struct Context<'a> {
    frames: &'a HashMap<&'static str, Frame>,
    preferred: &'a [&'static str],
    triangles: Option<&'a [Tri]>,
    part_faces: &'a HashMap<&'static str, f64>,
    all_features: &'a [Value],
}

impl Context<'_> {
    /// A feature's id, with those of the flat-wall features recognised from its own faces.
    fn absorbed(&self, feature: &Value) -> Vec<String> {
        let own: Vec<u64> = feature["faces"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
        let mut ids = vec![feature["id"].as_str().unwrap_or("").to_string()];
        for other in self.all_features {
            let faces: Vec<u64> = other["faces"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
            if other["family"] == "step_levels" && !faces.is_empty() && faces.iter().all(|x| own.contains(x)) {
                ids.push(other["id"].as_str().unwrap_or("").to_string());
            }
        }
        ids
    }
}

/// The part's box in model coordinates: quiddity gives `bounds` in its dimensioning `frame`.
fn model_bounds(dimensioning: &Value) -> Result<Option<(V3, V3)>, String> {
    let Some(b) = dimensioning.get("bounds").filter(|b| truthy(Some(b))) else { return Ok(None) };
    let (lo, hi) = (v3(&b[0])?, v3(&b[1])?);
    let Some(frame) = dimensioning.get("frame").filter(|x| truthy(Some(x))) else { return Ok(Some((lo, hi))) };
    let origin = v3(&frame["origin"])?;
    let axes = [v3(&frame["x"])?, v3(&frame["y"])?, v3(&frame["z"])?];
    let corners = box_corners(lo, hi).map(|c| {
        let mut p = origin;
        for k in 0..3 {
            p = add(p, scale(axes[k], c[k]));
        }
        p
    });
    Ok(Some(bounds(&corners)))
}

/// Whether roughing from a setup can reach a feature: none of its flat faces points down in it.
fn reachable(feature: &Value, faces: &HashMap<u64, &Value>, rows: &Rows) -> Result<bool, String> {
    for index in feature.get("faces").and_then(Value::as_array).into_iter().flatten() {
        if let Some(face) = index.as_u64().and_then(|i| faces.get(&i))
            && face["kind"].as_str() == Some("plane")
            && truthy(face.get("direction"))
            && rotate(rows, v3(&face["direction"])?)[2] < -0.01
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether a feature is all cylinders along a setup's tool: a corner of a wall, which that setup's
/// roughing follows, not a rounded edge.
fn upright(feature: &Value, faces: &HashMap<u64, &Value>, rows: &Rows) -> Result<bool, String> {
    for index in feature.get("faces").and_then(Value::as_array).into_iter().flatten() {
        let Some(face) = index.as_u64().and_then(|i| faces.get(&i)) else { return Ok(false) };
        if face["kind"].as_str() != Some("cylinder") || rotate(rows, v3(&face["direction"])?)[2].abs() < ALONG_AXIS {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The cam plan for an analysis (and optionally its mesh).
pub fn plan(analysis: &Value, mesh: Option<&Value>, options: &Options) -> Result<Value, String> {
    serde_json::to_value(build(analysis, mesh, options)?).map_err(|e| e.to_string())
}

/// Least-area published section containing the blank and fitting the vise.
pub(crate) fn standard_bar(size: [f64; 3], limit: Option<f64>) -> Option<[f64; 2]> {
    let bars = &crate::rules::standard_sizes()["flat_bar"];
    bars["sizes_mm"].as_array().unwrap().iter().flat_map(|bar| {
        let a = bar[0].as_f64().unwrap();
        let b = bar[1].as_f64().unwrap();
        [[a,b],[b,a]]
    }).filter(|bar| bar[0] >= size[2] && bar[1] >= size[usize::from(size[0] > size[1])] && limit.is_none_or(|max| bar[1] <= max))
        .min_by(|a,b| (a[0]*a[1]).total_cmp(&(b[0]*b[1])).then(a[0].total_cmp(&b[0])))
}

/// The cam plan for an analysis (and optionally its mesh), as types.
pub fn build(analysis: &Value, mesh: Option<&Value>, options: &Options) -> Result<model::Plan, String> {
    if analysis.get("features").is_none() || analysis.get("dimensioning").is_none() {
        return Err("not a specify-core analysis".into());
    }
    let mut part_box = model_bounds(&analysis["dimensioning"])?;
    if let Some(m) = mesh {
        if m["format"].as_str() != Some("pmi-assist-mesh") {
            return Err("not a specify-core mesh".into());
        }
        if m.get("binding") != analysis.get("binding") {
            return Err("the mesh and the analysis are of different files or parts".into());
        }
        if truthy(m.get("bbox")) {
            part_box = Some((v3(&m["bbox"][0])?, v3(&m["bbox"][1])?));
        }
    }
    let (part_lo, part_hi) = part_box.ok_or("the analysis has no part bounds")?;
    let rough_from: &'static str = SETUPS
        .iter()
        .map(|s| s.0)
        .find(|n| *n == options.rough_from)
        .ok_or_else(|| format!("rough_from must be one of {}", SETUPS.map(|s| s.0).join(", ")))?;

    // The stock is the part's box grown by the margins: top and bottom along the rough-from
    // direction, the margin across it.
    let up = rows_of(rough_from)[2];
    let (mut stock_lo, mut stock_hi) = (part_lo, part_hi);
    for i in 0..3 {
        let (grow_lo, grow_hi) = if up[i] > 0.5 {
            (options.bottom_stock, options.top_stock)
        } else if up[i] < -0.5 {
            (options.top_stock, options.bottom_stock)
        } else {
            (options.margin, options.margin)
        };
        stock_lo[i] -= grow_lo;
        stock_hi[i] += grow_hi;
    }
    // Select in the roughing frame, then grow the model box along those axes.
    let rows = rows_of(rough_from);
    let axes: [usize; 3] = rows.map(|row| row.iter().position(|v| v.abs() > 0.5).unwrap());
    let size = axes.map(|i| stock_hi[i] - stock_lo[i]);
    let width = usize::from(size[0] > size[1]);
    let length = 1 - width;
    let bars = &crate::rules::standard_sizes()["flat_bar"];
    let chosen_bar = standard_bar(size, options.stock_width_limit);
    let stock_size = if let Some(bar) = chosen_bar {
        let mut target = size;
        target[2] = bar[0];
        target[width] = bar[1];
        target[length] = size[length].ceil();
        for local in 0..3 {
            let i = axes[local];
            let extra = target[local] - size[local];
            if local == 2 {
                if rows[2][i] > 0. { stock_lo[i] -= extra; } else { stock_hi[i] += extra; }
            } else {
                stock_lo[i] -= extra / 2.;
                stock_hi[i] += extra / 2.;
            }
        }
        json!({"kind":"standard", "source":bars["source"], "cross_section_mm":bar,
            "sawn_length_mm":target[length], "allocation":"extra thickness below the part; width and sawn-length extra split equally between sides, keeping all margins as minimums"})
    } else { json!({"kind":"computed", "source":bars["source"], "requested_mm":size}) };
    let frames: HashMap<&'static str, Frame> = SETUPS
        .iter()
        .map(|(name, _, _)| {
            let frame = if *name == rough_from {
                Frame::new(name, stock_lo, stock_hi, "stock")
            } else {
                Frame::new(name, part_lo, part_hi, "part")
            };
            (*name, frame)
        })
        .collect();
    let part_faces: HashMap<&'static str, f64> = frames
        .iter()
        .map(|(n, fr)| (*n, bounds(&box_corners(part_lo, part_hi).map(|p| fr.point(p))).0[2]))
        .collect();
    let missing = mesh.is_some_and(|m| truthy(m.get("missing")));
    let triangles = match mesh {
        Some(m) if !missing => Some(mesh_triangles(m)?),
        _ => None,
    };
    let preferred: Vec<&'static str> = std::iter::once(rough_from)
        .chain(SETUPS.iter().map(|s| s.0).filter(|n| *n != rough_from))
        .collect();
    let faces: HashMap<u64, &Value> = analysis
        .get("faces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| x["id"].as_u64().map(|id| (id, x)))
        .collect();

    let all_features: &[Value] = analysis["features"].as_array().ok_or("features is not a list")?;
    let cx = Context { frames: &frames, preferred: &preferred, triangles: triangles.as_deref(), part_faces: &part_faces, all_features };
    let mut by_setup: HashMap<&'static str, Vec<Hole>> = HashMap::new();
    let mut slots: HashMap<&'static str, Vec<model::Cut>> = HashMap::new();
    let mut trace = Trace::default();
    if chosen_bar.is_none() {
        trace.warn("setup.standard_sizes", rough_from, format!("no standard flat bar in {} holds {} × {} × {} mm; stock is the computed size", bars["source"]["ref"].as_str().unwrap(), show(size[0]), show(size[1]), show(size[2])), json!({"chosen":{"stock_size":stock_size}}));
    } else {
        trace.decide("setup.standard_sizes", rough_from, "chosen", "standard flat bar, not checked against your shop; extra thickness below the part to preserve the top margin and tool reach; sides evenly to preserve minimum margins", json!({"chosen":stock_size}));
    }
    let mut covered: Vec<String> = vec![];
    let mut roughing_feature_depths = BTreeMap::new();
    // The decision that put each hole in its setup.
    let mut placed: HashMap<String, String> = HashMap::new();
    let asked = threads(analysis, &mut trace);
    not_acted_on(analysis, &mut trace);
    if mesh.is_none() {
        trace.warn("process.no_mesh_checks", "part", "no mesh: hole approaches and what lies beyond through holes were not checked against the part".to_string(), json!({}));
    }
    let mut in_slot: Vec<String> = vec![];
    for feature in all_features.iter().filter(|x| x["family"] == "slots") {
        if let Some((name, op)) = slot_operation(feature, &cx)? {
            in_slot.extend(op.features().unwrap_or_default().iter().cloned());
            slots.entry(name).or_default().push(op);
        }
    }
    // Slots that are not round-ended, notches and V-channels: what roughing's mill cannot get into.
    for feature in all_features.iter().filter(|x| ["slots", "pockets", "rectangular_blind_slots", "v_channels"].contains(&x["family"].as_str().unwrap_or(""))) {
        let id = feature["id"].as_str().unwrap_or("");
        if in_slot.iter().any(|x| x == id) {
            continue;
        }
        let planned = if feature["family"] == "v_channels" { v_channel_operation(feature, &cx)? } else { notch_operation(feature, &cx)? };
        if let Some((name, op)) = planned {
            in_slot.extend(op.features().unwrap_or_default().iter().cloned());
            slots.entry(name).or_default().push(op);
        }
    }
    for feature in all_features {
        let family = feature["family"].as_str().unwrap_or("");
        let id = feature["id"].as_str().unwrap_or("");
        // What becomes of it: the first case in rulebase/rules/process.json that holds.
        let case = crate::rules::case_for(family, &mut |condition| match condition {
            "cut_by_slot" => Ok(in_slot.iter().any(|x| x == id)),
            "convex" => Ok(feature["record"]["side"] == "convex"),
            "not_upright" => Ok(!upright(feature, &faces, rows_of(rough_from))?),
            "no_mesh" => Ok(triangles.is_none()),
            "reachable" => reachable(feature, &faces, rows_of(rough_from)),
            other => Err(format!("rulebase/rules/process.json: unknown condition {other}")),
        })?;
        match &case["then"] {
            Value::String(action) if action == "hole" => match hole_geometry(feature, &cx, asked.get(id))? {
                Ok(Placed { setup: name, hole, warnings: w, blocked }) => {
                    let considered: Vec<Value> = blocked.iter().map(|b| json!({"candidate": b, "rejected": "part material above its opening"})).collect();
                    let says = format!("cut from the {name} setup");
                    placed.insert(id.to_string(), trace.decide("setup.hole.first_reachable", id, "chosen", &says, json!({"chosen": name, "considered": considered})));
                    by_setup.entry(name).or_default().push(hole);
                    for (rule, text) in w {
                        trace.warn(rule, id, text, json!({"features": [id]}));
                    }
                }
                Err(Refused { rule, reason, needs }) => {
                    let skipped = trace.skip(rule, id, reason);
                    trace.needs(&skipped, needs);
                    if let Some(thread) = asked.get(id) {
                        trace.skip("process.hole.thread_lost", id, format!("its {} thread: the hole is not cut", thread.spec));
                    }
                }
            },
            // Cut by its slot's operation, or its member holes are planned one by one.
            Value::String(action) if action == "by_slot" || action == "members" => {}
            Value::String(action) if action == "covered" => {
                trace.decide("process.covered_by_roughing", id, "covered", &format!("{family}: no operation of its own; roughing reaches it"), Value::Null);
                covered.push(id.to_string());
                // Mesh face ranges use analysis face ids. Unknown or unmeshed faces fall
                // back to the part bottom; curved faces include camlib's maximum accepted
                // surface error so tessellation cannot overstate reach.
                let frame = &frames[rough_from];
                let bottom = bounds(&box_corners(part_lo, part_hi).map(|p| frame.point(p))).0[2];
                let lowest = feature["faces"].as_array().filter(|ids| !ids.is_empty()).and_then(|ids| {
                    ids.iter().map(|face| {
                        let index = face.as_u64()?;
                        let range = mesh?["faces"].get(index as usize)?;
                        let start = range[0].as_u64()? as usize;
                        let count = range[1].as_u64()? as usize;
                        let points = triangles.as_ref()?.get(start..start + count)?;
                        let low = points.iter().flatten().map(|p| frame.point(*p)[2]).min_by(f64::total_cmp)?;
                        let allowance = if faces.get(&index).is_some_and(|f| f["kind"] == "plane") { ROUNDING_ERROR } else { CAMLIB_MAX_SURFACE_ERROR };
                        Some(low - allowance)
                    }).collect::<Option<Vec<_>>>()?.into_iter().min_by(f64::total_cmp)
                }).unwrap_or(bottom);
                roughing_feature_depths.insert(id.to_string(), model::Depth { floor_z: lowest, family: family.into() });
            }
            Value::Object(skip) => {
                let rule = skip["skip"].as_str().ok_or("rulebase/rules/process.json: a skip names no rule")?;
                let mut reason = skip["reason"].as_str().unwrap_or("").replace("{family}", family).replace("{rough_from}", rough_from);
                if reason.contains("{radius}") {
                    reason = reason.replace("{radius}", &show(f(&feature["record"], "radius")?));
                }
                if matches!(family, "chamfers" | "oriented_chamfers") {
                    let rec = &feature["record"];
                    let width = f(rec, "leg1")?;
                    let cause = if (width - f(rec, "leg2")?).abs() > LEVEL_EPS {
                        Some("chamfer: unequal legs; only equal-leg 45° chamfers are planned")
                    } else if (f(rec, "angle")? - 45.).abs() > 0.01 {
                        Some("chamfer: angle is not 45°; needs a matching certified operation")
                    } else if triangles.is_none() {
                        Some("chamfer: no mesh to establish a reachable top loop")
                    } else if !reachable(feature, &faces, rows_of(rough_from))? {
                        Some("chamfer: bottom or downward-facing edge is unreachable from the roughing setup")
                    } else if feature["faces"].as_array().is_some_and(|ids| !ids.is_empty() && ids.iter().all(|id| {
                        id.as_u64().and_then(|id| faces.get(&id)).is_some_and(|face| {
                            face["kind"] == "plane" && v3(&face["direction"]).is_ok_and(|n| rotate(rows_of(rough_from), n)[2].abs() < 0.01)
                        })
                    })) {
                        Some("chamfer: vertical-edge chamfer; a top-down 90° chamfer mill cannot follow it")
                    } else {
                        None
                    };
                    if let Some(cause) = cause {
                        reason = format!("{cause}, so the edge stays sharp (chamfer it by hand)");
                    }
                    reason.push_str(&format!("; leg {} mm", show(width)));
                }
                trace.skip(rule, id, reason);
            }
            other => return Err(format!("rulebase/rules/process.json: unknown action {other}")),
        }
    }

    let mut setups = vec![];
    for name in &preferred {
        let frame = &frames[name];
        let empty = vec![];
        let holes = by_setup.get(name).unwrap_or(&empty);
        let setup_slots = slots.remove(name).unwrap_or_default();
        let mut ops: Vec<model::Cut> = vec![];
        if *name == rough_from {
            let part = bounds(&box_corners(part_lo, part_hi).map(|p| frame.point(p)));
            let (part_top, part_bottom) = (part.1[2], part.0[2]);
            if frame.stock.1[2] - part_top > 1e-6 {
                ops.push(model::Cut::Facing { floor_z: r(part_top) });
            }
            if let Some(m) = mesh {
                let floor = (part_bottom + options.skin).min(part_top);
                if missing {
                    let list: Vec<String> = m["missing"].as_array().into_iter().flatten().map(Value::to_string).collect();
                    trace.skip("process.roughing.missing_faces", "roughing", format!(
                        "faces [{}] did not triangulate: a mesh with holes in it would let roughing cut into the part",
                        list.join(", ")
                    ));
                } else {
                    // specify-core meshes curved faces to 0.1% of the part's diagonal.
                    let d = [part_hi[0] - part_lo[0], part_hi[1] - part_lo[1], part_hi[2] - part_lo[2]];
                    let error = surface_error(m, &faces, dot(d, d).sqrt() * 0.001)?;
                    if error > CAMLIB_MAX_SURFACE_ERROR {
                        trace.skip("process.roughing.mesh_error", "roughing", format!(
                            "the mesh strays up to {} mm from the part's curved faces; camlib accepts at most {} mm",
                            show(r(error)), show(CAMLIB_MAX_SURFACE_ERROR)
                        ));
                    } else {
                        let positions: Vec<f64> = nums(&m["positions"])?
                            .as_chunks::<3>()
                            .0
                            .iter()
                            .flat_map(|p| frame.point([p[0], p[1], p[2]]).map(r))
                            .collect();
                        ops.push(model::Cut::Roughing(model::Roughing {
                            top_z: r(frame.stock.1[2]),
                            floor_z: r(floor),
                            web_mm: r(floor - part_bottom),
                            // Rounded up, so the rounding never understates it.
                            surface_error_mm: (error * 1e4).ceil() / 1e4,
                            positions: Some(positions),
                            indices: Some(serde_json::from_value(m["indices"].clone()).map_err(|e| format!("mesh indices: {e}"))?),
                        }));
                        // Flats between the top, which facing cuts, and the floor are off the
                        // roughing layers, so they can have up to a stepdown left on them.
                        let top = bounds(&box_corners(part_lo, part_hi).map(|p| frame.point(p))).1[2];
                        let mut levels: Vec<f64> = faces
                            .values()
                            .filter(|f| f["kind"] == "plane")
                            .filter_map(|f| {
                                let up = rotate(frame.rows, v3(&f["direction"]).ok()?)[2] > 1. - 1e-9;
                                up.then(|| Some(r(frame.point(v3(&f["centroid"]).ok()?)[2])))?
                            })
                            .filter(|z| *z > floor && *z < r(top))
                            .collect();
                        levels.sort_by(f64::total_cmp);
                        levels.dedup();
                        if !levels.is_empty() {
                            ops.push(model::Cut::Flats { floor_z: r(floor), levels });
                        }
                    }
                }
            }
        }
        if ops.is_empty() && holes.is_empty() && setup_slots.is_empty() {
            continue;
        }
        ops.extend(hole_operations(holes));
        ops.extend(counterbore_operations(holes));
        let mut chamfered: Vec<String> = vec![];
        if let (Some(m), Some(all), true) = (mesh, triangles.as_deref(), ops.iter().any(|o| matches!(o, model::Cut::Roughing(_)))) {
            let part = bounds(&box_corners(part_lo, part_hi).map(|p| frame.point(p)));
            let floor = (part.0[2] + options.skin).min(part.1[2]);
            let mut heights = vec![floor];
            heights.extend(ops.iter().find_map(|o| match o { model::Cut::Flats { levels, .. } => Some(levels), _ => None }).into_iter().flatten().copied());
            heights.push(part.1[2]);
            let error = ops.iter().find_map(|o| match o { model::Cut::Roughing(c) => Some(c.surface_error_mm), _ => None }).unwrap_or(0.);
            let own = (2. * error).max(0.01);
            let (walls, outlines) = match m["faces"].as_array() {
                Some(ranges) => wall_operations(frame.triangles(all), ranges, &faces, frame, rows_of(name), &heights, own, &mut trace, name)?,
                None => (vec![], vec![]),
            };
            // Modelled chamfers may be on a lower flat; rounds retain their floor rule.
            let wall_of = |outer: &[Cut], floor_only: bool| {
                walls.iter().zip(&outlines).find(|(w, o)| {
                    if floor_only && w.floor_z != r(floor) { return false; }
                    let hull;
                    let o = if !floor_only && (area(o).abs()-area(outer).abs()).abs() > 0.005*area(outer).abs() {
                        hull = convex_perimeter(o).unwrap_or_default();
                        &hull
                    } else { o };
                    (area(o).abs() - area(outer).abs()).abs() <= 0.005 * area(outer).abs() && o.iter().all(|c| outer.iter().any(|d| segment_distance(c.0, d.0, d.1) <= 0.1))
                }).map(|(w, _)| w.wall.clone())
            };
            let edges;
            (edges, chamfered) = edge_operations(m, all, &cx, &faces, name, part.1[2], floor, holes, &setup_slots, &|o| wall_of(o, true))?;
            let (modelled, ids) = modelled_edge_operations(m, all, &cx, &faces, name, (holes, &setup_slots), &|o| wall_of(o, false))?;
            chamfered.extend(ids);
            ops.extend(setup_slots);
            ops.extend(walls.into_iter().map(model::Cut::WallFinish));
            ops.extend(edges);
            ops.extend(modelled);
        } else {
            ops.extend(setup_slots);
        }
        // Last: nothing cuts near a finished thread, and the stock can be compared with the part
        // just before, since a thread goes beyond the bore the part's mesh shows.
        ops.extend(thread_operations(holes));
        for h in holes {
            if !h.csink.is_null() {
                trace.skip("process.countersink.not_cut", &h.feature, "countersink: not planned (chamfer it by hand)".into());
            }
        }
        // In the order rulebase/rules/sequence.json gives; the same rank keeps the planned order.
        let mut ranked = Vec::with_capacity(ops.len());
        for op in ops {
            ranked.push((crate::rules::rank(op.kind().name())?, op));
        }
        ranked.sort_by_key(|(rank, _)| *rank);
        let ops: Vec<model::Cut> = ranked.into_iter().map(|(_, op)| op).collect();
        // Why each operation is here: its setup, the order, the rule that made it, and for holes
        // the decision that put them in this setup.
        let stock = json!({"min": rp(frame.stock.0), "max": rp(frame.stock.1)});
        let in_setup = trace.decide("setup.stock.part_box_with_margins", name, "chosen", &format!("the {name} setup, zeroed on the {}", frame.zeroed_on), json!({"zero": frame.zeroed_on, "stock": stock}));
        let order = trace.decide("sequence.fixed_order", name, "chosen", "facing, roughing, flats, spot drills, holes, counterbores, slots, notches and V-channels, walls, edge chamfers, threads last", Value::Null);
        let mut operations = Vec::with_capacity(ops.len());
        for cut in ops {
            let kind = cut.kind().name();
            let features: Vec<String> = cut.features().unwrap_or_default().to_vec();
            let rule = cut.kind().process_rule();
            let says = if features.is_empty() { format!("{kind} in the {name} setup") } else { format!("{kind} in the {name} setup: {}", features.join(", ")) };
            let made = trace.decide(rule, &format!("{name}:{kind}"), "planned", &says, json!({"features": features}));
            let holes_placed: Vec<String> = features.iter().filter_map(|f| placed.get(f).cloned()).collect();
            if kind == "edge_chamfer" {
                // Rounds skipped earlier are cut as this chamfer after all.
                for id in features.iter().filter(|f| chamfered.contains(f)) {
                    trace.supersede(id, "skipped", &made);
                }
            }
            let because = [in_setup.clone(), order.clone(), made].into_iter().chain(holes_placed).collect();
            operations.push(model::Operation { cut, tool: None, tool_note: None, because: Some(because), iso14649: None, technology: None });
        }
        trace.skipped.retain(|s| !chamfered.contains(&s.feature));
        let mut setup = frame.describe(name, operations);
        setup.stock_size = Some(if *name == rough_from { stock_size.clone() } else { json!({"kind":"remaining", "from_setup":rough_from}) });
        setups.push(setup);
    }

    // Nothing is covered by roughing that does not run.
    let roughs = setups.iter().any(|s| s.operations.iter().any(|o| matches!(o.cut, model::Cut::Roughing(_))));
    if !roughs && triangles.is_some() {
        for id in covered.drain(..) {
            let not_cut = trace.skip("process.no_roughing", &id, "no operation of its own, and roughing is not planned".into());
            trace.supersede(&id, "covered", &not_cut);
        }
    }

    // Only analytically planar faces are certain contact surfaces. Curved
    // tessellation facets must never become apparent flat jaw contact.
    let contact = mesh.and_then(|m| m["faces"].as_array()).map(|ranges| {
        ranges.iter().enumerate().filter(|(id, _)| faces.get(&(*id as u64)).is_some_and(|f| f["kind"] == "plane"))
            .flat_map(|(_, range)| {
                let start = range[0].as_u64().unwrap_or(0) as usize;
                let count = range[1].as_u64().unwrap_or(0) as usize;
                triangles.as_deref().unwrap_or(&[]).get(start..start.saturating_add(count)).unwrap_or(&[]).iter().copied()
            }).collect::<Vec<_>>()
    });
    Ok(model::Plan {
        format: Some(PLAN_FORMAT.into()),
        version: Some(1),
        units: Some("mm".into()),
        source: Some(analysis.get("binding").cloned().unwrap_or(Value::Null)),
        // The material as Specify was told it (part.material); capp::finish places it in a group.
        material: Some(json!({"specified": analysis["resume"]["answers"]["part.material"].as_str()})),
        part: Some(model::Box3 { min: rp(part_lo), max: rp(part_hi) }),
        workholding_surface_model: Some(json!(triangles)),
        contact_surface_model: Some(json!(contact)),
        setups,
        covered_by_roughing: covered,
        roughing_feature_depths: Some(roughing_feature_depths),
        skipped: trace.skipped,
        warnings: trace.warnings,
        decisions: Some(trace.decisions),
        rulebase: Some(crate::trace::rulebase()),
        tools_needed: None,
        supplied_decisions: None,
        workholding: None,
    })
}
