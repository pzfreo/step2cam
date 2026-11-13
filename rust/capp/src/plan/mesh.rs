//! The part's mesh: its triangles, what stands above or below a point, and how far it strays
//! from the part's surfaces.

use super::*;

/// A mesh vertex this close to a curved face lies on it (mm).
const ON_SURFACE: f64 = 1e-3;

/// The mesh's triangles; an error, never a panic, if it is malformed.
pub(crate) fn mesh_triangles(mesh: &Value) -> Result<Vec<Tri>, String> {
    let p = nums(&mesh["positions"])?;
    let idx = mesh["indices"].as_array().ok_or("mesh has no indices")?;
    let point = |i: usize| -> Result<V3, String> {
        if 3 * i + 2 >= p.len() {
            return Err("mesh index out of range".into());
        }
        Ok([p[3 * i], p[3 * i + 1], p[3 * i + 2]])
    };
    let mut out = Vec::with_capacity(idx.len() / 3);
    for t in idx.as_chunks::<3>().0 {
        let i = |k: usize| t[k].as_u64().map(|v| v as usize).ok_or("bad mesh index");
        out.push([point(i(0)?)?, point(i(1)?)?, point(i(2)?)?]);
    }
    Ok(out)
}

/// Which way `p` lies from the line a→b: positive to the left, twice the triangle's area.
pub(super) fn turn(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}
fn inside(c: (f64, f64), t: [(f64, f64); 3]) -> bool {
    // A vertical triangle projects to a line: nothing is inside it, and its edges are measured.
    if turn(t[0], t[1], t[2]).abs() < 1e-9 {
        return false;
    }
    let (s1, s2, s3) = (turn(t[0], t[1], c), turn(t[1], t[2], c), turn(t[2], t[0], c));
    (s1 >= 0. && s2 >= 0. && s3 >= 0.) || (s1 <= 0. && s2 <= 0. && s3 <= 0.)
}
pub(super) fn segment_distance(c: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length == 0. { 0. } else { (((c.0 - a.0) * dx + (c.1 - a.1) * dy) / length).clamp(0., 1.) };
    (c.0 - (a.0 + t * dx)).hypot(c.1 - (a.1 + t * dy))
}
/// Whether the XY projection of a triangle comes within `radius` of `c`.
pub(super) fn disk_meets(c: (f64, f64), radius: f64, t: &Tri) -> bool {
    let p = t.map(|v| (v[0], v[1]));
    inside(c, p) || (0..3).any(|i| segment_distance(c, p[i], p[(i + 1) % 3]) < radius)
}
#[doc(hidden)]
pub fn disk_meets_for_test(c: (f64, f64), radius: f64, t: &[[f64; 3]; 3]) -> bool {
    disk_meets(c, radius, t)
}
/// Whether any part material lies above `z` within `radius` of `c` (setup frame).
pub(super) fn blocked_above(triangles: &[Tri], c: (f64, f64), z: f64, radius: f64) -> bool {
    let radius = radius - LEVEL_EPS;
    triangles.iter().any(|t| {
        t[0][2].max(t[1][2]).max(t[2][2]) > z + LEVEL_EPS && disk_meets(c, radius, t)
    })
}
/// How far below `z` the column at `c` is free of part material, or None if nothing is below.
pub(super) fn clear_below(triangles: &[Tri], c: (f64, f64), z: f64, radius: f64) -> Option<f64> {
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
        ((rho - major).hypot(h) - minor).abs() <= ON_SURFACE
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
    if points.iter().any(|p| off(*p) > ON_SURFACE) {
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
pub(super) fn surface_error(mesh: &Value, faces: &HashMap<u64, &Value>, deflection: f64) -> Result<f64, String> {
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
    Ok(worst + ROUNDING_ERROR)
}
