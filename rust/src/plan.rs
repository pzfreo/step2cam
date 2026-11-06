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
/// Rounded for the plan, with no negative zero: the nearest 4-decimal value, as Python's round.
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
/// The most the mesh strays from the part: none on flat faces, measured on cylinders, and
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
}

struct Context<'a> {
    frames: &'a HashMap<&'static str, Frame>,
    preferred: &'a [&'static str],
    triangles: Option<&'a [Tri]>,
    part_faces: &'a HashMap<&'static str, f64>,
}

/// A hole as setup-frame geometry, or why it is skipped.
fn hole_geometry(feature: &Value, cx: &Context) -> Result<Result<(&'static str, Hole, Vec<String>), String>, String> {
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

    let cx = Context { frames: &frames, preferred: &preferred, triangles: triangles.as_deref(), part_faces: &part_faces };
    let mut by_setup: HashMap<&'static str, Vec<Hole>> = HashMap::new();
    let (mut skipped, mut covered, mut warnings) = (vec![], vec![], vec![]);
    if mesh.is_none() {
        warnings.push("no mesh: hole approaches and what lies beyond through holes were not checked against the part".to_string());
    }
    let skip = |feature: &str, reason: String| json!({"feature": feature, "reason": reason});
    for feature in analysis["features"].as_array().ok_or("features is not a list")? {
        let family = feature["family"].as_str().unwrap_or("");
        let id = feature["id"].as_str().unwrap_or("");
        if family == "holes" {
            match hole_geometry(feature, &cx)? {
                Ok((name, hole, w)) => {
                    by_setup.entry(name).or_default().push(hole);
                    warnings.extend(w);
                }
                Err(reason) => skipped.push(skip(id, reason)),
            }
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
        if ops.is_empty() && holes.is_empty() {
            continue;
        }
        ops.extend(hole_operations(holes));
        ops.extend(counterbore_operations(holes));
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
