//! Slices through the mesh at a height: loops, their camlib boundaries, and whether the part
//! occupies a region above a height.

use super::*;

/// One piece of a slice through the mesh: a segment, and the mesh face it lies on.
pub(super) type Cut = ((f64, f64), (f64, f64), usize);

/// Where the mesh crosses the plane at `z`.
pub(super) fn cuts_at(triangles: &[Tri], ranges: &[Value], z: f64) -> Option<Vec<Cut>> {
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
                && (p.0 - q.0).hypot(p.1 - q.1) > SAME_POINT
            {
                cuts.push((p, q, face));
            }
        }
    }
    Some(cuts)
}

/// The cuts at `z` as closed loops, or None if they do not close. Faces are meshed apart, so ends
/// that meet may differ by a little.
pub(super) fn slice(triangles: &[Tri], ranges: &[Value], z: f64) -> Option<Vec<Vec<Cut>>> {
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
pub(super) fn area(chain: &[Cut]) -> f64 {
    chain.iter().map(|c| c.0.0 * c.1.1 - c.1.0 * c.0.1).sum()
}

pub(super) fn reversed(chain: &[Cut]) -> Vec<Cut> {
    chain.iter().rev().map(|c| (c.1, c.0, c.2)).collect()
}

pub(super) fn contains(chain: &[Cut], p: (f64, f64)) -> bool {
    chain.iter().filter(|c| (c.0.1 > p.1) != (c.1.1 > p.1) && p.0 < c.0.0 + (p.1 - c.0.1) / (c.1.1 - c.0.1) * (c.1.0 - c.0.0)).count() % 2 == 1
}

/// A loop as camlib's boundary, with its true lines and arcs: [x, y, bulge to the next vertex].
/// None if a wall is not an upright flat or a cylinder along the tool.
pub(super) fn boundary(chain: &[Cut], faces: &HashMap<u64, &Value>, frame: &Frame, rows: &Rows) -> Result<Option<Vec<[f64; 3]>>, String> {
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
                if rotate(rows, v3(&face["direction"])?)[2].abs() < ALONG_AXIS {
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
pub(super) fn room_beside(triangles: &[Tri], outline: &[Cut], floor: f64, own: f64) -> Option<f64> {
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
pub(super) fn occupied_above(mesh: &[Tri], lo: (f64, f64), hi: (f64, f64), z: f64) -> bool {
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
pub(super) fn rounded_rectangle(lo: (f64, f64), hi: (f64, f64), radius: f64) -> Vec<model::Vertex> {
    let (x, y, a, b, t) = (lo.0, lo.1, hi.0, hi.1, radius);
    if t <= 0. {
        return vec![[r(x), r(y), 0.], [r(a), r(y), 0.], [r(a), r(b), 0.], [r(x), r(b), 0.]];
    }
    let q = (std::f64::consts::PI / 8.).tan();
    let mut vertices: Vec<(f64, f64, f64)> = vec![];
    for p in [(x+t,y,0.), (a-t,y,q), (a,y+t,0.), (a,b-t,q), (a-t,b,0.), (x+t,b,q), (x,b-t,0.), (x,y+t,q)] {
        // At a semicircular end the straight tangent edge has zero length.
        if let Some(last) = vertices.last_mut().filter(|v| (v.0-p.0).hypot(v.1-p.1) < SAME_POINT) {
            last.2 = p.2;
        } else {
            vertices.push(p);
        }
    }
    vertices.into_iter().map(|(x, y, b)| [r(x), r(y), b]).collect()
}

/// An inscribed convex polygon leaves the mesh's faceted curved walls outside the check.
/// Like the wall slice checks, allow 0.05 mm at the boundary: CTC 01's r10 corners
/// have chords about 0.03 mm inside the exact arc (including the adjacent rim mesh).
pub(super) fn occupied_rounded(mesh: &[Tri], lo: (f64, f64), hi: (f64, f64), z: f64, radius: f64) -> bool {
    if radius <= 0. { return occupied_above(mesh, lo, hi, z); }
    let mut corners = vec![];
    for (x,y,angle) in [(hi.0-radius,lo.1+radius,-90.),(hi.0-radius,hi.1-radius,0.),(lo.0+radius,hi.1-radius,90.),(lo.0+radius,lo.1+radius,180.)] {
        for i in 0..=32 {
            let a = (angle + 90. * i as f64 / 32.).to_radians();
            corners.push((x+(radius-0.05).max(0.)*a.cos(),y+(radius-0.05).max(0.)*a.sin()));
        }
    }
    corners.dedup_by(|a, b| (a.0-b.0).hypot(a.1-b.1) < SAME_POINT);
    if corners.first().zip(corners.last()).is_some_and(|(a, b)| (a.0-b.0).hypot(a.1-b.1) < SAME_POINT) {
        corners.pop();
    }
    occupied_polygon(mesh, &corners, z)
}
