//! specify-core analysis (+ mesh) -> cam plan: what to cut, per setup, independent of tools and
//! machine. Each setup is the part turned so its tool comes down -Z, with X0 Y0 on the minimum
//! corner and Z0 on the top of the stock (the roughing setup) or of the part's box (later ones).
use crate::trace::{Trace, because};
use serde_json::{Map, Value, json};
use std::cell::OnceCell;
use std::collections::HashMap;

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

/// Stock and setup choices.
#[derive(Clone, Debug)]
pub struct Options {
    pub margin: f64,
    pub top_stock: f64,
    pub bottom_stock: f64,
    pub skin: f64,
    pub rough_from: String,
}

impl Default for Options {
    fn default() -> Self {
        Self { margin: 2., top_stock: 1., bottom_stock: 0., skin: 0.5, rough_from: "top".into() }
    }
}

fn dot(a: V3, b: V3) -> f64 {
    let mut sum = 0.;
    for i in 0..3 {
        sum += a[i] * b[i];
    }
    sum
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn rotate(rows: &Rows, p: V3) -> V3 {
    [dot(rows[0], p), dot(rows[1], p), dot(rows[2], p)]
}
fn unit(v: V3) -> Result<V3, String> {
    let n = dot(v, v).sqrt();
    if n == 0. || !n.is_finite() {
        return Err(format!("axis {} has no direction", show3(v)));
    }
    Ok([v[0] / n, v[1] / n, v[2] / n])
}
/// Rounded for the plan, with no negative zero: the nearest 4-decimal value.
pub fn r(x: f64) -> f64 {
    format!("{x:.4}").parse::<f64>().unwrap_or(x) + 0.0
}
fn rp(p: V3) -> Value {
    json!([r(p[0]), r(p[1]), r(p[2])])
}
/// A number as messages show it.
pub fn show(x: f64) -> String {
    format!("{x:?}")
}
/// Display a position with the plan's precision, without trailing zeroes.
pub fn display_position(value: f64) -> String {
    r(value).to_string()
}

/// Join a tool query to camlib's assembly clearance requirement in one sentence.
pub fn assembly_requirement(query: &str, part: &str, depth: f64) -> String {
    let join = if query.contains(" with ") { "and" } else { "with" };
    format!("{query} {join} its {part} at least {:.0} mm above the tip", (depth + 0.001).ceil())
}

/// A short operation noun for reader-facing reasons; identifiers belong to the trace.
pub fn operation_noun(kind: &str) -> &'static str {
    match kind {
        "facing" => "facing pass",
        "roughing" => "roughing pass",
        "flats" => "flat finishing pass",
        "spot_drill" => "spot drilling",
        "holes" => "holes",
        "counterbore" => "counterbore",
        "slot" => "slot",
        "notch" => "notch",
        "v_channel" => "V-channel",
        "wall_finish" => "wall finishing pass",
        "edge_chamfer" => "chamfer",
        "thread" => "thread",
        _ => "operation",
    }
}
fn show3(p: V3) -> String {
    format!("[{}, {}, {}]", show(p[0]), show(p[1]), show(p[2]))
}
fn setup_for(axis: V3) -> Result<Option<&'static str>, String> {
    let a = unit(axis)?;
    Ok(SETUPS.iter().find(|(_, d, _)| dot(a, *d) > 1. - AXIS_TOLERANCE).map(|(n, _, _)| *n))
}
fn rows_of(name: &str) -> &'static Rows {
    &SETUPS.iter().find(|(n, _, _)| *n == name).expect("known setup").2
}
fn box_corners(lo: V3, hi: V3) -> [V3; 8] {
    let mut out = [[0.; 3]; 8];
    let mut n = 0;
    for x in [lo[0], hi[0]] {
        for y in [lo[1], hi[1]] {
            for z in [lo[2], hi[2]] {
                out[n] = [x, y, z];
                n += 1;
            }
        }
    }
    out
}
fn bounds(points: &[V3]) -> (V3, V3) {
    let mut lo = points[0];
    let mut hi = points[0];
    for p in points {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    (lo, hi)
}

/// One setup's coordinates: model point -> rotate -> translate to the setup's zero.
struct Frame {
    rows: &'static Rows,
    zeroed_on: &'static str,
    shift: V3,
    stock: (V3, V3),
    origin_model: V3,
    triangles: OnceCell<Vec<Tri>>,
}

impl Frame {
    fn new(name: &str, box_lo: V3, box_hi: V3, zeroed_on: &'static str) -> Self {
        let rows = rows_of(name);
        let (lo, hi) = bounds(&box_corners(box_lo, box_hi).map(|p| rotate(rows, p)));
        let shift = [-lo[0], -lo[1], -hi[2]];
        let point = |p: V3| add(rotate(rows, p), shift);
        let stock = bounds(&box_corners(box_lo, box_hi).map(point));
        // The rows are orthonormal, so the inverse is the transpose.
        let back = [-shift[0], -shift[1], -shift[2]];
        let mut origin_model = [0.; 3];
        for j in 0..3 {
            let mut sum = 0.;
            for i in 0..3 {
                sum += rows[i][j] * back[i];
            }
            origin_model[j] = sum;
        }
        Self { rows, zeroed_on, shift, stock, origin_model, triangles: OnceCell::new() }
    }
    fn point(&self, p: V3) -> V3 {
        add(rotate(self.rows, p), self.shift)
    }
    fn triangles(&self, mesh: &[Tri]) -> &[Tri] {
        self.triangles.get_or_init(|| mesh.iter().map(|t| t.map(|p| self.point(p))).collect())
    }
    fn describe(&self, out: &mut Map<String, Value>) {
        out.insert("zero".into(), json!(self.zeroed_on));
        let rotation: Vec<Vec<i64>> = self.rows.iter().map(|row| row.iter().map(|v| *v as i64).collect()).collect();
        out.insert("rotation".into(), json!(rotation));
        out.insert("origin_model".into(), rp(self.origin_model));
        out.insert("stock".into(), json!({"min": rp(self.stock.0), "max": rp(self.stock.1)}));
    }
}

/// Rigid transform from one recorded setup frame to another. Uses the same
/// rotation and origin that Frame::describe records for operations and stock.
/// R_to * transpose(R_from), R_to * (origin_from - origin_to).
pub fn between_setups(from: &Value, to: &Value) -> Result<(Rows, V3), String> {
    let read_rows = |s: &Value| -> Result<Rows, String> {
        Ok([v3(&s["rotation"][0])?, v3(&s["rotation"][1])?, v3(&s["rotation"][2])?])
    };
    let a = read_rows(from)?;
    let b = read_rows(to)?;
    let rotation = std::array::from_fn(|i| std::array::from_fn(|j| dot(b[i], a[j])));
    let offset = add(v3(&from["origin_model"])?, scale(v3(&to["origin_model"])?, -1.));
    Ok((rotation, rotate(&b, offset)))
}

fn nums(v: &Value) -> Result<Vec<f64>, String> {
    v.as_array()
        .ok_or("expected a list of numbers")?
        .iter()
        .map(|x| x.as_f64().ok_or_else(|| "expected a number".to_string()))
        .collect()
}
fn v3(v: &Value) -> Result<V3, String> {
    let n = nums(v)?;
    if n.len() != 3 {
        return Err("expected three numbers".into());
    }
    Ok([n[0], n[1], n[2]])
}
fn f(v: &Value, key: &str) -> Result<f64, String> {
    v.get(key).and_then(Value::as_f64).ok_or_else(|| format!("missing number {key}"))
}
/// Whether a record field says anything: present, and not null, false, empty or zero.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.),
    }
}

fn mesh_triangles(mesh: &Value) -> Result<Vec<Tri>, String> {
    let p = nums(&mesh["positions"])?;
    let idx = mesh["indices"].as_array().ok_or("mesh has no indices")?;
    let point = |i: usize| -> Result<V3, String> {
        if 3 * i + 2 >= p.len() {
            return Err("mesh index out of range".into());
        }
        Ok([p[3 * i], p[3 * i + 1], p[3 * i + 2]])
    };
    let mut out = Vec::with_capacity(idx.len() / 3);
    for t in idx.chunks_exact(3) {
        let i = |k: usize| t[k].as_u64().map(|v| v as usize).ok_or("bad mesh index");
        out.push([point(i(0)?)?, point(i(1)?)?, point(i(2)?)?]);
    }
    Ok(out)
}

fn side(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}
fn inside(c: (f64, f64), t: [(f64, f64); 3]) -> bool {
    // A vertical triangle projects to a line: nothing is inside it, and its edges are measured.
    if side(t[0], t[1], t[2]).abs() < 1e-9 {
        return false;
    }
    let (s1, s2, s3) = (side(t[0], t[1], c), side(t[1], t[2], c), side(t[2], t[0], c));
    (s1 >= 0. && s2 >= 0. && s3 >= 0.) || (s1 <= 0. && s2 <= 0. && s3 <= 0.)
}
fn segment_distance(c: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length == 0. { 0. } else { (((c.0 - a.0) * dx + (c.1 - a.1) * dy) / length).clamp(0., 1.) };
    (c.0 - (a.0 + t * dx)).hypot(c.1 - (a.1 + t * dy))
}
/// Whether the XY projection of a triangle comes within `radius` of `c`.
fn disk_meets(c: (f64, f64), radius: f64, t: &Tri) -> bool {
    let p = t.map(|v| (v[0], v[1]));
    inside(c, p) || (0..3).any(|i| segment_distance(c, p[i], p[(i + 1) % 3]) < radius)
}
#[doc(hidden)]
pub fn disk_meets_for_test(c: (f64, f64), radius: f64, t: &[[f64; 3]; 3]) -> bool {
    disk_meets(c, radius, t)
}
/// Whether any part material lies above `z` within `radius` of `c` (setup frame).
fn blocked_above(triangles: &[Tri], c: (f64, f64), z: f64, radius: f64) -> bool {
    let radius = radius - LEVEL_EPS;
    triangles.iter().any(|t| {
        t[0][2].max(t[1][2]).max(t[2][2]) > z + LEVEL_EPS && disk_meets(c, radius, t)
    })
}
/// How far below `z` the column at `c` is free of part material, or None if nothing is below.
fn clear_below(triangles: &[Tri], c: (f64, f64), z: f64, radius: f64) -> Option<f64> {
    let radius = radius - LEVEL_EPS;
    let mut found: Option<f64> = None;
    for t in triangles {
        if t[0][2].min(t[1][2]).min(t[2][2]) < z - LEVEL_EPS && disk_meets(c, radius, t) {
            let gap = (z - t[0][2].max(t[1][2]).max(t[2][2])).max(0.);
            found = Some(found.map_or(gap, |g| g.min(gap)));
        }
    }
    found
}
fn distance_to_triangle(c: (f64, f64), t: [(f64, f64); 3]) -> f64 {
    if inside(c, t) {
        return 0.;
    }
    (0..3).map(|i| segment_distance(c, t[i], t[(i + 1) % 3])).fold(f64::INFINITY, f64::min)
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
/// How far a cylinder face's flat triangles stray from the cylinder: its radius less the closest
/// any triangle comes to the axis.
fn cylinder_error(triangles: &[Tri], face: &Value) -> Result<f64, String> {
    let axis = unit(v3(&face["direction"])?)?;
    let helper = if axis[0].abs() < 0.9 { [1., 0., 0.] } else { [0., 1., 0.] };
    let u = unit(cross(axis, helper))?;
    let w = cross(axis, u);
    let origin = v3(&face["origin"])?;
    let radius = f(face, "radius")?;
    let mut worst: f64 = 0.;
    for t in triangles {
        let flat = t.map(|p| {
            let q = add(p, scale(origin, -1.));
            (dot(q, u), dot(q, w))
        });
        worst = worst.max(radius - distance_to_triangle((0., 0.), flat));
    }
    Ok(worst)
}
/// How far a torus face's flat triangles stray from the torus, sampled across each triangle; None
/// if the mesh does not fit a torus of the face's axis and (minor) radius. The analysis does not
/// give the major radius: it is the one the mesh's vertices agree on.
fn torus_error(triangles: &[Tri], face: &Value) -> Result<Option<f64>, String> {
    let axis = unit(v3(&face["direction"])?)?;
    let origin = v3(&face["origin"])?;
    let minor = f(face, "radius")?;
    let polar = |p: V3| {
        let q = add(p, scale(origin, -1.));
        let along = dot(q, axis);
        ((dot(q, q) - along * along).max(0.).sqrt(), along)
    };
    let fits = |major: f64| triangles.iter().flatten().all(|p| {
        let (rho, h) = polar(*p);
        ((rho - major).hypot(h) - minor).abs() <= 1e-3
    });
    let Some(first) = triangles.first().map(|t| polar(t[0])) else { return Ok(Some(0.)) };
    let offset = minor.mul_add(minor, -first.1 * first.1).max(0.).sqrt();
    let Some(major) = [first.0 - offset, first.0 + offset].into_iter().find(|m| fits(*m)) else { return Ok(None) };
    const STEPS: usize = 8;
    let mut worst: f64 = 0.;
    for t in triangles {
        for i in 0..=STEPS {
            for j in 0..=STEPS - i {
                let (u, v) = (i as f64 / STEPS as f64, j as f64 / STEPS as f64);
                let w = 1. - u - v;
                let p = [0, 1, 2].map(|k| u * t[0][k] + v * t[1][k] + w * t[2][k]);
                let (rho, h) = polar(p);
                worst = worst.max(((rho - major).hypot(h) - minor).abs());
            }
        }
    }
    Ok(Some(worst))
}

/// How far a cone face's flat triangles stray from the cone, sampled across each triangle; None if
/// its vertices do not lie on one. The analysis gives its axis: the slope is the vertices' own.
fn cone_error(triangles: &[Tri], face: &Value) -> Result<Option<f64>, String> {
    let axis = unit(v3(&face["direction"])?)?;
    let origin = v3(&face["origin"])?;
    let polar = |p: V3| {
        let q = add(p, scale(origin, -1.));
        let along = dot(q, axis);
        ((dot(q, q) - along * along).max(0.).sqrt(), along)
    };
    let points: Vec<(f64, f64)> = triangles.iter().flatten().map(|p| polar(*p)).collect();
    let (Some(low), Some(high)) = (points.iter().copied().min_by(|a, b| a.1.total_cmp(&b.1)), points.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1))) else {
        return Ok(Some(0.));
    };
    if high.1 - low.1 < 1e-6 {
        return Ok(None);
    }
    let slope = (high.0 - low.0) / (high.1 - low.1);
    // Distance from the cone's line, in the plane through the axis.
    let off = |p: (f64, f64)| (p.0 - (low.0 + slope * (p.1 - low.1))).abs() / slope.hypot(1.);
    if points.iter().any(|p| off(*p) > 1e-3) {
        return Ok(None);
    }
    const STEPS: usize = 8;
    let mut worst: f64 = 0.;
    for t in triangles {
        for i in 0..=STEPS {
            for j in 0..=STEPS - i {
                let (u, v) = (i as f64 / STEPS as f64, j as f64 / STEPS as f64);
                let w = 1. - u - v;
                worst = worst.max(off(polar([0, 1, 2].map(|k| u * t[0][k] + v * t[1][k] + w * t[2][k]))));
            }
        }
    }
    Ok(Some(worst))
}

/// The most the mesh strays from the part: none on flat faces, measured on cylinders, tori and cones, and
/// specify-core's chordal bound on other curved faces; plus coordinate rounding.
fn surface_error(mesh: &Value, faces: &HashMap<u64, &Value>, deflection: f64) -> Result<f64, String> {
    let triangles = mesh_triangles(mesh)?;
    let mut worst: f64 = 0.;
    let ranges = mesh["faces"].as_array().filter(|a| !a.is_empty());
    for (index, range) in ranges.into_iter().flatten().enumerate() {
        let (start, count) = (range[0].as_u64().unwrap_or(0) as usize, range[1].as_u64().unwrap_or(0) as usize);
        if count == 0 {
            continue;
        }
        let face = faces.get(&(index as u64));
        let kind = face.and_then(|x| x["kind"].as_str());
        if kind == Some("plane") {
            continue;
        }
        let complete = face.is_some_and(|x| ["direction", "origin", "radius"].iter().all(|k| x.get(*k).is_some()));
        if kind == Some("cylinder") && complete {
            worst = worst.max(cylinder_error(&triangles[start..start + count], face.unwrap())?);
        } else if kind == Some("cone") && ["direction", "origin"].iter().all(|k| face.is_some_and(|x| x.get(*k).is_some())) {
            worst = worst.max(cone_error(&triangles[start..start + count], face.unwrap())?.unwrap_or(deflection));
        } else if kind == Some("torus") && complete {
            worst = worst.max(torus_error(&triangles[start..start + count], face.unwrap())?.unwrap_or(deflection));
        } else {
            worst = worst.max(deflection);
        }
    }
    if ranges.is_none() {
        worst = deflection;
    }
    Ok(worst + 1e-4 * 3f64.sqrt())
}

struct Entry {
    setup: &'static str,
    axis: V3,
    mouth: V3,
    reversed: bool,
    steps: Vec<(f64, f64)>,
}

/// The enlargements between a hole's opening and its bore, widest (outermost) first.
fn steps_of(record: &Value) -> Result<Vec<(f64, f64)>, String> {
    let mut steps: Vec<&Value> = record.get("outer_steps").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    for key in ["cbore", "spotface"] {
        if truthy(record.get(key)) {
            steps.push(&record[key]);
        }
    }
    let mut out = steps.iter().map(|s| Ok((f(s, "diameter")?, f(s, "depth")?))).collect::<Result<Vec<_>, String>>()?;
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(out)
}

struct Hole {
    feature: String,
    center: Value,
    top_z: f64,
    bore_top_z: f64,
    bottom_z: f64,
    diameter: f64,
    bottom: String,
    exit_clear: Option<Option<f64>>,
    enlargements: Vec<(f64, f64, f64)>,
    csink: Value,
    thread: Option<Value>,
}

/// An internal thread asked for on a hole.
#[derive(Clone)]
struct Thread {
    spec: String,
    major: f64,
    pitch: f64,
    full_depth: Option<f64>,
}

/// (major diameter, pitch) of a metric thread spec (`M5`, `M10x1.25`).
fn thread_size(spec: &str) -> Option<(f64, f64)> {
    let (size, fine) = spec.split_once('x').map_or((spec, None), |(a, b)| (a, Some(b)));
    let major: f64 = size.strip_prefix('M')?.parse().ok()?;
    let pitch = match fine {
        Some(text) => text.parse().ok()?,
        None => crate::reference::coarse_pitch(size)?,
    };
    (major > 0. && pitch > 0. && pitch < major).then_some((major, pitch))
}

/// The internal threads asked for, by hole feature id: from Specify's answers
/// (`hole.function:<hole or pattern>` = `tapped:M5` or `tapped:M5@5.5`). Threads another tool wrote,
/// which state no size, and answers that cannot be used become warnings.
fn threads(analysis: &Value, trace: &mut Trace) -> HashMap<String, Thread> {
    let features: Vec<&Value> = analysis["features"].as_array().into_iter().flatten().collect();
    let holes: Vec<&&Value> = features.iter().filter(|f| f["family"] == "holes").collect();
    let faces_of = |f: &Value| -> Vec<u64> { f["faces"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect() };
    let mut found = HashMap::new();
    for (key, value) in analysis["resume"]["answers"].as_object().into_iter().flatten() {
        let (Some(target), Some(answer)) = (key.strip_prefix("hole.function:"), value.as_str().and_then(|v| v.strip_prefix("tapped:"))) else {
            continue;
        };
        let (spec, full) = answer.split_once('@').map_or((answer, None), |(a, b)| (a, Some(b)));
        let Some((major, pitch)) = thread_size(spec) else {
            trace.warn("requirement.thread.unknown_size", target, format!("{target}: tapped {spec} is not a metric size this bridge knows; not threaded"), json!({"features": [target]}));
            continue;
        };
        let full_depth = match full.filter(|text| !text.is_empty()).map(str::parse::<f64>) {
            None => None,
            Some(Ok(depth)) => Some(depth),
            Some(Err(_)) => {
                trace.warn("requirement.thread.unreadable_depth", target, format!("{target}: tapped '{}' has an unreadable depth; not threaded", value.as_str().unwrap_or("")), json!({"features": [target]}));
                continue;
            }
        };
        let Some(feature) = features.iter().find(|f| f["id"].as_str() == Some(target)) else {
            trace.warn("requirement.thread.no_feature", target, format!("{target}: tapped {spec}, but the part has no such feature now; not threaded"), json!({"features": [target]}));
            continue;
        };
        let thread = Thread { spec: spec.to_string(), major, pitch, full_depth };
        if feature["family"] == "holes" {
            found.insert(target.to_string(), thread);
        } else {
            // A hole pattern or set: the holes whose faces it is made of.
            let own = faces_of(feature);
            for hole in &holes {
                let faces = faces_of(hole);
                if !faces.is_empty() && faces.iter().all(|x| own.contains(x)) {
                    found.insert(hole["id"].as_str().unwrap_or("").to_string(), thread.clone());
                }
            }
        }
    }
    for item in analysis["existing"].as_array().into_iter().flatten() {
        if item["type"] != "internal thread" {
            continue;
        }
        let on = faces_of(item);
        for hole in &holes {
            let id = hole["id"].as_str().unwrap_or("");
            if !found.contains_key(id) && faces_of(hole).iter().any(|x| on.contains(x)) {
                trace.warn("requirement.thread.no_size", id, format!("{id}: the file marks an internal thread here but states no size; not threaded (answer it in Specify to thread it)"), json!({"features": [id]}));
            }
        }
    }
    found
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

/// A hole as setup-frame geometry, or why it is skipped.
/// What became of a hole: its setup, the hole, its warnings (rule, text) and the setups it lies
/// along that were rejected because part material stands above its opening.
type Placed = (&'static str, Hole, Vec<(&'static str, String)>, Vec<&'static str>);

fn hole_geometry(feature: &Value, cx: &Context, thread: Option<&Thread>) -> Result<Result<Placed, (&'static str, String)>, String> {
    let record = &feature["record"];
    let id = feature["id"].as_str().unwrap_or("").to_string();
    let axis0 = unit(v3(&record["axis"])?)?;
    let mouth0 = v3(&record["location"])?;
    let depth = f(record, "depth")?;
    let diameter = f(record, "diameter")?;
    let plain = !["cbore", "spotface", "csink", "outer_steps"].iter().any(|k| truthy(record.get(*k)));
    let mut entries = vec![(axis0, mouth0, false)];
    if record["bottom"].as_str() == Some("through") && plain && !truthy(record.get("exit_obstructions")) {
        entries.push((scale(axis0, -1.), add(mouth0, scale(axis0, depth)), true));
    }
    let mut options: Vec<Entry> = vec![];
    let mut blocked: Vec<&str> = vec![];
    for (axis, mouth, reversed) in entries {
        let Some(setup) = setup_for(axis)? else { continue };
        let steps = if reversed { vec![] } else { steps_of(record)? };
        let widest = steps.iter().map(|s| s.0).fold(diameter, f64::max);
        if let Some(mesh) = cx.triangles {
            let frame = &cx.frames[setup];
            let at = frame.point(mouth);
            if blocked_above(frame.triangles(mesh), (at[0], at[1]), at[2], widest / 2.) {
                blocked.push(setup);
                continue;
            }
        }
        options.push(Entry { setup, axis, mouth, reversed, steps });
    }
    if options.is_empty() {
        if !blocked.is_empty() {
            return Ok(Err(("setup.hole.blocked_above", format!(
                "part material above its opening from {}: a three-axis tool cannot reach it",
                blocked.join(", ")
            ))));
        }
        let axis = v3(&record["axis"])?;
        return Ok(Err(("setup.hole.oblique", format!(
            "oblique axis [{}, {}, {}]: needs a setup this bridge does not make",
            show(r(axis[0])), show(r(axis[1])), show(r(axis[2]))
        ))));
    }
    let rank = |name: &str| cx.preferred.iter().position(|p| *p == name).unwrap_or(usize::MAX);
    let mut best = 0;
    for (i, o) in options.iter().enumerate() {
        if rank(o.setup) < rank(options[best].setup) {
            best = i;
        }
    }
    let chosen = &options[best];
    let name = chosen.setup;
    let frame = &cx.frames[name];

    // `location` is the axis point at the opening; `diameter`/`depth` describe the narrowest bore,
    // `depth` from the top of that bore to the deep end.
    let mut enlargements = vec![];
    let mut at = chosen.mouth;
    for (step_d, step_depth) in &chosen.steps {
        let floor = add(at, scale(chosen.axis, *step_depth));
        enlargements.push((r(*step_d), r(frame.point(at)[2]), r(frame.point(floor)[2])));
        at = floor;
    }
    let bore_top = at;
    let bottom = frame.point(add(bore_top, scale(chosen.axis, depth)));
    let centre = frame.point(chosen.mouth);
    let bottom_kind = record.get("bottom").and_then(Value::as_str).unwrap_or("unknown").to_string();
    let mut hole = Hole {
        feature: id.clone(),
        center: json!([r(centre[0]), r(centre[1])]),
        top_z: r(centre[2]),
        bore_top_z: r(frame.point(bore_top)[2]),
        bottom_z: r(bottom[2]),
        diameter: r(diameter),
        bottom: bottom_kind.clone(),
        exit_clear: None,
        enlargements,
        csink: Value::Null,
        thread: None,
    };
    let mut warnings = vec![];
    if bottom_kind == "through" {
        let part_bottom = cx.part_faces[name];
        let on_face = (bottom[2] - part_bottom).abs() <= LEVEL_EPS;
        let clear = match cx.triangles {
            Some(mesh) => clear_below(frame.triangles(mesh), (bottom[0], bottom[1]), bottom[2], diameter / 2.),
            None => if on_face { None } else { Some(0.) },
        };
        hole.exit_clear = Some(clear.map(r));
        if !on_face {
            warnings.push(("process.hole.short_of_far_face", format!(
                "{id}: the through hole ends {} mm short of the part's far face in the {name} setup; \
                 anything between (a counterbore or chamfer at the far end, a shoulder) is not planned",
                show(r(bottom[2] - part_bottom))
            )));
        }
    }
    if let Some(thread) = thread {
        // The thread runs down the bore from its top: all of it for a through hole, else the
        // minimum full thread asked for (specify-core's default when none was).
        let (top, mut end) = (hole.bore_top_z, hole.bottom_z);
        let length = top - end;
        let mut fits = true;
        if hole.bottom != "through" {
            let full = thread.full_depth.filter(|d| *d != 0.).unwrap_or_else(|| crate::reference::full_thread(depth, thread.pitch));
            if full > length + 1e-9 {
                warnings.push(("requirement.thread.too_deep", format!("{id}: {} thread {} mm deep is deeper than the hole ({} mm); not threaded", thread.spec, show(full), show(r(length)))));
                fits = false;
            } else {
                end = top - full;
            }
        }
        if fits {
            hole.thread = Some(json!({"spec": thread.spec, "major": r(thread.major), "pitch": thread.pitch, "top_z": r(top), "bottom_z": r(end)}));
        }
    }
    hole.csink = if !chosen.reversed && truthy(record.get("csink")) { record["csink"].clone() } else { Value::Null };
    Ok(Ok((name, hole, warnings, blocked)))
}

/// Holes of one diameter, depth and ending as one operation over their centres, in the order of
/// those values.
fn hole_operations(holes: &[Hole]) -> Vec<Value> {
    struct Group<'a> {
        members: Vec<&'a Hole>,
    }
    let same = |a: &Hole, b: &Hole| {
        a.diameter == b.diameter && a.top_z == b.top_z && a.bore_top_z == b.bore_top_z && a.bottom_z == b.bottom_z
            && a.bottom == b.bottom && a.exit_clear.flatten() == b.exit_clear.flatten()
    };
    let mut groups: Vec<Group> = vec![];
    for h in holes {
        match groups.iter_mut().find(|g| same(g.members[0], h)) {
            Some(g) => g.members.push(h),
            None => groups.push(Group { members: vec![h] }),
        }
    }
    let clear_key = |h: &Hole| h.exit_clear.flatten().unwrap_or(-1e18);
    groups.sort_by(|a, b| {
        let (x, y) = (a.members[0], b.members[0]);
        x.diameter.total_cmp(&y.diameter)
            .then(x.top_z.total_cmp(&y.top_z))
            .then(x.bore_top_z.total_cmp(&y.bore_top_z))
            .then(x.bottom_z.total_cmp(&y.bottom_z))
            .then(x.bottom.cmp(&y.bottom))
            .then(clear_key(x).total_cmp(&clear_key(y)))
    });
    groups
        .iter()
        .map(|g| {
            let h = g.members[0];
            let mut op = json!({
                "kind": "holes", "diameter": h.diameter, "top_z": h.top_z, "bore_top_z": h.bore_top_z,
                "bottom_z": h.bottom_z, "bottom": h.bottom,
                "centers": g.members.iter().map(|m| m.center.clone()).collect::<Vec<_>>(),
                "features": g.members.iter().map(|m| m.feature.clone()).collect::<Vec<_>>(),
            });
            if g.members.iter().any(|h| !h.csink.is_null()) {
                op["countersinks"] = json!(g.members.iter().map(|h| h.csink.clone()).collect::<Vec<_>>());
            }
            if h.bottom == "through" {
                op["exit_clear_mm"] = json!(h.exit_clear.flatten());
            }
            op
        })
        .collect()
}

fn counterbore_operations(holes: &[Hole]) -> Vec<Value> {
    let mut groups: Vec<((f64, f64, f64), Vec<Value>, Vec<String>)> = vec![];
    for h in holes {
        for e in &h.enlargements {
            match groups.iter_mut().find(|g| g.0 == *e) {
                Some(g) => {
                    g.1.push(h.center.clone());
                    g.2.push(h.feature.clone());
                }
                None => groups.push((*e, vec![h.center.clone()], vec![h.feature.clone()])),
            }
        }
    }
    groups.sort_by(|a, b| a.0.0.total_cmp(&b.0.0).then(a.0.1.total_cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2)));
    groups
        .into_iter()
        .map(|(e, centers, features)| {
            json!({"kind": "counterbore", "diameter": e.0, "top_z": e.1, "floor_z": e.2, "centers": centers, "features": features})
        })
        .collect()
}

fn thread_operations(holes: &[Hole]) -> Vec<Value> {
    let key = |h: &Hole, t: &Value| (t["spec"].as_str().unwrap_or("").to_string(), t["major"].as_f64().unwrap_or(0.), t["pitch"].as_f64().unwrap_or(0.), h.diameter, t["top_z"].as_f64().unwrap_or(0.), t["bottom_z"].as_f64().unwrap_or(0.), h.bottom == "through", h.exit_clear.flatten());
    let mut groups: Vec<(_, Vec<&Hole>)> = vec![];
    for h in holes {
        let Some(t) = &h.thread else { continue };
        let k = key(h, t);
        match groups.iter_mut().find(|g| g.0 == k) {
            Some(g) => g.1.push(h),
            None => groups.push((k, vec![h])),
        }
    }
    groups.sort_by(|(a, _), (b, _)| {
        a.0.cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
            .then(a.3.total_cmp(&b.3))
            .then(a.4.total_cmp(&b.4))
            .then(a.5.total_cmp(&b.5))
            .then(a.6.cmp(&b.6))
            .then(a.7.unwrap_or(f64::INFINITY).total_cmp(&b.7.unwrap_or(f64::INFINITY)))
    });
    groups
        .into_iter()
        .map(|(k, members)| {
            let mut op = json!({
                "kind": "thread", "spec": k.0, "major": k.1, "pitch": k.2, "start_diameter": k.3,
                "top_z": k.4, "bottom_z": k.5, "through": k.6,
                "centers": members.iter().map(|m| m.center.clone()).collect::<Vec<_>>(),
                "features": members.iter().map(|m| m.feature.clone()).collect::<Vec<_>>(),
            });
            // Preserve unrestricted legacy plans; only a measured obstruction adds a bound.
            if k.6 && let Some(clear) = k.7 { op["exit_clear_mm"] = json!(clear); }
            op
        })
        .collect()
}

/// A closed slot with round ends as an operation of its own, cut to size: which setup, and the
/// operation. None if it is not that shape or no setup can reach it; roughing is then all it gets.
fn slot_operation(feature: &Value, cx: &Context) -> Result<Option<(&'static str, Value)>, String> {
    let record = &feature["record"];
    let index = |key: &str| ["x", "y", "z"].iter().position(|a| record[key].as_str() == Some(a));
    let (Some(w), Some(l), Some(triangles)) = (index("width_axis"), index("long_axis"), cx.triangles) else {
        return Ok(None);
    };
    if w == l {
        return Ok(None);
    }
    let depth_axis = 3 - w - l;
    let width = f(record, "width")?;
    let radius = width / 2.;
    let round_ends = record["end_radius"].as_f64().is_some_and(|e| (e - radius).abs() <= LEVEL_EPS);
    let (lo, hi) = (f(record, "lo")?, f(record, "hi")?);
    if !round_ends || hi - lo <= width + LEVEL_EPS {
        return Ok(None);
    }
    let (w_center, d_lo, d_hi) = (f(record, "w_center")?, f(record, "d_lo")?, f(record, "d_hi")?);
    let model = |along: f64, down: f64| {
        let mut p = [0.; 3];
        p[w] = w_center;
        p[l] = along;
        p[depth_axis] = down;
        p
    };
    for name in cx.preferred {
        if rows_of(name)[2][depth_axis].abs() < 0.99 {
            continue;
        }
        let frame = &cx.frames[name];
        let mesh = frame.triangles(triangles);
        let (a, b) = (frame.point(model(lo + radius, d_lo)), frame.point(model(hi - radius, d_lo)));
        let wall_top = frame.point(model(lo, d_lo))[2].max(frame.point(model(lo, d_hi))[2]);
        // The footprint, as discs along the centre line.
        let count = (((hi - lo - width) / (radius / 4.)).ceil() as usize).max(1);
        let centres: Vec<(f64, f64)> = (0..=count)
            .map(|i| {
                let t = i as f64 / count as f64;
                (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
            })
            .collect();
        // Well inside the walls: the mesh's flat facets cut inside their curved ends.
        let inner = radius * 0.8;
        if centres.iter().any(|c| blocked_above(mesh, *c, wall_top, inner)) {
            continue;
        }
        let gap = centres.iter().filter_map(|c| clear_below(mesh, *c, wall_top, inner)).reduce(f64::min);
        let part = cx.part_faces[name];
        let top = bounds(&mesh.iter().flatten().copied().collect::<Vec<_>>()).1[2];
        // With nothing under it, a slot ends where the part around it does: past any edge round.
        let exit = centres.iter().flat_map(|c| mesh.iter().filter(|t| disk_meets(*c, radius + 1., t))).flatten().map(|p| p[2]).fold(wall_top, f64::min).max(part);
        let floor = gap.map_or(exit, |g| wall_top - g);
        let features = cx.absorbed(feature);
        return Ok(Some((name, json!({
            "kind": "slot", "width": r(width), "length": r(hi - lo),
            "a": [r(a[0]), r(a[1])], "b": [r(b[0]), r(b[1])],
            "top_z": r(top), "floor_z": r(floor), "bottom": if gap.is_some() { "blind" } else { "through" },
            "features": features,
        }))));
    }
    Ok(None)
}

/// One piece of a slice through the mesh: a segment, and the mesh face it lies on.
type Cut = ((f64, f64), (f64, f64), usize);

/// Where the mesh crosses the plane at `z`.
fn cuts_at(triangles: &[Tri], ranges: &[Value], z: f64) -> Option<Vec<Cut>> {
    let mut cuts: Vec<Cut> = vec![];
    for (face, range) in ranges.iter().enumerate() {
        let (start, count) = (range[0].as_u64()? as usize, range[1].as_u64()? as usize);
        for t in triangles.get(start..start + count)? {
            let mut points = vec![];
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                if (a[2] < z) != (b[2] < z) {
                    let u = (z - a[2]) / (b[2] - a[2]);
                    points.push((a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u));
                }
            }
            if let [p, q] = points[..]
                && (p.0 - q.0).hypot(p.1 - q.1) > 1e-7
            {
                cuts.push((p, q, face));
            }
        }
    }
    Some(cuts)
}

/// The cuts at `z` as closed loops, or None if they do not close. Faces are meshed apart, so ends
/// that meet may differ by a little.
fn slice(triangles: &[Tri], ranges: &[Value], z: f64) -> Option<Vec<Vec<Cut>>> {
    const MEET: f64 = 2e-3;
    let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1) <= MEET;
    let cuts = cuts_at(triangles, ranges, z)?;
    let mut used = vec![false; cuts.len()];
    let mut loops = vec![];
    for first in 0..cuts.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let mut chain = vec![cuts[first]];
        loop {
            let end = chain.last()?.1;
            if chain.len() > 2 && close(end, chain[0].0) {
                break;
            }
            let next = (0..cuts.len()).find(|i| !used[*i] && (close(cuts[*i].0, end) || close(cuts[*i].1, end)));
            let Some(next) = next else {
                if close(end, chain[0].0) {
                    break;
                }
                return None;
            };
            used[next] = true;
            let c = cuts[next];
            chain.push(if close(c.0, end) { c } else { (c.1, c.0, c.2) });
        }
        loops.push(chain);
    }
    Some(loops)
}

/// Twice the area enclosed, positive when counter-clockwise.
fn area(chain: &[Cut]) -> f64 {
    chain.iter().map(|c| c.0.0 * c.1.1 - c.1.0 * c.0.1).sum()
}

fn reversed(chain: &[Cut]) -> Vec<Cut> {
    chain.iter().rev().map(|c| (c.1, c.0, c.2)).collect()
}

fn contains(chain: &[Cut], p: (f64, f64)) -> bool {
    chain.iter().filter(|c| (c.0.1 > p.1) != (c.1.1 > p.1) && p.0 < c.0.0 + (p.1 - c.0.1) / (c.1.1 - c.0.1) * (c.1.0 - c.0.0)).count() % 2 == 1
}

/// A loop as camlib's boundary, with its true lines and arcs: [x, y, bulge to the next vertex].
/// None if a wall is not an upright flat or a cylinder along the tool.
fn boundary(chain: &[Cut], faces: &HashMap<u64, &Value>, frame: &Frame, rows: &Rows) -> Result<Option<Vec<[f64; 3]>>, String> {
    let n = chain.len();
    let start = (0..n).find(|i| chain[*i].2 != chain[(i + n - 1) % n].2).unwrap_or(0);
    let mut runs: Vec<Vec<Cut>> = vec![];
    for i in 0..n {
        let c = chain[(start + i) % n];
        match runs.last_mut() {
            Some(run) if run[0].2 == c.2 => run.push(c),
            _ => runs.push(vec![c]),
        }
    }
    let mut vertices = vec![];
    for run in &runs {
        let Some(face) = faces.get(&(run[0].2 as u64)) else { return Ok(None) };
        match face["kind"].as_str() {
            Some("plane") => {
                if rotate(rows, v3(&face["direction"])?)[2].abs() > 1e-6 {
                    return Ok(None);
                }
                vertices.push([r(run[0].0.0), r(run[0].0.1), 0.]);
            }
            Some("cylinder") => {
                if rotate(rows, v3(&face["direction"])?)[2].abs() < 0.99 {
                    return Ok(None);
                }
                let centre = frame.point(v3(&face["origin"])?);
                let radius = f(face, "radius")?;
                let angle = |p: (f64, f64)| (p.1 - centre[1]).atan2(p.0 - centre[0]);
                let mut sweep = 0.;
                for c in run {
                    let mut step = angle(c.1) - angle(c.0);
                    if step > std::f64::consts::PI {
                        step -= std::f64::consts::TAU;
                    } else if step < -std::f64::consts::PI {
                        step += std::f64::consts::TAU;
                    }
                    sweep += step;
                }
                // A bulge is at most a half turn; more is written as equal arcs.
                let arcs = (sweep.abs() / std::f64::consts::PI - 1e-6).ceil().max(1.);
                let from = angle(run[0].0);
                for k in 0..arcs as usize {
                    let at = from + sweep * k as f64 / arcs;
                    vertices.push([r(centre[0] + radius * at.cos()), r(centre[1] + radius * at.sin()), (sweep / arcs / 4.).tan().clamp(-1., 1.)]);
                }
            }
            _ => return Ok(None),
        }
    }
    Ok(Some(vertices))
}

/// How far from this outline the cutter meets the part on its way down to `floor`: from the outline
/// out to a cutter's width must be empty. None if nothing is outside it.
///
/// This looks at every triangle above the floor rather than at sections, which can miss what lies
/// between them. Any material outside the outline has a face above it within these heights, and
/// even a face that encloses the whole outline counts: a contour goes through it. A face within
/// `own` of the outline is the wall itself (a curved wall's facets are not all its outline's chords,
/// so that is at least the mesh's error), and one inside it is what the wall encloses.
fn room_beside(triangles: &[Tri], outline: &[Cut], floor: f64, own: f64) -> Option<f64> {
    const ABOVE_FLOOR: f64 = 1e-3;
    let distance = |p: (f64, f64)| outline.iter().map(|c| segment_distance(p, c.0, c.1)).fold(f64::MAX, f64::min);
    let outside = |p: (f64, f64)| !contains(outline, p) && distance(p) > own;
    let mut room: Option<f64> = None;
    for triangle in triangles {
        let corners = clip_above(triangle, floor + ABOVE_FLOOR);
        if corners.len() < 3 {
            continue;
        }
        let edges = || (0..corners.len()).map(|i| (corners[i], corners[(i + 1) % corners.len()]));
        let middle = (corners.iter().map(|c| c.0).sum::<f64>() / corners.len() as f64, corners.iter().map(|c| c.1).sum::<f64>() / corners.len() as f64);
        let turn = |a: (f64, f64), b: (f64, f64), p: (f64, f64)| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        // An outline's corner well inside the face means the face spans a notch, or the outline.
        let holds = |p: (f64, f64)| {
            let sides: Vec<f64> = edges().map(|(a, b)| turn(a, b, p)).collect();
            (sides.iter().all(|s| *s > 0.) || sides.iter().all(|s| *s < 0.)) && edges().all(|(a, b)| segment_distance(p, a, b) > own)
        };
        let spans = outline.iter().any(|c| holds(c.0));
        let beyond = spans || outside(middle) || edges().any(|(a, b)| outside(a) || outside(((a.0 + b.0) / 2., (a.1 + b.1) / 2.)));
        if !beyond {
            continue;
        }
        let gap = if spans {
            0.
        } else {
            edges().flat_map(|(a, b)| outline.iter().map(move |c| segments_apart((a, b), (c.0, c.1)))).fold(f64::MAX, f64::min)
        };
        room = Some(room.map_or(gap, |r: f64| r.min(gap)));
    }
    room
}

/// A triangle's part above a height, seen from above.
fn clip_above(triangle: &Tri, z: f64) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for i in 0..3 {
        let (a, b) = (triangle[i], triangle[(i + 1) % 3]);
        if a[2] >= z {
            out.push((a[0], a[1]));
        }
        if (a[2] >= z) != (b[2] >= z) {
            let t = (z - a[2]) / (b[2] - a[2]);
            out.push((a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t));
        }
    }
    out
}

/// The distance between two segments, 0 if they cross.
fn segments_apart(p: ((f64, f64), (f64, f64)), q: ((f64, f64), (f64, f64))) -> f64 {
    let turn = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if (turn(p.0, p.1, q.0) > 0.) != (turn(p.0, p.1, q.1) > 0.) && (turn(q.0, q.1, p.0) > 0.) != (turn(q.0, q.1, p.1) > 0.) {
        return 0.;
    }
    [segment_distance(p.0, q.0, q.1), segment_distance(p.1, q.0, q.1), segment_distance(q.0, p.0, p.1), segment_distance(q.1, p.0, p.1)].into_iter().fold(f64::MAX, f64::min)
}

/// Whether anything of the part is above a height within this box, seen from above. What only
/// touches its edge does not count: that is the walls of the space being cut.
fn occupied_above(mesh: &[Tri], lo: (f64, f64), hi: (f64, f64), z: f64) -> bool {
    const EDGE: f64 = 0.01;
    let (lo, hi) = ((lo.0 + EDGE, lo.1 + EDGE), (hi.0 - EDGE, hi.1 - EDGE));
    if lo.0 >= hi.0 || lo.1 >= hi.1 {
        return false;
    }
    let corners = [lo, (hi.0, lo.1), hi, (lo.0, hi.1)];
    occupied_polygon(mesh, &corners, z)
}

fn occupied_polygon(mesh: &[Tri], corners: &[(f64, f64)], z: f64) -> bool {
    let inside = |p: (f64, f64)| (0..corners.len()).all(|i| {
        let (a,b) = (corners[i], corners[(i+1)%corners.len()]);
        (b.0-a.0)*(p.1-a.1) - (b.1-a.1)*(p.0-a.0) > 0.
    });
    mesh.iter().any(|triangle| {
        let shape = clip_above(triangle, z);
        if shape.len() < 3 {
            return false;
        }
        let edge = |i: usize| (shape[i], shape[(i + 1) % shape.len()]);
        let turn = |a: (f64, f64), b: (f64, f64), p: (f64, f64)| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        let holds = |p: (f64, f64)| {
            let sides: Vec<f64> = (0..shape.len()).map(|i| turn(edge(i).0, edge(i).1, p)).collect();
            sides.iter().all(|s| *s > 0.) || sides.iter().all(|s| *s < 0.)
        };
        shape.iter().copied().any(inside)
            || corners.iter().copied().any(holds)
            || (0..shape.len()).any(|i| (0..corners.len()).any(|k| segments_apart(edge(i), (corners[k], corners[(k + 1) % corners.len()])) == 0.))
    })
}

/// Counter-clockwise camlib vertices; the bulge belongs to the following arc.
fn rounded_rectangle(lo: (f64, f64), hi: (f64, f64), radius: f64) -> Vec<Value> {
    let (x, y, a, b, t) = (lo.0, lo.1, hi.0, hi.1, radius);
    if t <= 0. {
        return vec![json!([r(x), r(y), 0.]), json!([r(a), r(y), 0.]), json!([r(a), r(b), 0.]), json!([r(x), r(b), 0.])];
    }
    let q = (std::f64::consts::PI / 8.).tan();
    let mut vertices: Vec<(f64, f64, f64)> = vec![];
    for p in [(x+t,y,0.), (a-t,y,q), (a,y+t,0.), (a,b-t,q), (a-t,b,0.), (x+t,b,q), (x,b-t,0.), (x,y+t,q)] {
        // At a semicircular end the straight tangent edge has zero length.
        if let Some(last) = vertices.last_mut().filter(|v| (v.0-p.0).hypot(v.1-p.1) < 1e-7) {
            last.2 = p.2;
        } else {
            vertices.push(p);
        }
    }
    vertices.into_iter().map(|(x, y, b)| json!([r(x), r(y), b])).collect()
}

/// An inscribed convex polygon leaves the mesh's faceted curved walls outside the check.
/// Like the wall slice checks, allow 0.05 mm at the boundary: CTC 01's r10 corners
/// have chords about 0.03 mm inside the exact arc (including the adjacent rim mesh).
fn occupied_rounded(mesh: &[Tri], lo: (f64, f64), hi: (f64, f64), z: f64, radius: f64) -> bool {
    if radius <= 0. { return occupied_above(mesh, lo, hi, z); }
    let mut corners = vec![];
    for (x,y,angle) in [(hi.0-radius,lo.1+radius,-90.),(hi.0-radius,hi.1-radius,0.),(lo.0+radius,hi.1-radius,90.),(lo.0+radius,lo.1+radius,180.)] {
        for i in 0..=32 {
            let a = (angle + 90. * i as f64 / 32.).to_radians();
            corners.push((x+(radius-0.05).max(0.)*a.cos(),y+(radius-0.05).max(0.)*a.sin()));
        }
    }
    corners.dedup_by(|a, b| (a.0-b.0).hypot(a.1-b.1) < 1e-7);
    if corners.first().zip(corners.last()).is_some_and(|(a, b)| (a.0-b.0).hypot(a.1-b.1) < 1e-7) {
        corners.pop();
    }
    occupied_polygon(mesh, &corners, z)
}

/// A rectangular recess, optionally with rounded corners, cut with an end mill that fits.
/// The mesh must agree that its interior is empty. An open side is cut past the part;
/// settle records any corner radius the cutter leaves beyond the model's radius.
fn notch_operation(feature: &Value, cx: &Context) -> Result<Option<(&'static str, Value)>, String> {
    notch_with_corner_pairs(feature, cx, true)
}

fn notch_with_corner_pairs(feature: &Value, cx: &Context, recover_open: bool) -> Result<Option<(&'static str, Value)>, String> {
    let mut normalized = feature["record"].clone();
    if feature["family"] == "rectangular_blind_slots" {
        let axis = |key: &str| ["x", "y", "z"].iter().position(|a| normalized[key].as_str() == Some(a));
        let (Some(w), Some(l), Some(d)) = (axis("width_axis"), axis("axis"), axis("depth_axis")) else { return Ok(None) };
        if w == l || w == d || l == d { return Ok(None); }
        let at = v3(&normalized["at"])?;
        normalized["long_axis"] = normalized["depth_axis"].clone();
        normalized["w_center"] = json!(at[w]);
        normalized["lo"] = json!(at[d] - f(&normalized, "depth")? / 2.);
        normalized["hi"] = json!(at[d] + f(&normalized, "depth")? / 2.);
        normalized["d_lo"] = json!(at[l] - f(&normalized, "length")? / 2.);
        normalized["d_hi"] = json!(at[l] + f(&normalized, "length")? / 2.);
    }
    let record = &normalized;
    let index = |key: &str| ["x", "y", "z"].iter().position(|a| record[key].as_str() == Some(a));
    let (Some(w), Some(l), Some(triangles)) = (index("width_axis"), index("long_axis"), cx.triangles) else {
        return Ok(None);
    };
    if w == l {
        return Ok(None);
    }
    let depth_axis = 3 - w - l;
    let width = f(record, "width")?;
    let (w_center, along, across) = (f(record, "w_center")?, (f(record, "lo")?, f(record, "hi")?), (f(record, "d_lo")?, f(record, "d_hi")?));
    // Which way the feature opens, along the axis across it: only a pocket has one.
    let open = record["open_sign"].as_f64().unwrap_or(0.);
    for name in cx.preferred {
        let rows = rows_of(name);
        let Some(tool_axis) = [l, depth_axis].into_iter().find(|a| rows[2][*a].abs() >= 0.99) else { continue };
        // This family's record states the depth axis and opening direction explicitly.
        if feature["family"] == "rectangular_blind_slots"
            && (tool_axis != l || rows[2][l] * record["depth_sign"].as_f64().unwrap_or(1.) < 0.99) {
            continue;
        }
        let (depth, mut side, side_axis) = if tool_axis == l { (along, across, depth_axis) } else { (across, along, l) };
        let mut radius = record["corner_radius"].as_f64().unwrap_or(0.).max(0.);
        let mut side_open = if side_axis == depth_axis { open } else { 0. };
        let mut inferred_open = false;
        // Some records report only the straight side-wall interval. Four matching concave
        // quarter-rounds recover a closed recess; a pair recovers its closed end and implies
        // an opening at the other end, which the mesh/run-out check must confirm.
        if radius == 0. && ["slots", "pockets"].contains(&feature["family"].as_str().unwrap_or("")) && side_open == 0. {
            let mut corners = vec![];
            for other in cx.all_features {
                let rec = &other["record"];
                if other["family"] != "fillets" || rec["side"] != "concave" || rec["axis"].as_str() != Some(["x", "y", "z"][tool_axis]) { continue; }
                let (Some(rad), Ok(p)) = (rec["radius"].as_f64(), v3(&rec["at"])) else { continue };
                if rad <= 0. || rad >= width / 2. || (p[tool_axis] - (depth.0 + depth.1) / 2.).abs() > LEVEL_EPS { continue; }
                let width_at = width / 2. - rad * (1. - std::f64::consts::FRAC_1_SQRT_2);
                let near_width = ((p[w] - w_center).abs() - width_at).abs() < 0.002;
                let low = (p[side_axis] - (side.0 - rad * std::f64::consts::FRAC_1_SQRT_2)).abs() < 0.002;
                let high = (p[side_axis] - (side.1 + rad * std::f64::consts::FRAC_1_SQRT_2)).abs() < 0.002;
                if near_width && (low || high) { corners.push((rad, p[w] > w_center, high)); }
            }
            let same_radius = !corners.is_empty() && corners.iter().all(|c| (c.0-corners[0].0).abs() < LEVEL_EPS);
            let pair_at = |end| [false, true].into_iter().all(|w| corners.iter().filter(|c| c.1 == w && c.2 == end).count() == 1);
            if same_radius && corners.len() == 4 && pair_at(false) && pair_at(true) {
                radius = corners[0].0;
                side = (side.0-radius, side.1+radius);
            } else if recover_open && same_radius && corners.len() == 2 && (pair_at(false) || pair_at(true)) {
                inferred_open = true;
                radius = corners[0].0;
                if pair_at(false) {
                    side.0 -= radius;
                    side_open = 1.;
                } else {
                    side.1 += radius;
                    side_open = -1.;
                }
            }
        }
        let run_out = if side_open != 0. { width / 2. + 1. } else { 0. };
        let side_ends = (side.0 - if side_open < 0. { run_out } else { 0. }, side.1 + if side_open > 0. { run_out } else { 0. });
        let model = |width_at: f64, side_at: f64, depth_at: f64| {
            let mut p = [0.; 3];
            p[w] = width_at;
            p[side_axis] = side_at;
            p[tool_axis] = depth_at;
            p
        };
        let frame = &cx.frames[name];
        let (a, b) = (frame.point(model(w_center - width / 2., side_ends.0, depth.0)), frame.point(model(w_center + width / 2., side_ends.1, depth.1)));
        let (lo, hi) = ((a[0].min(b[0]), a[1].min(b[1])), (a[0].max(b[0]), a[1].max(b[1])));
        let (floor, top) = (a[2].min(b[2]), a[2].max(b[2]));
        let mesh = frame.triangles(triangles);
        let invalid_radius = radius > width / 2. + LEVEL_EPS || radius > (side_ends.1 - side_ends.0) / 2. + LEVEL_EPS;
        if invalid_radius || occupied_rounded(mesh, lo, hi, floor + 1e-3, radius) {
            // A corner pair alone cannot prove the opposite end is open. Partial records
            // of irregular recesses can have other walls there. Preserve their previously
            // validated interior cut when the inferred run-out is blocked.
            if inferred_open {
                return notch_with_corner_pairs(feature, cx, false);
            }
            continue;
        }
        let part = cx.part_faces[name];
        let part_top = bounds(&mesh.iter().flatten().copied().collect::<Vec<_>>()).1[2];
        let through = floor <= part + LEVEL_EPS;
        let mut features = cx.absorbed(feature);
        // Only upright concave corner faces inside this recess belong to it. Horizontal floor
        // blends and neighbouring outside rounds must remain independent.
        let mut corner_faces = feature["faces"].as_array().cloned().unwrap_or_default();
        for other in cx.all_features {
            let rec = &other["record"];
            if radius <= 0. || rec["side"] != "concave" || !["fillets", "blends"].contains(&other["family"].as_str().unwrap_or("")) { continue; }
            let axis_ok = rec["axis"].as_str() == Some(["x", "y", "z"][tool_axis])
                || rec["path"]["direction"][tool_axis].as_f64().is_some_and(|v| v.abs() > 0.99);
            let at = rec.get("at").unwrap_or(&rec["path"]["at"]);
            if !axis_ok || (rec["radius"].as_f64().unwrap_or(0.) - radius).abs() > LEVEL_EPS { continue; }
            let Ok(p) = v3(at) else { continue };
            let p = frame.point(p);
            if p[2] >= floor - LEVEL_EPS && p[2] <= top + LEVEL_EPS
                && p[0] > lo.0 && p[0] < hi.0 && p[1] > lo.1 && p[1] < hi.1
                && (p[0] - lo.0).min(hi.0 - p[0]) <= radius + LEVEL_EPS
                && (p[1] - lo.1).min(hi.1 - p[1]) <= radius + LEVEL_EPS {
                features.push(other["id"].as_str().unwrap_or("").to_string());
                corner_faces.extend(other["faces"].as_array().into_iter().flatten().cloned());
            }
        }
        for other in cx.all_features {
            if other["family"] == "section_recesses" {
                let faces = other["faces"].as_array();
                if faces.is_some_and(|fs| !fs.is_empty() && fs.iter().all(|f| corner_faces.contains(f)) && feature["faces"].as_array().is_some_and(|own| own.iter().all(|f| fs.contains(f)))) {
                    features.push(other["id"].as_str().unwrap_or("").to_string());
                }
            }
        }
        return Ok(Some((name, json!({
            "kind": "notch", "width": r(width), "length": r(side.1 - side.0), "open": side_open != 0.,
            "outer": rounded_rectangle(lo, hi, radius), "corner_radius": r(radius), "inside_corners": if side_open != 0. { 2 } else { 4 },
            "top_z": r(part_top), "floor_z": r(if through { part } else { floor }), "wall_top": r(top), "bottom": if through { "through" } else { "blind" },
            "features": features,
        }))));
    }
    Ok(None)
}

/// A V-channel, cut by running a sharp V-bit along it with its tip on the channel's root. The bit
/// must be no wider than the channel: the job picks one. The channel must run out of the part at
/// both ends, as the bit's cone is wider than its tip.
fn v_channel_operation(feature: &Value, cx: &Context) -> Result<Option<(&'static str, Value)>, String> {
    let record = &feature["record"];
    let Some(triangles) = cx.triangles else { return Ok(None) };
    let (axis, opening, at) = (v3(&record["axis"])?, v3(&record["opening"])?, v3(&record["at"])?);
    let (length, width, depth, angle) = (f(record, "length")?, f(record, "width")?, f(record, "depth")?, f(record, "included_angle_degrees")?);
    for name in cx.preferred {
        let rows = rows_of(name);
        if (0..3).map(|k| rows[2][k] * opening[k]).sum::<f64>() < 0.99 {
            continue;
        }
        let frame = &cx.frames[name];
        let end = |by: f64| frame.point(add(at, scale(axis, by)));
        let (a, b, root) = (end(-length / 2.), end(length / 2.), frame.point(at));
        let top = root[2] + depth;
        let run_out = width / 2. + 1.;
        // In this frame the channel lies along X or Y.
        let along_x = (a[0] - b[0]).abs() > (a[1] - b[1]).abs();
        let reach = |p: V3, q: V3| -> ((f64, f64), (f64, f64)) {
            let (half_x, half_y) = if along_x { (0., width / 2.) } else { (width / 2., 0.) };
            let (lo_x, hi_x) = (p[0].min(q[0]) - half_x, p[0].max(q[0]) + half_x);
            let (lo_y, hi_y) = (p[1].min(q[1]) - half_y, p[1].max(q[1]) + half_y);
            ((lo_x, lo_y), (hi_x, hi_y))
        };
        let beyond = |p: V3, q: V3, by: f64| {
            let n = ((p[0] - q[0]).hypot(p[1] - q[1])).max(1e-9);
            [p[0] + (p[0] - q[0]) / n * by, p[1] + (p[1] - q[1]) / n * by, p[2]]
        };
        let (out_a, out_b) = (beyond(a, b, run_out), beyond(b, a, run_out));
        let mesh = frame.triangles(triangles);
        let (ends_a, ends_b) = (reach(a, out_a), reach(b, out_b));
        let whole = reach(a, b);
        if occupied_above(mesh, ends_a.0, ends_a.1, root[2] + 1e-3) || occupied_above(mesh, ends_b.0, ends_b.1, root[2] + 1e-3) || occupied_above(mesh, whole.0, whole.1, top + 1e-3) {
            continue;
        }
        // The bit's body is wider than the channel, above the surface the channel is cut in.
        let path = reach(out_a, out_b);
        let corner = |x: f64, y: f64| (x, y);
        let points = [corner(path.0.0, path.0.1), corner(path.1.0, path.0.1), corner(path.1.0, path.1.1), corner(path.0.0, path.1.1)];
        let outline: Vec<Cut> = (0..4).map(|i| (points[i], points[(i + 1) % 4], 0)).collect();
        let room = room_beside(mesh, &outline, top, 0.01);
        let part_top = bounds(&mesh.iter().flatten().copied().collect::<Vec<_>>()).1[2];
        return Ok(Some((name, json!({
            "kind": "v_channel", "angle": r(angle), "width": r(width), "depth": r(depth), "length": r(length),
            "a": [r(out_a[0]), r(out_a[1])], "b": [r(out_b[0]), r(out_b[1])], "top_z": r(top), "root_z": r(root[2]),
            "on_flat": top < part_top - LEVEL_EPS, "room": room.map(r),
            "features": cx.absorbed(feature),
        }))));
    }
    Ok(None)
}

/// Whether a point is in the part's section: an odd count of the mesh's cuts along +X from it.
fn in_section(p: (f64, f64), cuts: &[Cut]) -> bool {
    let crosses = |c: &&Cut| (c.0.1 > p.1) != (c.1.1 > p.1) && c.0.0 + (p.1 - c.0.1) / (c.1.1 - c.0.1) * (c.1.0 - c.0.0) > p.0;
    cuts.iter().filter(crosses).count() % 2 == 1
}

/// Finishes the outsides of the part's solid outlines, in the roughing setup, at each band between
/// flats: a contour stops at its band's floor, so a raised part's outline is not followed through
/// what it stands on. An outline already finished from a lower band is not repeated, as a contour
/// starts at the top of the stock. The outline is the band's middle, or lower where a side hole
/// breaks that: what tapers in stays clear of the cutter, and what flares out at the foot or
/// overhangs above leaves no room beside it. Returns the operations and each one's outline.
fn wall_operations(triangles: &[Tri], ranges: &[Value], faces: &HashMap<u64, &Value>, frame: &Frame, rows: &Rows, heights: &[f64], own: f64, trace: &mut Trace, setup: &str) -> Result<(Vec<Value>, Vec<Vec<Cut>>), String> {
    const ABOVE: f64 = 0.05;
    const SMALLEST: f64 = 0.5; // mm²: less is a sliver between faces, not an outline.
    let distance = |p: (f64, f64), chain: &[Cut]| chain.iter().map(|c| segment_distance(p, c.0, c.1)).fold(f64::MAX, f64::min);
    let mut finished: Vec<Vec<Cut>> = vec![];
    let mut ops = vec![];
    for (band_at, band) in heights.windows(2).enumerate() {
        let (low, high) = (band[0], band[1]);
        let (mut had_wall, mut unreadable) = (false, false);
        for fraction in [0.5, 0.25, 0.125] {
            let z = low + (high - low) * fraction;
            let (Some(section), Some(cuts)) = (slice(triangles, ranges, z), cuts_at(triangles, ranges, z)) else { continue };
            for chain in &section {
                let size = area(chain).abs();
                let repeat = finished.iter().any(|f| (area(f).abs() - size).abs() <= 0.005 * size && distance(chain[0].0, f) <= 2. * ABOVE);
                if size < SMALLEST || repeat {
                    continue;
                }
                let outline = if area(chain) > 0. { chain.clone() } else { reversed(chain) };
                // Part on its left, nothing on its right: a solid's outside, not a hole's wall.
                let Some(longest) = outline.iter().max_by(|a, b| (a.1.0 - a.0.0).hypot(a.1.1 - a.0.1).total_cmp(&(b.1.0 - b.0.0).hypot(b.1.1 - b.0.1))) else { continue };
                let length = (longest.1.0 - longest.0.0).hypot(longest.1.1 - longest.0.1);
                let (mid, left) = (((longest.0.0 + longest.1.0) / 2., (longest.0.1 + longest.1.1) / 2.), (-(longest.1.1 - longest.0.1) / length * ABOVE, (longest.1.0 - longest.0.0) / length * ABOVE));
                if !in_section((mid.0 + left.0, mid.1 + left.1), &cuts) || in_section((mid.0 - left.0, mid.1 - left.1), &cuts) {
                    continue;
                }
                let Some(vertices) = boundary(&outline, faces, frame, rows)? else {
                    unreadable = true;
                    continue;
                };
                had_wall = true;
                // A contour knows only its outline: the cutter needs room beside it.
                let room = room_beside(triangles, &outline, low, own);
                finished.push(outline);
                ops.push(json!({
                    "kind": "wall_finish", "wall": format!("wall-{}", ops.len() + 1), "outer": vertices, "floor_z": r(low),
                    "on_flat": band_at > 0, "room": room.map(r),
                }));
            }
        }
        if unreadable && !had_wall {
            trace.warn("process.wall_finish.not_upright", setup, format!("an outside wall above Z{} is not upright planes and cylinders, so it keeps its roughing allowance", show(r(low))), json!({}));
        }
    }
    Ok((ops, finished))
}

/// Chamfers the top edges that the model rounds, in the roughing setup: (operations, ids of the
/// rounds they stand for). A 45° chamfer meeting a round at its middle never cuts into the part.
/// A loop qualifies only if its whole top edge has the one radius and its wall ends up at size:
/// the outside (`wall` is the operation that finishes this outline) or a hole or slot's (already
/// cut). An outside chamfer says which wall it needs, as that wall can still be left out.
fn edge_operations(mesh: &Value, triangles: &[Tri], cx: &Context, faces: &HashMap<u64, &Value>, setup: &'static str, part_top: f64, floor: f64, holes: &[Hole], slots: &[Value], wall: &dyn Fn(&[Cut]) -> Option<String>) -> Result<(Vec<Value>, Vec<String>), String> {
    let nothing = (vec![], vec![]);
    let frame = &cx.frames[setup];
    let rows = rows_of(setup);
    let Some(ranges) = mesh["faces"].as_array() else { return Ok(nothing) };
    let mut rounds: Vec<(&str, f64, V3)> = vec![];
    for feature in cx.all_features {
        if feature["family"] != "fillets" || feature["record"]["side"] != "convex" || upright(feature, faces, rows)? {
            continue;
        }
        let radius = f(&feature["record"], "radius")?;
        let Some(face) = feature["faces"][0].as_u64().and_then(|i| faces.get(&i)) else { continue };
        let centre = frame.point(v3(&face["centroid"])?);
        if centre[2] > part_top - radius - LEVEL_EPS {
            rounds.push((feature["id"].as_str().unwrap_or(""), radius, centre));
        }
    }
    let Some(radius) = rounds.first().map(|x| x.1) else { return Ok(nothing) };
    if rounds.iter().any(|x| (x.1 - radius).abs() > LEVEL_EPS) {
        return Ok(nothing);
    }
    let triangles = frame.triangles(triangles);
    // Just below the round, the walls; at its middle, where a rounded edge has moved in.
    let inset = radius * (1. - std::f64::consts::FRAC_1_SQRT_2);
    let (Some(walls), Some(mids)) = (slice(triangles, ranges, part_top - radius - 0.05), cuts_at(triangles, ranges, part_top - inset)) else {
        return Ok(nothing);
    };
    let Some(outer_at) = (0..walls.len()).max_by(|a, b| area(&walls[*a]).abs().total_cmp(&area(&walls[*b]).abs())) else { return Ok(nothing) };
    let rounded = |chain: &[Cut]| {
        chain.iter().all(|c| {
            let p = ((c.0.0 + c.1.0) / 2., (c.0.1 + c.1.1) / 2.);
            let near = mids.iter().map(|m| segment_distance(p, m.0, m.1)).fold(f64::MAX, f64::min);
            (near - inset).abs() <= 0.08
        })
    };
    let outer = if area(&walls[outer_at]) > 0. { walls[outer_at].clone() } else { reversed(&walls[outer_at]) };
    let cut_to_size = |p: (f64, f64)| {
        holes.iter().find(|h| {
            let c = (h.center[0].as_f64().unwrap_or(f64::MAX), h.center[1].as_f64().unwrap_or(f64::MAX));
            (c.0 - p.0).hypot(c.1 - p.1) < h.diameter / 2.
        }).map(|h| h.feature.clone()).or_else(|| slots.iter().filter(|s| s["kind"] == "slot").find(|s| {
            let (a, b) = ((s["a"][0].as_f64().unwrap_or(0.), s["a"][1].as_f64().unwrap_or(0.)), (s["b"][0].as_f64().unwrap_or(0.), s["b"][1].as_f64().unwrap_or(0.)));
            segment_distance(p, a, b) < s["width"].as_f64().unwrap_or(0.) / 2.
        }).and_then(|s| s["features"][0].as_str().map(String::from)))
    };
    let mut needs_openings = vec![];
    let mut chamfered: Vec<Vec<Cut>> = vec![];
    let mut inner: Vec<Value> = vec![];
    for (i, chain) in walls.iter().enumerate() {
        let (count, sum) = (chain.len() as f64, chain.iter().fold((0., 0.), |s, c| (s.0 + c.0.0, s.1 + c.0.1)));
        let middle_of = (sum.0 / count, sum.1 / count);
        if i == outer_at || !contains(&outer, chain[0].0) {
            continue;
        }
        if rounded(chain) && let Some(opening) = cut_to_size(middle_of) {
            let clockwise = if area(chain) < 0. { chain.clone() } else { reversed(chain) };
            if let Some(vertices) = boundary(&clockwise, faces, frame, rows)? {
                inner.push(json!(vertices));
                needs_openings.push(opening);
                chamfered.push(clockwise);
            }
        }
    }
    let Some(outline) = boundary(&outer, faces, frame, rows)? else { return Ok(nothing) };
    // The wall must run down to the floor: on a stepped part this outline is a raised part's.
    let largest = |z: f64| slice(triangles, ranges, z).and_then(|l| l.iter().map(|c| area(c).abs()).max_by(f64::total_cmp));
    let same = |z: f64| largest(z).is_some_and(|a| (a - area(&outer)).abs() <= 0.005 * area(&outer));
    let wall_low = floor + radius + 0.05;
    let needs_wall = wall(&outer).filter(|_| rounded(&outer) && same(wall_low) && same((wall_low + part_top - radius) / 2.));
    let outside = needs_wall.is_some();
    if outside {
        chamfered.push(outer.clone());
    } else if inner.is_empty() {
        return Ok(nothing);
    }
    let ids: Vec<String> = rounds
        .iter()
        .filter(|x| chamfered.iter().flatten().any(|c| segment_distance((x.2[0], x.2[1]), c.0, c.1) <= radius + 0.5))
        .map(|x| x.0.to_string())
        .collect();
    let near_outer: Vec<&String> = ids.iter().filter(|id| rounds.iter().any(|x| x.0 == id.as_str() && outer.iter().any(|c| segment_distance((x.2[0], x.2[1]), c.0, c.1) <= radius + 0.5))).collect();
    let ops = vec![json!({
        "kind": "edge_chamfer", "needs_openings": needs_openings, "needs_wall": needs_wall, "wall_features": if outside { json!(near_outer) } else { json!([]) }, "radius": r(radius), "width": r(radius * (2. - 2f64.sqrt())), "top_z": r(part_top),
        "outer": outline, "holes": inner, "boundaries": match (outside, inner.is_empty()) {
            (true, true) => "outer",
            (true, false) => "all",
            _ => "holes",
        },
        "features": ids,
    })];
    Ok((ops, ids))
}

/// A convex perimeter can continue across removed notches. Keep the wall face on each
/// hull edge so `boundary` restores the exact cylindrical corners rather than mesh chords.
fn convex_perimeter(chain: &[Cut]) -> Option<Vec<Cut>> {
    let mut points: Vec<_> = chain.iter().map(|c| c.0).collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points.dedup_by(|a, b| (a.0 - b.0).hypot(a.1 - b.1) < 1e-6);
    let turn = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let mut hull = vec![];
    for half in [points.clone(), points.iter().rev().copied().collect()] {
        let mut side = vec![];
        for p in half {
            while side.len() >= 2 && turn(side[side.len() - 2], side[side.len() - 1], p) <= 1e-8 {
                side.pop();
            }
            side.push(p);
        }
        side.pop();
        hull.extend(side);
    }
    if hull.len() < 3 {
        return None;
    }
    (0..hull.len())
        .map(|i| {
            let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
            let n = (b.0 - a.0).hypot(b.1 - a.1);
            let face = chain.iter().find(|c| turn(a, b, c.0).abs() / n < 1e-4 && turn(a, b, c.1).abs() / n < 1e-4)?.2;
            Some((a, b, face))
        })
        .collect()
}

/// Check an interrupted convex bevel against the mesh. Upright cylinders use their
/// exact arcs, rather than the inscribed chords that tessellation leaves at corners.
fn convex_bevel_clear(triangles: &[Tri], hull: &[Cut], faces: &HashMap<u64, &Value>, frame: &Frame, top: f64, width: f64) -> Result<bool, String> {
    for c in hull {
        let n = (c.1.0 - c.0.0).hypot(c.1.1 - c.0.1);
        let support = |p: (f64, f64)| ((c.1.0 - c.0.0) * (p.1 - c.0.1) - (c.1.1 - c.0.1) * (p.0 - c.0.0)) / n;
        let circle = match faces.get(&(c.2 as u64)).filter(|f| f["kind"] == "cylinder") {
            Some(face) => Some((frame.point(v3(&face["origin"])?), f(face, "radius")?)),
            None => None,
        };
        for p in triangles.iter().flatten().filter(|p| p[2] >= top - width) {
            let mut inside = support((p[0], p[1]));
            if let Some((centre, radius)) = circle {
                let (a, b, q) = ((c.0.0 - centre[0], c.0.1 - centre[1]), (c.1.0 - centre[0], c.1.1 - centre[1]), (p[0] - centre[0], p[1] - centre[1]));
                if a.0 * q.1 - a.1 * q.0 >= 0. && q.0 * b.1 - q.1 * b.0 >= 0. {
                    inside = radius - q.0.hypot(q.1);
                } else {
                    inside += (radius - support((centre[0], centre[1]))).max(0.);
                }
            }
            if inside + 0.002 < (p[2] - (top - width)).min(width) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Modelled equal-leg 45° top chamfers. A complete loop must have the requested inset
/// at its middle. Interrupted convex outside loops additionally admit only material
/// inside every bevel half-plane, at every mesh vertex above the bevel's foot.
fn modelled_edge_operations(
    mesh: &Value,
    triangles: &[Tri],
    cx: &Context,
    faces: &HashMap<u64, &Value>,
    setup: &'static str,
    openings: (&[Hole], &[Value]),
    wall: &dyn Fn(&[Cut]) -> Option<String>,
) -> Result<(Vec<Value>, Vec<String>), String> {
    let (holes, slots) = openings;
    let frame = &cx.frames[setup];
    let rows = rows_of(setup);
    let Some(ranges) = mesh["faces"].as_array() else {
        return Ok((vec![], vec![]));
    };
    let triangles = frame.triangles(triangles);
    type Edges = Vec<(String, V3)>;
    let mut groups: Vec<(f64, f64, Edges)> = vec![];
    for feature in cx.all_features.iter().filter(|f| matches!(f["family"].as_str(), Some("chamfers" | "oriented_chamfers"))) {
        let rec = &feature["record"];
        let (width, leg2, angle) = (f(rec, "leg1")?, f(rec, "leg2")?, f(rec, "angle")?);
        if width <= LEVEL_EPS || (width - leg2).abs() > LEVEL_EPS || (angle - 45.).abs() > 0.01 {
            continue;
        }
        let mut points = vec![];
        let mut upward = true;
        for id in feature["faces"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
            if let Some(face) = faces.get(&id) {
                if face["kind"] == "plane" {
                    let z = rotate(rows, v3(&face["direction"])?)[2];
                    upward &= (z - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01;
                } else {
                    upward &= face["kind"] == "cone" && rotate(rows, v3(&face["direction"])?)[2].abs() > 0.99;
                }
            }
            if let Some(range) = ranges.get(id as usize) {
                let (start, count) = (range[0].as_u64().unwrap_or(0) as usize, range[1].as_u64().unwrap_or(0) as usize);
                points.extend(triangles[start..start + count].iter().flatten().copied());
            }
        }
        if !upward || points.is_empty() {
            continue;
        }
        let (lo, hi) = bounds(&points);
        if (hi[2] - lo[2] - width).abs() > LEVEL_EPS {
            continue;
        }
        // A full conical mouth's centroid is on its axis, far from the edge.
        // A face vertex associates both full cones and planar strips with their loop.
        let centre = points[0];
        let item = (feature["id"].as_str().unwrap_or("").to_owned(), centre);
        if let Some(g) = groups.iter_mut().find(|g| (g.0 - width).abs() < LEVEL_EPS && (g.1 - hi[2]).abs() < LEVEL_EPS) {
            g.2.push(item);
        } else {
            groups.push((width, hi[2], vec![item]));
        }
    }
    let mut ops = vec![];
    let mut covered = vec![];
    for (width, top, features) in groups {
        let (Some(loops), Some(mids)) = (slice(triangles, ranges, top - width - 0.1), cuts_at(triangles, ranges, top - width / 2.)) else {
            continue;
        };
        let Some(outer_at) = (0..loops.len()).max_by(|a, b| area(&loops[*a]).abs().total_cmp(&area(&loops[*b]).abs())) else {
            continue;
        };
        let outer = if area(&loops[outer_at]) > 0. { loops[outer_at].clone() } else { reversed(&loops[outer_at]) };
        let bevelled = |chain: &[Cut], outside: bool| {
            chain.iter().all(|c| {
                let p = ((c.0.0 + c.1.0) / 2., (c.0.1 + c.1.1) / 2.);
                let near = mids.iter().map(|m| segment_distance(p, m.0, m.1)).fold(f64::MAX, f64::min);
                (near - width / 2.).abs() < 0.08
                    && mids
                        .iter()
                        .filter(|m| {
                            let q = ((m.0.0 + m.1.0) / 2., (m.0.1 + m.1.1) / 2.);
                            segment_distance(q, c.0, c.1) < width
                        })
                        .all(|m| contains(chain, ((m.0.0 + m.1.0) / 2., (m.0.1 + m.1.1) / 2.)) == outside)
            })
        };
        for (i, chain) in loops.iter().enumerate() {
            if i != outer_at && !contains(&outer, chain[0].0) {
                continue;
            }
            let outside = i == outer_at;
            let path = if bevelled(chain, outside) {
                if area(chain) > 0. { chain.clone() } else { reversed(chain) }
            } else if outside {
                let all: Vec<Cut> = loops.iter().flatten().copied().collect();
                let Some(hull) = convex_perimeter(&all) else {
                    continue;
                };
                if boundary(&hull, faces, frame, rows)?.is_none() || !convex_bevel_clear(triangles, &hull, faces, frame, top, width)? {
                    continue;
                }
                hull
            } else {
                continue;
            };
            let needs_wall = if outside { wall(&path) } else { None };
            if outside && needs_wall.is_none() {
                continue;
            }
            let needs_opening = if !outside {
                let p = chain[0].0;
                holes.iter().find(|h| {
                    ((h.top_z - top).abs() < LEVEL_EPS || (h.bore_top_z - (top - width)).abs() < LEVEL_EPS)
                        && (h.center[0].as_f64().unwrap_or(0.) - p.0).hypot(h.center[1].as_f64().unwrap_or(0.) - p.1) <= h.diameter / 2. + 0.1
                }).map(|h| h.feature.clone()).or_else(|| slots.iter().find(|s| {
                    s["kind"] == "slot"
                        && s["top_z"].as_f64().is_some_and(|z| (z - top).abs() < LEVEL_EPS)
                        && segment_distance(
                            p,
                            (s["a"][0].as_f64().unwrap_or(0.), s["a"][1].as_f64().unwrap_or(0.)),
                            (s["b"][0].as_f64().unwrap_or(0.), s["b"][1].as_f64().unwrap_or(0.)),
                        ) <= s["width"].as_f64().unwrap_or(0.) / 2. + 0.1
                }).and_then(|s| s["features"][0].as_str().map(String::from)))
            } else { None };
            if !outside && needs_opening.is_none() { continue; }
            let ids: Vec<_> = features
                .iter()
                .filter(|(_, p)| path.iter().any(|c| segment_distance((p[0], p[1]), c.0, c.1) < width + 0.1))
                .map(|(id, _)| id.clone())
                .collect();
            if ids.is_empty() {
                continue;
            }
            let selected = if outside { path.clone() } else { reversed(&path) };
            let Some(vertices) = boundary(&selected, faces, frame, rows)? else {
                continue;
            };
            let outline = if outside {
                vertices.clone()
            } else {
                let Some(outline) = boundary(&outer, faces, frame, rows)? else {
                    continue;
                };
                outline
            };
            let room = if outside { room_beside(triangles, &path, top-width, 0.1) } else { None };
            if room.is_some_and(|r| r < width+1.) { continue; }
            let maximum = if outside {
                None
            } else {
                let lo_x = path.iter().map(|c| c.0.0).fold(f64::INFINITY, f64::min);
                let hi_x = path.iter().map(|c| c.0.0).fold(f64::NEG_INFINITY, f64::max);
                let lo_y = path.iter().map(|c| c.0.1).fold(f64::INFINITY, f64::min);
                let hi_y = path.iter().map(|c| c.0.1).fold(f64::NEG_INFINITY, f64::max);
                Some((hi_x - lo_x).min(hi_y - lo_y))
            };
            ops.push(json!({"kind":"edge_chamfer", "maximum_diameter":maximum, "room":room, "width":r(width), "top_z":r(top), "outer":if outside {json!(vertices)} else {json!(outline)},
                "holes":if outside {json!([])} else {json!([vertices])}, "boundaries":if outside {"outer"} else {"holes"},
                "needs_openings":needs_opening.into_iter().collect::<Vec<_>>(), "needs_wall":needs_wall, "wall_features":if outside {ids.clone()} else {vec![]}, "features":ids}));
            covered.extend(ids);
        }
    }
    Ok((ops, covered))
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
        if face["kind"].as_str() != Some("cylinder") || rotate(rows, v3(&face["direction"])?)[2].abs() < 0.99 {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The cam plan for an analysis (and optionally its mesh).
pub fn plan(analysis: &Value, mesh: Option<&Value>, options: &Options) -> Result<Value, String> {
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
    let mut slots: HashMap<&'static str, Vec<Value>> = HashMap::new();
    let mut trace = Trace::default();
    let mut covered: Vec<Value> = vec![];
    let mut roughing_feature_depths = Map::new();
    // The decision that put each hole in its setup.
    let mut placed: HashMap<String, String> = HashMap::new();
    let asked = threads(analysis, &mut trace);
    if mesh.is_none() {
        trace.warn("process.no_mesh_checks", "part", "no mesh: hole approaches and what lies beyond through holes were not checked against the part".to_string(), json!({}));
    }
    let mut in_slot: Vec<String> = vec![];
    for feature in all_features.iter().filter(|x| x["family"] == "slots") {
        if let Some((name, op)) = slot_operation(feature, &cx)? {
            in_slot.extend(op["features"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from));
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
            in_slot.extend(op["features"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from));
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
                Ok((name, hole, w, blocked)) => {
                    let considered: Vec<Value> = blocked.iter().map(|b| json!({"candidate": b, "rejected": "part material above its opening"})).collect();
                    let says = format!("cut from the {name} setup");
                    placed.insert(id.to_string(), trace.decide("setup.hole.first_reachable", id, "chosen", &says, json!({"chosen": name, "considered": considered})));
                    by_setup.entry(name).or_default().push(hole);
                    for (rule, text) in w {
                        trace.warn(rule, id, text, json!({"features": [id]}));
                    }
                }
                Err((rule, reason)) => {
                    trace.skip(rule, id, reason);
                    if let Some(thread) = asked.get(id) {
                        trace.skip("process.hole.thread_lost", id, format!("its {} thread: the hole is not cut", thread.spec));
                    }
                }
            },
            // Cut by its slot's operation, or its member holes are planned one by one.
            Value::String(action) if action == "by_slot" || action == "members" => {}
            Value::String(action) if action == "covered" => {
                trace.decide("process.covered_by_roughing", id, "covered", &format!("{family}: no operation of its own; roughing reaches it"), Value::Null);
                covered.push(json!(id));
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
                        let allowance = if faces.get(&index).is_some_and(|f| f["kind"] == "plane") { 1e-4 * 3f64.sqrt() } else { CAMLIB_MAX_SURFACE_ERROR };
                        Some(low - allowance)
                    }).collect::<Option<Vec<_>>>()?.into_iter().min_by(f64::total_cmp)
                }).unwrap_or(bottom);
                roughing_feature_depths.insert(id.into(), json!({"floor_z": lowest, "family": family}));
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
        let mut ops: Vec<Value> = vec![];
        if *name == rough_from {
            let part = bounds(&box_corners(part_lo, part_hi).map(|p| frame.point(p)));
            let (part_top, part_bottom) = (part.1[2], part.0[2]);
            if frame.stock.1[2] - part_top > 1e-6 {
                ops.push(json!({"kind": "facing", "floor_z": r(part_top)}));
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
                            .chunks_exact(3)
                            .flat_map(|p| frame.point([p[0], p[1], p[2]]).map(r))
                            .collect();
                        ops.push(json!({
                            "kind": "roughing",
                            "top_z": r(frame.stock.1[2]),
                            "floor_z": r(floor),
                            "web_mm": r(floor - part_bottom),
                            // Rounded up, so the rounding never understates it.
                            "surface_error_mm": (error * 1e4).ceil() / 1e4,
                            "positions": positions,
                            "indices": m["indices"].clone(),
                        }));
                        // Flats between the top, which facing cuts, and the floor are off the
                        // roughing layers, so they can have up to a stepdown left on them.
                        let top = bounds(&box_corners(part_lo, part_hi).map(|p| frame.point(p))).1[2];
                        let mut levels: Vec<f64> = faces
                            .values()
                            .filter(|f| f["kind"] == "plane")
                            .filter_map(|f| {
                                let up = rotate(&frame.rows, v3(&f["direction"]).ok()?)[2] > 1. - 1e-9;
                                up.then(|| Some(r(frame.point(v3(&f["centroid"]).ok()?)[2])))?
                            })
                            .filter(|z| *z > floor && *z < r(top))
                            .collect();
                        levels.sort_by(f64::total_cmp);
                        levels.dedup();
                        if !levels.is_empty() {
                            ops.push(json!({"kind": "flats", "floor_z": r(floor), "levels": levels}));
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
        if let (Some(m), Some(all), true) = (mesh, triangles.as_deref(), ops.iter().any(|o| o["kind"] == "roughing")) {
            let part = bounds(&box_corners(part_lo, part_hi).map(|p| frame.point(p)));
            let floor = (part.0[2] + options.skin).min(part.1[2]);
            let mut heights = vec![floor];
            heights.extend(ops.iter().find(|o| o["kind"] == "flats").into_iter().flat_map(|o| o["levels"].as_array().into_iter().flatten()).filter_map(Value::as_f64));
            heights.push(part.1[2]);
            let error = ops.iter().find(|o| o["kind"] == "roughing").and_then(|o| o["surface_error_mm"].as_f64()).unwrap_or(0.);
            let own = (2. * error).max(0.01);
            let (walls, outlines) = match m["faces"].as_array() {
                Some(ranges) => wall_operations(frame.triangles(all), ranges, &faces, &frame, rows_of(name), &heights, own, &mut trace, name)?,
                None => (vec![], vec![]),
            };
            // Modelled chamfers may be on a lower flat; rounds retain their floor rule.
            let wall_of = |outer: &[Cut], floor_only: bool| {
                walls.iter().zip(&outlines).find(|(w, o)| {
                    if floor_only && w["floor_z"].as_f64() != Some(r(floor)) { return false; }
                    let hull;
                    let o = if !floor_only && (area(o).abs()-area(outer).abs()).abs() > 0.005*area(outer).abs() {
                        hull = convex_perimeter(o).unwrap_or_default();
                        &hull
                    } else { o };
                    (area(o).abs() - area(outer).abs()).abs() <= 0.005 * area(outer).abs() && o.iter().all(|c| outer.iter().any(|d| segment_distance(c.0, d.0, d.1) <= 0.1))
                }).and_then(|(w, _)| w["wall"].as_str().map(String::from))
            };
            let edges;
            (edges, chamfered) = edge_operations(m, all, &cx, &faces, name, part.1[2], floor, holes, &setup_slots, &|o| wall_of(o, true))?;
            let (modelled, ids) = modelled_edge_operations(m, all, &cx, &faces, name, (holes, &setup_slots), &|o| wall_of(o, false))?;
            chamfered.extend(ids);
            ops.extend(setup_slots);
            ops.extend(walls);
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
            ranked.push((crate::rules::rank(op["kind"].as_str().unwrap_or(""))?, op));
        }
        ranked.sort_by_key(|(rank, _)| *rank);
        let mut ops: Vec<Value> = ranked.into_iter().map(|(_, op)| op).collect();
        // Why each operation is here: its setup, the order, the rule that made it, and for holes
        // the decision that put them in this setup.
        let stock = json!({"min": rp(frame.stock.0), "max": rp(frame.stock.1)});
        let in_setup = trace.decide("setup.stock.part_box_with_margins", name, "chosen", &format!("the {name} setup, zeroed on the {}", frame.zeroed_on), json!({"zero": frame.zeroed_on, "stock": stock}));
        let order = trace.decide("sequence.fixed_order", name, "chosen", "facing, roughing, flats, spot drills, holes, counterbores, slots, notches and V-channels, walls, edge chamfers, threads last", Value::Null);
        for op in &mut ops {
            let kind = op["kind"].as_str().unwrap_or("").to_string();
            let features: Vec<String> = op["features"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
            let rule = match kind.as_str() {
                "facing" => "process.facing",
                "roughing" => "process.roughing",
                "flats" => "process.flats",
                "holes" => "process.holes",
                "spot_drill" => "process.spot_drill",
                "counterbore" => "process.counterbore",
                "thread" => "process.thread",
                "slot" => "process.slot",
                "notch" => "process.notch",
                "v_channel" => "process.v_channel",
                "wall_finish" => "process.wall_finish",
                _ => "process.edge_chamfer",
            };
            let says = if features.is_empty() { format!("{kind} in the {name} setup") } else { format!("{kind} in the {name} setup: {}", features.join(", ")) };
            let made = trace.decide(rule, &format!("{name}:{kind}"), "planned", &says, json!({"features": features}));
            let holes_placed: Vec<String> = features.iter().filter_map(|f| placed.get(f).cloned()).collect();
            if kind == "edge_chamfer" {
                // Rounds skipped earlier are cut as this chamfer after all.
                for id in features.iter().filter(|f| chamfered.contains(f)) {
                    trace.supersede(id, "skipped", &made);
                }
            }
            because(op, [in_setup.clone(), order.clone(), made].into_iter().chain(holes_placed));
        }
        trace.skipped.retain(|s: &Value| !chamfered.iter().any(|id| s["feature"].as_str() == Some(id.as_str())));
        let mut setup = Map::new();
        setup.insert("id".into(), json!(name));
        frame.describe(&mut setup);
        setup.insert("operations".into(), json!(ops));
        setups.push(Value::Object(setup));
    }

    // Nothing is covered by roughing that does not run.
    let roughs = setups.iter().any(|s| s["operations"].as_array().into_iter().flatten().any(|o| o["kind"] == "roughing"));
    if !roughs && triangles.is_some() {
        for id in covered.drain(..) {
            let id = id.as_str().unwrap_or("").to_string();
            let not_cut = trace.skip("process.no_roughing", &id, "no operation of its own, and roughing is not planned".into());
            trace.supersede(&id, "covered", &not_cut);
        }
    }

    Ok(json!({
        "format": PLAN_FORMAT,
        "version": 1,
        "units": "mm",
        "source": analysis.get("binding"),
        // The material as Specify was told it (part.material); capp::finish places it in a group.
        "material": {"specified": analysis["resume"]["answers"]["part.material"].as_str()},
        "part": {"min": rp(part_lo), "max": rp(part_hi)},
        "setups": setups,
        "covered_by_roughing": covered,
        "roughing_feature_depths": roughing_feature_depths,
        "skipped": trace.skipped,
        "warnings": trace.warnings,
        "decisions": trace.decisions,
        "rulebase": crate::trace::rulebase(),
    }))
}
