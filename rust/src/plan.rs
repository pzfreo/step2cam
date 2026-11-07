//! specify-core analysis (+ mesh) -> cam plan: what to cut, per setup, independent of tools and
//! machine. Each setup is the part turned so its tool comes down -Z, with X0 Y0 on the minimum
//! corner and Z0 on the top of the stock (the roughing setup) or of the part's box (later ones).
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
/// Families made on a lathe.
const TURNED: [&str; 3] = ["turned_bodies", "turned_steps", "face_grooves"];
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
pub(crate) fn r(x: f64) -> f64 {
    format!("{x:.4}").parse::<f64>().unwrap_or(x) + 0.0
}
fn rp(p: V3) -> Value {
    json!([r(p[0]), r(p[1]), r(p[2])])
}
/// A number as messages show it.
pub(crate) fn show(x: f64) -> String {
    format!("{x:?}")
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
    csink: bool,
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

/// ISO 261 coarse pitches, as specify-core offers tapped sizes (its `PITCH` table).
const COARSE_PITCH: [(&str, f64); 13] = [
    ("M1.6", 0.35), ("M2", 0.4), ("M2.5", 0.45), ("M3", 0.5), ("M4", 0.7), ("M5", 0.8), ("M6", 1.0),
    ("M8", 1.25), ("M10", 1.5), ("M12", 1.75), ("M16", 2.0), ("M20", 2.5), ("M24", 3.0),
];

/// (major diameter, pitch) of a metric thread spec (`M5`, `M10x1.25`).
fn thread_size(spec: &str) -> Option<(f64, f64)> {
    let (size, fine) = spec.split_once('x').map_or((spec, None), |(a, b)| (a, Some(b)));
    let major: f64 = size.strip_prefix('M')?.parse().ok()?;
    let pitch = match fine {
        Some(text) => text.parse().ok()?,
        None => COARSE_PITCH.iter().find(|(name, _)| *name == size)?.1,
    };
    (major > 0. && pitch > 0. && pitch < major).then_some((major, pitch))
}

/// specify-core's default minimum full thread in a blind tapped hole of drill depth `depth`: the
/// depth less three pitches, rounded down to 0.5 mm, at least one pitch.
fn full_thread(depth: f64, pitch: f64) -> f64 {
    let down = ((depth - 3. * pitch) * 2.).floor() / 2.;
    if down > pitch { down } else { pitch }
}

/// The internal threads asked for, by hole feature id: from Specify's answers
/// (`hole.function:<hole or pattern>` = `tapped:M5` or `tapped:M5@5.5`). Threads another tool wrote,
/// which state no size, and answers that cannot be used become warnings.
fn threads(analysis: &Value, warnings: &mut Vec<String>) -> HashMap<String, Thread> {
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
            warnings.push(format!("{target}: tapped {spec} is not a metric size this bridge knows; not threaded"));
            continue;
        };
        let full_depth = match full.filter(|text| !text.is_empty()).map(str::parse::<f64>) {
            None => None,
            Some(Ok(depth)) => Some(depth),
            Some(Err(_)) => {
                warnings.push(format!("{target}: tapped '{}' has an unreadable depth; not threaded", value.as_str().unwrap_or("")));
                continue;
            }
        };
        let Some(feature) = features.iter().find(|f| f["id"].as_str() == Some(target)) else {
            warnings.push(format!("{target}: tapped {spec}, but the part has no such feature now; not threaded"));
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
                warnings.push(format!("{id}: the file marks an internal thread here but states no size; not threaded (answer it in Specify to thread it)"));
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
fn hole_geometry(feature: &Value, cx: &Context, thread: Option<&Thread>) -> Result<Result<(&'static str, Hole, Vec<String>), String>, String> {
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
            return Ok(Err(format!(
                "part material above its opening from {}: a three-axis tool cannot reach it",
                blocked.join(", ")
            )));
        }
        let axis = v3(&record["axis"])?;
        return Ok(Err(format!(
            "oblique axis [{}, {}, {}]: needs a setup this bridge does not make",
            show(r(axis[0])), show(r(axis[1])), show(r(axis[2]))
        )));
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
        csink: false,
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
            warnings.push(format!(
                "{id}: the through hole ends {} mm short of the part's far face in the {name} setup; \
                 anything between (a counterbore or chamfer at the far end, a shoulder) is not planned",
                show(r(bottom[2] - part_bottom))
            ));
        }
    }
    if let Some(thread) = thread {
        // The thread runs down the bore from its top: all of it for a through hole, else the
        // minimum full thread asked for (specify-core's default when none was).
        let (top, mut end) = (hole.bore_top_z, hole.bottom_z);
        let length = top - end;
        let mut fits = true;
        if hole.bottom != "through" {
            let full = thread.full_depth.filter(|d| *d != 0.).unwrap_or_else(|| full_thread(depth, thread.pitch));
            if full > length + 1e-9 {
                warnings.push(format!("{id}: {} thread {} mm deep is deeper than the hole ({} mm); not threaded", thread.spec, show(full), show(r(length))));
                fits = false;
            } else {
                end = top - full;
            }
        }
        if fits {
            hole.thread = Some(json!({"spec": thread.spec, "major": r(thread.major), "pitch": thread.pitch, "top_z": r(top), "bottom_z": r(end)}));
        }
    }
    hole.csink = truthy(record.get("csink")) && !chosen.reversed;
    Ok(Ok((name, hole, warnings)))
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
    let key = |h: &Hole, t: &Value| (t["spec"].as_str().unwrap_or("").to_string(), t["major"].as_f64().unwrap_or(0.), t["pitch"].as_f64().unwrap_or(0.), h.diameter, t["top_z"].as_f64().unwrap_or(0.), t["bottom_z"].as_f64().unwrap_or(0.), h.bottom == "through");
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
    });
    groups
        .into_iter()
        .map(|(k, members)| {
            json!({
                "kind": "thread", "spec": k.0, "major": k.1, "pitch": k.2, "start_diameter": k.3,
                "top_z": k.4, "bottom_z": k.5, "through": k.6,
                "centers": members.iter().map(|m| m.center.clone()).collect::<Vec<_>>(),
                "features": members.iter().map(|m| m.feature.clone()).collect::<Vec<_>>(),
            })
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
/// None if a wall is not a flat or a cylinder along the tool.
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
            Some("plane") => vertices.push([r(run[0].0.0), r(run[0].0.1), 0.]),
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
                    vertices.push([r(centre[0] + radius * at.cos()), r(centre[1] + radius * at.sin()), (sweep / arcs / 4.).tan()]);
                }
            }
            _ => return Ok(None),
        }
    }
    Ok(Some(vertices))
}

/// Finishes the outside wall and chamfers the top edges that the model rounds, in the roughing
/// setup: (operations, ids of the rounds they stand for). A 45° chamfer meeting a round at its
/// middle never cuts into the part. A loop qualifies only if its whole top edge has the one radius
/// and its wall ends up at size: the outside (finished here) or a hole or slot's (already cut).
fn edge_operations(mesh: &Value, triangles: &[Tri], cx: &Context, faces: &HashMap<u64, &Value>, setup: &'static str, part_top: f64, floor: f64, holes: &[Hole], slots: &[Value]) -> Result<(Vec<Value>, Vec<String>), String> {
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
        holes.iter().any(|h| {
            let c = (h.center[0].as_f64().unwrap_or(f64::MAX), h.center[1].as_f64().unwrap_or(f64::MAX));
            (c.0 - p.0).hypot(c.1 - p.1) < h.diameter / 2.
        }) || slots.iter().any(|s| {
            let (a, b) = ((s["a"][0].as_f64().unwrap_or(0.), s["a"][1].as_f64().unwrap_or(0.)), (s["b"][0].as_f64().unwrap_or(0.), s["b"][1].as_f64().unwrap_or(0.)));
            segment_distance(p, a, b) < s["width"].as_f64().unwrap_or(0.) / 2.
        })
    };
    let mut chamfered: Vec<Vec<Cut>> = vec![];
    let mut inner: Vec<Value> = vec![];
    for (i, chain) in walls.iter().enumerate() {
        let (count, sum) = (chain.len() as f64, chain.iter().fold((0., 0.), |s, c| (s.0 + c.0.0, s.1 + c.0.1)));
        let middle_of = (sum.0 / count, sum.1 / count);
        if i == outer_at || !contains(&outer, chain[0].0) {
            continue;
        }
        if rounded(chain) && cut_to_size(middle_of) {
            let clockwise = if area(chain) < 0. { chain.clone() } else { reversed(chain) };
            if let Some(vertices) = boundary(&clockwise, faces, frame, rows)? {
                inner.push(json!(vertices));
                chamfered.push(clockwise);
            }
        }
    }
    let Some(outline) = boundary(&outer, faces, frame, rows)? else { return Ok(nothing) };
    // The finish runs down to the floor, so the wall must too: on a stepped part this outline
    // is a raised part's, and following it down would cut what it stands on.
    let largest = |z: f64| slice(triangles, ranges, z).and_then(|l| l.iter().map(|c| area(c).abs()).max_by(f64::total_cmp));
    let same = |z: f64| largest(z).is_some_and(|a| (a - area(&outer)).abs() <= 0.005 * area(&outer));
    let wall_low = floor + radius + 0.05;
    let outside = rounded(&outer) && same(wall_low) && same((wall_low + part_top - radius) / 2.);
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
    let mut ops = vec![];
    if outside {
        ops.push(json!({"kind": "wall_finish", "outer": outline, "floor_z": r(floor)}));
    }
    ops.push(json!({
        "kind": "edge_chamfer", "radius": r(radius), "width": r(radius * (2. - 2f64.sqrt())), "top_z": r(part_top),
        "outer": outline, "holes": inner, "boundaries": match (outside, inner.is_empty()) {
            (true, true) => "outer",
            (true, false) => "all",
            _ => "holes",
        },
        "features": ids,
    }));
    Ok((ops, ids))
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
    let (mut skipped, mut covered, mut warnings) = (vec![], vec![], vec![]);
    let asked = threads(analysis, &mut warnings);
    if mesh.is_none() {
        warnings.push("no mesh: hole approaches and what lies beyond through holes were not checked against the part".to_string());
    }
    let skip = |feature: &str, reason: String| json!({"feature": feature, "reason": reason});
    let mut in_slot: Vec<String> = vec![];
    for feature in all_features.iter().filter(|x| x["family"] == "slots") {
        if let Some((name, op)) = slot_operation(feature, &cx)? {
            in_slot.extend(op["features"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from));
            slots.entry(name).or_default().push(op);
        }
    }
    for feature in all_features {
        let family = feature["family"].as_str().unwrap_or("");
        let id = feature["id"].as_str().unwrap_or("");
        if family == "holes" {
            match hole_geometry(feature, &cx, asked.get(id))? {
                Ok((name, hole, w)) => {
                    by_setup.entry(name).or_default().push(hole);
                    warnings.extend(w);
                }
                Err(reason) => {
                    skipped.push(skip(id, reason));
                    if let Some(thread) = asked.get(id) {
                        skipped.push(skip(id, format!("its {} thread: the hole is not cut", thread.spec)));
                    }
                }
            }
        } else if in_slot.iter().any(|x| x == id) {
            // Cut by its slot's operation.
        } else if family == "fillets" && feature["record"]["side"] == "convex" && !upright(feature, &faces, rows_of(rough_from))? {
            skipped.push(skip(id, format!("r{} edge round: not reached from the roughing setup with its wall at size, so it stays sharp (break it by hand)", show(f(&feature["record"], "radius")?))));
        } else if ["chamfers", "oriented_chamfers"].contains(&family) {
            skipped.push(skip(id, "chamfer: no operation cuts it, so the edge stays sharp (chamfer it by hand)".into()));
        } else if family == "hole_patterns" {
            // Its member holes are planned one by one.
        } else if TURNED.contains(&family) {
            skipped.push(skip(id, "turned feature: make it on a lathe".into()));
        } else if triangles.is_none() {
            skipped.push(skip(id, format!("{family}: no operation of its own; give the mesh to rough it")));
        } else if reachable(feature, &faces, rows_of(rough_from))? {
            covered.push(json!(id));
        } else {
            skipped.push(skip(id, format!("{family}: faces away from the {rough_from} setup, where roughing runs; not planned")));
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
                    skipped.push(skip("roughing", format!(
                        "faces [{}] did not triangulate: a mesh with holes in it would let roughing cut into the part",
                        list.join(", ")
                    )));
                } else {
                    // specify-core meshes curved faces to 0.1% of the part's diagonal.
                    let d = [part_hi[0] - part_lo[0], part_hi[1] - part_lo[1], part_hi[2] - part_lo[2]];
                    let error = surface_error(m, &faces, dot(d, d).sqrt() * 0.001)?;
                    if error > CAMLIB_MAX_SURFACE_ERROR {
                        skipped.push(skip("roughing", format!(
                            "the mesh strays up to {} mm from the part's curved faces; camlib accepts at most {} mm",
                            show(r(error)), show(CAMLIB_MAX_SURFACE_ERROR)
                        )));
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
                    }
                }
            }
        }
        if ops.is_empty() && holes.is_empty() && setup_slots.is_empty() {
            continue;
        }
        ops.extend(hole_operations(holes));
        ops.extend(counterbore_operations(holes));
        ops.extend(thread_operations(holes));
        if let (Some(m), Some(all), true) = (mesh, triangles.as_deref(), ops.iter().any(|o| o["kind"] == "roughing")) {
            let part = bounds(&box_corners(part_lo, part_hi).map(|p| frame.point(p)));
            let floor = (part.0[2] + options.skin).min(part.1[2]);
            let (edges, chamfered) = edge_operations(m, all, &cx, &faces, name, part.1[2], floor, holes, &setup_slots)?;
            ops.extend(setup_slots);
            ops.extend(edges);
            skipped.retain(|s: &Value| !chamfered.iter().any(|id| s["feature"].as_str() == Some(id.as_str())));
        } else {
            ops.extend(setup_slots);
        }
        for h in holes {
            if h.csink {
                skipped.push(skip(&h.feature, "countersink: not planned (chamfer it by hand)".into()));
            }
        }
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
            let id: Value = id;
            skipped.push(skip(id.as_str().unwrap_or(""), "no operation of its own, and roughing is not planned".into()));
        }
    }

    Ok(json!({
        "format": PLAN_FORMAT,
        "version": 1,
        "units": "mm",
        "source": analysis.get("binding"),
        "part": {"min": rp(part_lo), "max": rp(part_hi)},
        "setups": setups,
        "covered_by_roughing": covered,
        "skipped": skipped,
        "warnings": warnings,
    }))
}
