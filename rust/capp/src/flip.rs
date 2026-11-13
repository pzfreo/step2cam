//! The vise flip-and-face option (#86). Callers can offer or withhold this function
//! independently of machine-independent planning; setup.workholding owns the choice.
use crate::{
    model::{Box3, Cut, Operation, Plan, Setup},
    plan::{Options, mesh_triangles, r},
    trace::{Trace, because_op},
};
use serde_json::{Value, json};
const RULE: &str = "setup.flip_face";
type Point = [f64; 3];
type Tri = [Point; 3];

/// Solve B >= grip(H + T + B) + clearance + facing allowance, without
/// rounding the fixed point down. Parallels prescribe a fixed grip instead.
fn bottom_stock(height: f64, top: f64, parallel: Option<f64>, jaw: f64) -> Result<f64, String> {
    let rule = crate::rules::workholding();
    let minimum = rule["min_grip_mm"].as_f64().unwrap();
    let fraction = rule["min_grip_fraction"].as_f64().unwrap();
    let extra =
        rule["clearance_mm"].as_f64().unwrap() + rule["flip_facing_allowance_mm"].as_f64().unwrap();
    let b = if let Some(p) = parallel {
        let grip = jaw - p;
        let b = grip + extra;
        if !p.is_finite() || p <= 0. || grip < minimum.max(fraction * (height + top + b)) {
            return Err("parallels prescribe less than the minimum Op 1 grip".into());
        }
        b
    } else {
        (minimum + extra).max((fraction * (height + top) + extra) / (1. - fraction))
    };
    let rounded = r(b);
    let b = if rounded + 1e-9 >= b {
        rounded
    } else {
        rounded + 0.0001
    };
    let grip = parallel.map_or(minimum.max(fraction * (height + top + b)), |p| jaw - p);
    if grip > jaw || grip > (height + top + b) * rule["max_grip_fraction"].as_f64().unwrap() {
        return Err("Op 1 grip exceeds the jaw height or maximum stock fraction".into());
    }
    Ok(b)
}

pub(crate) fn area(p: &[[f64; 2]]) -> f64 {
    (0..p.len())
        .map(|i| {
            let q = p[(i + 1) % p.len()];
            p[i][0] * q[1] - q[0] * p[i][1]
        })
        .sum::<f64>()
        .abs()
        / 2.
}
/// Clip a polygon to a rectangular band. Used for mesh contact measurements
/// and again by the retained-operation audit; holes never become box material.
pub(crate) fn clipped(mut poly: Vec<[f64; 2]>, rect: [[f64; 2]; 2]) -> Vec<[f64; 2]> {
    for axis in 0..2 {
        for (side, corner) in rect.iter().enumerate() {
            let edge = corner[axis];
            let inside = |p: [f64; 2]| {
                if side == 0 {
                    p[axis] >= edge
                } else {
                    p[axis] <= edge
                }
            };
            let mut out = vec![];
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                if inside(a) {
                    out.push(a);
                }
                if inside(a) != inside(b) {
                    let t = (edge - a[axis]) / (b[axis] - a[axis]);
                    out.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
                }
            }
            poly = out;
        }
    }
    poly
}
fn patches(tris: &[Tri], axes: [usize; 2], rect: [[f64; 2]; 2]) -> Vec<Vec<[f64; 2]>> {
    tris.iter()
        .map(|t| clipped(t.iter().map(|p| [p[axes[0]], p[axes[1]]]).collect(), rect))
        .filter(|p| area(p) > 1e-9)
        .collect()
}
fn measured(tris: &[Tri], axes: [usize; 2], rect: [[f64; 2]; 2]) -> f64 {
    patches(tris, axes, rect).iter().map(|p| area(p)).sum()
}

/// Contact may be interrupted by modelled holes/notches. Measure the retained
/// planar face, and search inboard for supported strips below horizontal faces.
fn seating(mesh: &Value, lo: Point, hi: Point, vise: &Value) -> Result<Value, String> {
    let width = vise["jaw_width_mm"].as_f64().unwrap();
    let tris = mesh_triangles(mesh).map_err(|_| "flip seating needs a complete mesh")?;
    let rule = crate::rules::workholding();
    let bar = rule["min_grip_mm"].as_f64().unwrap();
    let fraction = rule["flip_min_contact_fraction"].as_f64().unwrap();
    let support_fraction = rule["flip_min_support_fraction"].as_f64().unwrap();
    let inset = rule["flip_max_support_inset_mm"].as_f64().unwrap();
    let step = rule["flip_support_search_step_mm"].as_f64().unwrap();
    let eps = rule["flip_frame_tolerance_mm"].as_f64().unwrap();
    if !tris
        .iter()
        .any(|t| t[0][2] > lo[2] + bar && t.iter().all(|p| (p[2] - t[0][2]).abs() < eps))
    {
        return Err("no horizontal support face above the minimum grip level".into());
    }
    let preferred = usize::from(hi[0] - lo[0] > hi[1] - lo[1]);
    let mut best: Option<(f64, Value)> = None;
    let mut best_contact = 0_f64;
    let mut contact_ok = false;
    for axis in [preferred, 1 - preferred] {
        if hi[axis] - lo[axis] > vise["max_jaw_opening_mm"].as_f64().unwrap() {
            continue;
        }
        let along = 1 - axis;
        let mut levels: Vec<_> = tris
            .iter()
            .filter(|t| t.iter().all(|p| (p[2] - t[0][2]).abs() < eps))
            .map(|t| t[0][2])
            .collect();
        levels.sort_by(|a, b| b.total_cmp(a));
        levels.dedup_by(|a, b| (*a - *b).abs() < eps);
        let side_tris: Vec<Vec<Tri>> = [lo[axis], hi[axis]]
            .iter()
            .map(|edge| {
                tris.iter()
                    .filter(|t| t.iter().all(|p| (p[axis] - edge).abs() < eps))
                    .copied()
                    .collect()
            })
            .collect();
        let start = side_tris
            .iter()
            .map(|ts| {
                ts.iter()
                    .flatten()
                    .map(|p| p[along])
                    .min_by(f64::total_cmp)
                    .unwrap_or(f64::INFINITY)
            })
            .fold(lo[along], f64::max);
        let end = side_tris
            .iter()
            .map(|ts| {
                ts.iter()
                    .flatten()
                    .map(|p| p[along])
                    .max_by(f64::total_cmp)
                    .unwrap_or(f64::NEG_INFINITY)
            })
            .fold(hi[along], f64::min);
        let center = (lo[along] + hi[along]) / 2.;
        let (start, end) = (start.max(center - width / 2.), end.min(center + width / 2.));
        if end <= start {
            continue;
        }
        for z in levels {
            let minimum = bar.max((z - lo[2]) * rule["min_grip_fraction"].as_f64().unwrap());
            let maximum = ((z - lo[2]) * rule["max_grip_fraction"].as_f64().unwrap())
                .min(vise["jaw_height_mm"].as_f64().unwrap());
            let grips: Vec<f64> = if let Some(p) = vise["parallels_height_mm"].as_f64() {
                vec![r(vise["jaw_height_mm"].as_f64().unwrap() - p)]
            } else if let Some(heights) = vise["parallels_heights_mm"].as_array() {
                heights.iter().filter_map(Value::as_f64).map(|p| r(vise["jaw_height_mm"].as_f64().unwrap()-p)).collect()
            } else if maximum >= minimum {
                (0..=((maximum - minimum) / step).floor() as usize)
                    .map(|n| r(minimum + n as f64 * step))
                    .chain([r(maximum)])
                    .collect()
            } else {
                vec![]
            };
            for grip in grips {
                if grip < r(minimum)
                    || grip > vise["jaw_height_mm"].as_f64().unwrap()
                    || grip > r((z - lo[2]) * rule["max_grip_fraction"].as_f64().unwrap())
                    || z - lo[2] <= grip + rule["clearance_mm"].as_f64().unwrap()
                {
                    continue;
                }
                let band = [[start, z - grip], [end, z]];
                let contact: Vec<_> = side_tris
                    .iter()
                    .map(|ts| patches(ts, [along, 2], band))
                    .collect();
                let areas: Vec<f64> = contact
                    .iter()
                    .map(|ps| ps.iter().map(|p| area(p)).sum())
                    .collect();
                let fractions: Vec<_> = areas.iter().map(|a| a / ((end - start) * grip)).collect();
                let score = fractions.iter().copied().fold(1., f64::min);
                best_contact = best_contact.max(score);
                if areas
                    .iter()
                    .any(|a| *a / ((end - start) * grip) + 1e-6 < fraction || *a / grip < bar)
                {
                    continue;
                }
                contact_ok = true;
                let horizontal: Vec<_> = tris
                    .iter()
                    .filter(|t| t.iter().all(|p| (p[2] - z).abs() < eps))
                    .copied()
                    .collect();
                let mut supports = vec![];
                for side in 0..2 {
                    let mut best_support: Option<(f64, Value)> = None;
                    let limit = inset.min((hi[axis] - lo[axis]) / 2. - bar);
                    if limit < 0. {
                        continue;
                    }
                    for n in 0..=((limit / step).floor() as usize) {
                        let offset = n as f64 * step;
                        let edge = if side == 0 {
                            lo[axis] + offset
                        } else {
                            hi[axis] - offset - bar
                        };
                        let mut rect = [[0.; 2]; 2];
                        rect[0][axis] = edge;
                        rect[1][axis] = edge + bar;
                        rect[0][along] = start;
                        rect[1][along] = end;
                        let a = measured(&horizontal, [0, 1], rect);
                        // Material projecting below this seating level in the flipped
                        // setup must not intersect either parallel's footprint.
                        let obstructed = tris
                            .iter()
                            .filter(|t| t.iter().any(|p| p[2] > z + eps))
                            .any(|t| {
                                area(&clipped(t.iter().map(|p| [p[0], p[1]]).collect(), rect))
                                    > 1e-9
                            });
                        if !obstructed
                            && a / (bar * (end - start)) + 1e-6 >= support_fraction
                            && a / bar >= bar
                            && best_support.as_ref().is_none_or(|(old, _)| a > *old + eps)
                        {
                            best_support = Some((
                                a,
                                json!({"offset_mm":offset,"area_mm2":r(a),"fraction":r(a/(bar*(end-start)))}),
                            ));
                        }
                    }
                    if let Some((_, s)) = best_support {
                        supports.push(s);
                    }
                }
                if supports.len() != 2 {
                    continue;
                }
                let contact_model: Vec<Vec<Vec<Point>>> = contact
                    .iter()
                    .enumerate()
                    .map(|(side, ps)| {
                        ps.iter()
                            .map(|p| {
                                p.iter()
                                    .map(|q| {
                                        let mut point = [0.; 3];
                                        point[axis] = [lo[axis], hi[axis]][side];
                                        point[along] = q[0];
                                        point[2] = q[1];
                                        point
                                    })
                                    .collect()
                            })
                            .collect()
                    })
                    .collect();
                let contacts: Vec<_>=areas.iter().zip(&fractions).enumerate().map(|(side,(a,f))|json!({"face_model":format!("part.{}.{}",["x","y"][axis],["min","max"][side]),"area_mm2":r(*a),"fraction":r(*f),"effective_length_mm":r(a/grip)})).collect();
                let seat = json!({"axis":axis,"along_min_model":start,"along_max_model":end,"seat_z_model":z,"grip_height_mm":grip,"support_width_mm":bar,"contacts":contacts,"supports":supports,"contact_patches_model":contact_model,"material_below_seat_mm":r(hi[2]-z)});
                if best.as_ref().is_none_or(|(old, _)| score > *old + 1e-6) {
                    best = Some((score, seat));
                }
                // Use the least grip that satisfies the contact rule at this seat.
                break;
            }
        }
    }
    best.map(|(_,s)|s).ok_or_else(|| if contact_ok {
        format!("no horizontal support pair reaches {}% contact within {} mm inboard, without interfering material",100.*support_fraction,inset)
    } else {format!("opposed finished side contact below rule: best minimum fraction {}, required {} and {} mm effective length per side",r(best_contact),fraction,bar)})
}

/// A cut is modelled only when all its original geometric inputs still agree.
/// New or altered cuts continue through the conservative grip audit.
pub(crate) fn modeled_cut(seat: &Value, op: &Operation) -> bool {
    // Ordinary generated spots fit entirely inside their modelled parent bore.
    // Enlarged countersinks and altered/unrecognised spots retain the audit.
    if let Cut::SpotDrill(drill) = &op.cut {
        let eps = crate::rules::workholding()["flip_frame_tolerance_mm"]
            .as_f64()
            .unwrap();
        return !drill.spots.is_empty()
            && drill.spots.iter().all(|spot| {
                seat["modeled_cuts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|hole| {
                        hole["kind"] == "holes"
                            && hole["centers"]
                                .as_array()
                                .is_some_and(|centers| centers.contains(&json!(spot.center)))
                            && !spot.features.is_empty()
                            && spot.features.iter().all(|f| {
                                hole["features"]
                                    .as_array()
                                    .is_some_and(|fs| fs.iter().any(|x| x == f))
                            })
                            && hole["diameter"]
                                .as_f64()
                                .is_some_and(|h| spot.diameter <= h)
                            && hole["top_z"]
                                .as_f64()
                                .is_some_and(|h| (h - spot.top_z).abs() <= eps)
                            && spot
                                .depth
                                .zip(hole["bottom_z"].as_f64())
                                .is_some_and(|(depth, bottom)| spot.top_z - depth >= bottom)
                    })
            });
    }
    let op = json!(op);
    seat["modeled_cuts"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|cut| {
            cut.as_object()
                .is_some_and(|fields| fields.iter().all(|(k, v)| op.get(k) == Some(v)))
        })
}

/// Reset exclusions when switching plans: operation indices belong to one plan.
pub fn fallback(analysis: &Value, mesh: &Value, reason: &str) -> Result<Value, String> {
    serde_json::to_value(build_fallback(analysis, mesh, reason)?).map_err(|e| e.to_string())
}

/// [`fallback`], as types.
pub fn build_fallback(analysis: &Value, mesh: &Value, reason: &str) -> Result<Plan, String> {
    let mut p = crate::plan::build(analysis, Some(mesh), &Options::default())?;
    let mut trace = Trace::take_plan(&mut p);
    trace.decide(
        RULE,
        "part",
        "none",
        &format!("vise flip option withheld; replanned without sacrificial bottom stock: {reason}"),
        json!({"chosen":{"option":"vise_flip_face","selected":true,"available":false}}),
    );
    trace.store_plan(&mut p);
    Ok(p)
}

/// Resource-aware entry point. Always trace availability, but only build the
/// flip when overrides.workholding_option explicitly selects vise_flip_face.
pub fn plan(analysis: &Value, mesh: &Value, resources: &Value) -> Result<Value, String> {
    serde_json::to_value(build(analysis, mesh, resources)?).map_err(|e| e.to_string())
}

/// [`plan`], as types.
pub fn build(analysis: &Value, mesh: &Value, resources: &Value) -> Result<Plan, String> {
    let requested = crate::resources::workholding_option(resources)?;
    let selected = requested == Some("vise_flip_face");
    let defaults = Options::default();
    let original = crate::plan::build(analysis, Some(mesh), &defaults)?;
    let availability: Result<(f64, Value, Point, Point), String> = (|| {
        let vise = crate::machines::dimensioned_vise(resources)
            .ok_or("a mill with a dimensioned vise is required (jaw width, height and opening)")?;
        if resources["overrides"]["clamping"]
            .as_object()
            .is_some_and(|p| !p.is_empty())
        {
            return Err(
                "explicit clamping proposals retain their original stock and setup contract".into(),
            );
        }
        let eligible = original
            .setups
            .iter()
            .flat_map(|s| &s.operations)
            .any(|o| match &o.cut {
                Cut::Roughing(c) => c.web_mm > 0.,
                Cut::Holes(c) => c.bottom == "through",
                Cut::Slot(c) => c.bottom == "through",
                Cut::Notch(c) => c.bottom == "through",
                _ => false,
            });
        if !eligible {
            return Err(
                "the part has no planned web or through features requiring flip-and-face".into(),
            );
        }
        let lo: Point = serde_json::from_value(mesh["bbox"][0].clone())
            .map_err(|e| format!("flip needs mesh bounds: {e}"))?;
        let hi: Point = serde_json::from_value(mesh["bbox"][1].clone())
            .map_err(|e| format!("flip needs mesh bounds: {e}"))?;
        if !original.setups.iter().any(|s| {
            s.id == "top" && s.operations.iter().any(|o| matches!(o.cut, Cut::Roughing(_)))
        })
        {
            return Err("no valid top roughing surface to prepare the finished grip".into());
        }

        let b = bottom_stock(
            hi[2] - lo[2],
            defaults.top_stock,
            vise["parallels_height_mm"].as_f64(),
            vise["jaw_height_mm"].as_f64().unwrap(),
        )?;
        let seat = seating(mesh, lo, hi, vise)?;
        let axis = seat["axis"].as_u64().unwrap() as usize;
        if hi[axis] - lo[axis] > vise["max_jaw_opening_mm"].as_f64().unwrap() {
            return Err("finished side pair exceeds vise opening".into());
        }
        let library = crate::resources::Library::new(resources)?;
        let facing = library
            .clearing("facing")?
            .ok_or("no facing tool for Op 2")?;
        if crate::resources::num(facing, "diameter")? / 2. <= defaults.margin {
            return Err("facing cutter radius cannot overlap the residual Op 1 margin".into());
        }
        let tool = library
            .clearing("roughing")?
            .ok_or("no roughing tool to finish the grip faces")?;
        if hi[2] - lo[2] + defaults.top_stock
            > crate::resources::num(tool, "flute_length")?
                .min(crate::resources::num(tool, "stickout")?)
                - 1.
        {
            return Err("Op 1 tool cannot reach the whole outline; camlib#121 / #45 box stock cannot carry the uncut margin for Op 2 finishing, so the flip option is withheld".into());
        }
        Ok((b, seat, lo, hi))
    })();
    if !selected || availability.is_err() {
        let available = availability.is_ok();
        let reason=match &availability {
            Ok(_) => "vise flip-and-face option is available but was not selected; selection belongs to the caller (setup.workholding compares simulated runs)".to_string(),
            Err(reason) if selected => format!("selected vise flip-and-face option is unavailable: {reason}"),
            Err(reason) => format!("vise flip-and-face option was not selected and is unavailable: {reason}"),
        };
        let mut p = original;
        let mut trace = Trace::take_plan(&mut p);
        trace.decide(RULE,"part","none",&reason,json!({"chosen":{"option":"vise_flip_face","selected":selected,"requested":requested,"available":available,"unavailable_reason":availability.as_ref().err()}}));
        trace.store_plan(&mut p);
        return Ok(p);
    }
    let (b, mut seat, lo, hi) = availability?;
    let options = Options {
        bottom_stock: b,
        skin: 0.,
        ..defaults
    };
    let mut p = crate::plan::build(analysis, Some(mesh), &options)?;
    let mut trace = Trace::take_plan(&mut p);
    let chosen = trace.decide(RULE,"part","chosen", "selected vise flip-and-face candidate: sacrifice bottom stock for Op 1 grip, side-cut clearance and facing allowance; through exits break into that stock and are finished by Op 2 facing; no outline remains below Op 1 reach",json!({"chosen":{"option":"vise_flip_face","selected":true,"available":true,"bottom_stock_mm":b,"seating":seat}}));
    let warning = trace.warn(RULE,"bottom",format!("bottom setup requests the preceding finished stock; simulation records the carried-stock premise or an explicit freed-block assumption. Facing covers the residual margin. Edge supports seat at model Z{}, width {} mm, offsets {}; measured side contact {}. Holding strength is not simulated{}", seat["seat_z_model"],seat["support_width_mm"],seat["supports"],seat["contacts"],if seat["material_below_seat_mm"].as_f64().unwrap_or(0.)>0. {format!("; the conservative height-field stock can fill voids within the {} mm of shaped material below the seat in the flipped frame",seat["material_below_seat_mm"])}else{String::new()}), json!({}));
    let setups = &mut p.setups;
    let top = setups.iter_mut().find(|s| s.id == "top").unwrap();
    top.flip_role = Some("sacrificial_grip".into());
    seat["modeled_cuts"] = json!(
        top.operations
            .iter()
            .filter(|op| !matches!(
                op.cut,
                Cut::Roughing(_) | Cut::Flats { .. } | Cut::Facing { .. } | Cut::WallFinish(_)
            ))
            .map(|op| {
                let mut cut = json!(op);
                cut.as_object_mut().unwrap().remove("because");
                cut
            })
            .collect::<Vec<_>>()
    );
    for op in &mut top.operations {
        because_op(op, [chosen.clone()]);
    }
    // Preserve genuine bottom-entered features and their original part-zero frame.
    let index = setups.iter().position(|s| s.id == "bottom");
    let mut bottom = index.map(|i| setups.remove(i)).unwrap_or_else(|| Setup {
        id: "bottom".into(),
        zero: Some("part".into()),
        rotation: Some([[1, 0, 0], [0, -1, 0], [0, 0, -1]]),
        origin_model: Some([lo[0], hi[1], lo[2]]),
        stock: Box3 { min: [0., 0., r(lo[2] - hi[2])], max: [r(hi[0] - lo[0]), r(hi[1] - lo[1]), 0.] },
        operations: vec![],
        machine: None,
        stock_note: None,
        flip_role: None,
        part_surface: None,
        clamping: None,
        flip_seating: None,
        before_setup: None,
        grip_surface_model: None,
    });
    bottom.stock.max[2] = b;
    // Both jobs carry the complete target surface for camlib's material check.
    if let Some(rough) = setups[0].operations.iter().find(|o| matches!(o.cut, Cut::Roughing(_))) {
        let mut surface = rough.clone();
        let positions: Vec<_> = mesh["positions"]
            .as_array()
            .unwrap()
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| {
                [
                    r(p[0].as_f64().unwrap() - lo[0]),
                    r(hi[1] - p[1].as_f64().unwrap()),
                    r(lo[2] - p[2].as_f64().unwrap()),
                ]
            })
            .collect();
        if let Cut::Roughing(c) = &mut surface.cut {
            c.positions = Some(positions);
        }
        bottom.part_surface = Some(Box::new(surface));
    }
    seat["residual_margin_mm"] = json!(options.margin);
    bottom.flip_seating = Some(seat);
    let faced = trace.decide(
        "process.facing",
        "bottom:facing",
        "planned",
        "face sacrificial bottom stock to the part bottom at thickness",
        Value::Null,
    );
    let stock = trace.decide("setup.stock.part_box_with_margins", "bottom", "chosen", "bottom setup zeroed on the part bottom; request the preceding finished stock, with part box plus sacrificial stock only as an explicit fallback", json!({"zero":"part","stock":bottom.stock}));
    let order = trace.decide(
        "sequence.fixed_order",
        "bottom",
        "chosen",
        "face to thickness before genuine bottom-entered features",
        Value::Null,
    );
    bottom.operations.insert(
        0,
        Operation {
            cut: Cut::Facing { floor_z: 0. },
            tool: None,
            tool_note: None,
            because: Some(vec![chosen, warning, faced, stock, order]),
            iso14649: None,
            technology: None,
        },
    );
    setups.insert(1, bottom);
    trace.store_plan(&mut p);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_spots_inside_a_modeled_parent_bore_share_its_contact_measurement() {
        let seat = json!({"modeled_cuts":[{"kind":"holes","centers":[[2.,3.]],"features":["hole:1"],"top_z":-1.,"bottom_z":-9.,"diameter":4.}]});
        let mut spot: Operation = serde_json::from_value(json!({"kind":"spot_drill","spot_tool":null,"diameter":4.,"angle":118.,"reach":1.,"spots":[{"center":[2.,3.],"features":["hole:1"],"top_z":-1.,"depth":1.,"diameter":4.,"countersink":false}]})).unwrap();
        assert!(modeled_cut(&seat, &spot));
        if let Cut::SpotDrill(drill) = &mut spot.cut { drill.spots[0].diameter = 5.; }
        assert!(!modeled_cut(&seat, &spot));
        if let Cut::SpotDrill(drill) = &mut spot.cut { drill.spots[0].diameter = 4.; drill.spots[0].depth = Some(10.); }
        assert!(!modeled_cut(&seat, &spot));
    }
    fn interrupted_sides(hole_width: f64, hole_bottom: f64, top: bool) -> Value {
        let mut tris: Vec<Tri> = vec![];
        let mut quad = |a: Point, b: Point, c: Point, d: Point| {
            tris.push([a, b, c]);
            tris.push([a, c, d]);
        };
        for y in [0., 20.] {
            let a = 20. - hole_width / 2.;
            let b = 20. + hole_width / 2.;
            for (x0, x1, z0, z1) in [
                (0., a, 0., 10.),
                (b, 40., 0., 10.),
                (a, b, 0., hole_bottom),
                (a, b, 9., 10.),
            ] {
                quad([x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]);
            }
        }
        if top {
            quad(
                [0., 0., 10.],
                [40., 0., 10.],
                [40., 20., 10.],
                [0., 20., 10.],
            );
        }
        json!({"positions":tris.iter().flatten().flatten().copied().collect::<Vec<_>>(),"indices":(0..tris.len()*3).collect::<Vec<_>>()})
    }
    #[test]
    fn holes_reduce_measured_contact_but_do_not_require_continuous_faces() {
        let vise = json!({"jaw_width_mm":100.,"jaw_height_mm":20.,"max_jaw_opening_mm":150.});
        let seat = seating(
            &interrupted_sides(4., 7., true),
            [0., 0., 0.],
            [40., 20., 10.],
            &vise,
        )
        .unwrap();
        assert_eq!(seat["axis"], 1);
        for c in seat["contacts"].as_array().unwrap() {
            let f = c["fraction"].as_f64().unwrap();
            assert!(f > 0.9 && f < 1.);
            assert!((c["area_mm2"].as_f64().unwrap() - (40. * 3.175 - 8.)).abs() < 1e-4);
        }
        let deeper = seating(
            &interrupted_sides(30., 7., true),
            [0., 0., 0.],
            [40., 20., 10.],
            &vise,
        )
        .unwrap();
        assert!(deeper["grip_height_mm"].as_f64().unwrap() > 3.175);
        assert!(deeper["grip_height_mm"].as_f64().unwrap() <= 5.);
        assert!(
            deeper["contacts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["fraction"].as_f64().unwrap() >= 0.6)
        );
        let err = seating(
            &interrupted_sides(40., 5., true),
            [0., 0., 0.],
            [40., 20., 10.],
            &vise,
        )
        .unwrap_err();
        assert!(err.contains("contact below rule"), "{err}");
        assert!(
            seating(
                &interrupted_sides(4., 7., false),
                [0., 0., 0.],
                [40., 20., 10.],
                &vise
            )
            .unwrap_err()
            .contains("no horizontal support face")
        );
    }
    #[test]
    fn a_malformed_mesh_is_an_error_not_a_panic() {
        let vise = json!({"jaw_width_mm":100.,"jaw_height_mm":20.,"max_jaw_opening_mm":150.});
        let mut mesh = interrupted_sides(4., 7., true);
        mesh["indices"][0] = json!(1_000_000);
        let err = seating(&mesh, [0., 0., 0.], [40., 20., 10.], &vise).unwrap_err();
        assert!(err.contains("complete mesh"), "{err}");
    }
    #[test]
    fn no_vise_or_missing_dimensions_traces_unavailability_without_changing_the_plan() {
        let a: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/plate.analysis.json"))
                .unwrap();
        let m: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/plate.mesh.json")).unwrap();
        let original = crate::plan::plan(&a, Some(&m), &Options::default()).unwrap();
        for resources in [
            json!({}),
            json!({"machines":[{"kind":"mill","workholding":[{"kind":"vise","max_jaw_opening_mm":150}]}]}),
        ] {
            for selected in [false, true] {
                let mut resources = resources.clone();
                if selected {
                    resources["overrides"]["workholding_option"] = json!("vise_flip_face");
                }
                let mut p = plan(&a, &m, &resources).unwrap();
                let decision = p["decisions"].as_array_mut().unwrap().pop().unwrap();
                assert_eq!(decision["chosen"]["selected"], selected);
                assert_eq!(decision["chosen"]["available"], false);
                assert!(
                    decision["says"]
                        .as_str()
                        .unwrap()
                        .contains("dimensioned vise")
                );
                assert_eq!(p, original);
            }
        }
    }
    #[test]
    fn stock_solves_the_height_dependent_grip() {
        let b = bottom_stock(18., 1., None, 20.).unwrap();
        assert_eq!(b, 7.25);
        assert_eq!(b, 0.2 * (19. + b) + 2.);
        assert_eq!(bottom_stock(2., 1., None, 20.).unwrap(), 5.175);
        assert_eq!(bottom_stock(18., 1., Some(14.), 20.).unwrap(), 8.);
        assert!(bottom_stock(18., 1., Some(19.), 20.).is_err());
        assert!(bottom_stock(100., 1., None, 20.).is_err());
    }
}
