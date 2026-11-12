//! Slots, notches and V-channels, each cut as an operation of its own.

use super::*;

/// A closed slot with round ends as an operation of its own, cut to size: which setup, and the
/// operation. None if it is not that shape or no setup can reach it; roughing is then all it gets.
pub(super) fn slot_operation(feature: &Value, cx: &Context) -> Result<Option<(&'static str, Value)>, String> {
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

/// A rectangular recess, optionally with rounded corners, cut with an end mill that fits.
/// The mesh must agree that its interior is empty. An open side is cut past the part;
/// settle records any corner radius the cutter leaves beyond the model's radius.
pub(super) fn notch_operation(feature: &Value, cx: &Context) -> Result<Option<(&'static str, Value)>, String> {
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
pub(super) fn v_channel_operation(feature: &Value, cx: &Context) -> Result<Option<(&'static str, Value)>, String> {
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
