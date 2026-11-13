//! Wall finishing: outlines of the part's section in bands, finished from the setup's top.

use super::*;

/// Finishes the outsides of the part's solid outlines, in the roughing setup, at each band between
/// flats: a contour stops at its band's floor, so a raised part's outline is not followed through
/// what it stands on. An outline already finished from a lower band is not repeated, as a contour
/// starts at the top of the stock. The outline is the band's middle, or lower where a side hole
/// breaks that: what tapers in stays clear of the cutter, and what flares out at the foot or
/// overhangs above leaves no room beside it. Returns the operations and each one's outline.
#[allow(clippy::too_many_arguments)] // Each is a different piece of the setup; a struct would only rename them.
pub(super) fn wall_operations(triangles: &[Tri], ranges: &[Value], faces: &HashMap<u64, &Value>, frame: &Frame, rows: &Rows, heights: &[f64], own: f64, trace: &mut Trace, setup: &str) -> Result<(Vec<model::WallFinish>, Vec<Vec<Cut>>), String> {
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
                // In the part's section: an odd count of the mesh's cuts along +X from the point.
                if !contains(&cuts, (mid.0 + left.0, mid.1 + left.1)) || contains(&cuts, (mid.0 - left.0, mid.1 - left.1)) {
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
                ops.push(model::WallFinish {
                    wall: format!("wall-{}", ops.len() + 1), outer: vertices, floor_z: r(low),
                    on_flat: band_at > 0, room: room.map(r),
                });
            }
        }
        if unreadable && !had_wall {
            trace.warn("process.wall_finish.not_upright", setup, format!("an outside wall above Z{} is not upright planes and cylinders, so it keeps its roughing allowance", show(r(low))), json!({}));
        }
    }
    Ok((ops, finished))
}
