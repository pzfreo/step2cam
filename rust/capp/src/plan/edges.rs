//! Top-edge chamfers and rounds: edge-break loops and modelled 45 degree chamfers.

use super::*;

/// Chamfers the top edges that the model rounds, in the roughing setup: (operations, ids of the
/// rounds they stand for). A 45° chamfer meeting a round at its middle never cuts into the part.
/// A loop qualifies only if its whole top edge has the one radius and its wall ends up at size:
/// the outside (`wall` is the operation that finishes this outline) or a hole or slot's (already
/// cut). An outside chamfer says which wall it needs, as that wall can still be left out.
pub(super) fn edge_operations(mesh: &Value, triangles: &[Tri], cx: &Context, faces: &HashMap<u64, &Value>, setup: &'static str, part_top: f64, floor: f64, holes: &[Hole], slots: &[Value], wall: &dyn Fn(&[Cut]) -> Option<String>) -> Result<(Vec<Value>, Vec<String>), String> {
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
pub(super) fn convex_perimeter(chain: &[Cut]) -> Option<Vec<Cut>> {
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
pub(super) fn modelled_edge_operations(
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
