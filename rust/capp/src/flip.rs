//! The vise flip-and-face option (#86). Callers can offer or withhold this function
//! independently of machine-independent planning (#94); no supplied decision protocol.
use crate::{
    plan::{Options, r},
    trace::{Trace, because},
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

fn triangles(mesh: &Value) -> Option<Vec<Tri>> {
    let p = mesh["positions"].as_array()?;
    mesh["indices"]
        .as_array()?
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let mut tri = [[0.; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    tri[i][j] = p[t[i].as_u64()? as usize * 3 + j].as_f64()?;
                }
            }
            Some(tri)
        })
        .collect()
}
fn area(p: &[[f64; 2]]) -> f64 {
    (0..p.len())
        .map(|i| {
            let q = p[(i + 1) % p.len()];
            p[i][0] * q[1] - q[0] * p[i][1]
        })
        .sum::<f64>()
        .abs()
        / 2.
}
/// Exact polygon clipping of mesh triangles to a support rectangle: holes and
/// missing corners reduce the covered area, rather than becoming box material.
fn covered(tris: &[Tri], axes: [usize; 2], rect: [[f64; 2]; 2]) -> bool {
    let mut sum = 0.;
    for tri in tris {
        let mut poly: Vec<_> = tri.iter().map(|p| [p[axes[0]], p[axes[1]]]).collect();
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
        sum += area(&poly);
    }
    let want = (rect[1][0] - rect[0][0]) * (rect[1][1] - rect[0][1]);
    want > 0. && (sum - want).abs() < 1e-6 * want.max(1.)
}

/// Mesh-backed contact and support. Prefer the narrower axis, but try the other
/// pair when a notch splits a long side. Require rectangular opposed face bands
/// and continuous, horizontal support strips; do not infer seating from the box.
fn seating(mesh: &Value, lo: Point, hi: Point, vise: &Value) -> Result<Value, String> {
    let width = vise["jaw_width_mm"].as_f64().unwrap();
    let tris = triangles(mesh).ok_or("flip seating needs a complete mesh")?;
    let rule = crate::rules::workholding();
    let bar = rule["min_grip_mm"].as_f64().unwrap();
    let preferred = usize::from(hi[0] - lo[0] > hi[1] - lo[1]);
    for axis in [preferred, 1 - preferred] {
        let along = 1 - axis;
        // Candidate seating levels come from actual horizontal mesh triangles.
        let mut levels: Vec<_> = tris
            .iter()
            .filter(|t| t.iter().all(|p| (p[2] - t[0][2]).abs() < 1e-6))
            .map(|t| t[0][2])
            .collect();
        levels.sort_by(|a, b| b.total_cmp(a));
        levels.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        for z in levels {
            let minimum = rule["min_grip_mm"]
                .as_f64()
                .unwrap()
                .max((z - lo[2]) * rule["min_grip_fraction"].as_f64().unwrap());
            let grip = r(vise["parallels_height_mm"]
                .as_f64()
                .map_or(minimum, |p| vise["jaw_height_mm"].as_f64().unwrap() - p));
            if grip < r(minimum)
                || grip > vise["jaw_height_mm"].as_f64().unwrap()
                || grip > r((z - lo[2]) * rule["max_grip_fraction"].as_f64().unwrap())
            {
                continue;
            }
            if z - lo[2] < grip + rule["clearance_mm"].as_f64().unwrap() {
                continue;
            }
            // Restrict contact to the central jaw width. Side-plane coverage
            // rejects notches, holes and round corners inside this band.
            let side_tris: Vec<Vec<Tri>> = [lo[axis], hi[axis]]
                .iter()
                .map(|edge| {
                    tris.iter()
                        .filter(|t| t.iter().all(|p| (p[axis] - edge).abs() < 1e-6))
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
            let band = [[start, z - grip], [end, z]];
            if !side_tris.iter().all(|ts| covered(ts, [along, 2], band)) {
                continue;
            }
            let horizontal: Vec<_> = tris
                .iter()
                .filter(|t| t.iter().all(|p| (p[2] - z).abs() < 1e-6))
                .copied()
                .collect();
            let strips: Vec<_> = [lo[axis], hi[axis] - bar]
                .iter()
                .map(|edge| {
                    let mut rect = [[0.; 2]; 2];
                    rect[0][axis] = *edge;
                    rect[1][axis] = edge + bar;
                    rect[0][along] = start;
                    rect[1][along] = end;
                    rect
                })
                .collect();
            if strips
                .iter()
                .all(|rect| covered(&horizontal, [0, 1], *rect))
            {
                return Ok(
                    json!({"axis": axis, "along_min_model": start, "along_max_model": end, "seat_z_model": z, "grip_height_mm": grip, "support_width_mm": bar}),
                );
            }
        }
    }
    Err("no opposed continuous side bands with horizontal support strips clear of bosses, holes and notches".into())
}

/// Resource-aware entry point. Without a dimensioned vise this returns exactly
/// machine-independent plan(defaults), including its original trace.
pub fn plan(analysis: &Value, mesh: &Value, resources: &Value) -> Result<Value, String> {
    let defaults = Options::default();
    let original = crate::plan::plan(analysis, Some(mesh), &defaults)?;
    let vise = crate::machines::mill(resources).and_then(|m| {
        m["workholding"]
            .as_array()?
            .iter()
            .find(|w| w["kind"] == "vise")
    });
    let Some(vise) = vise.filter(|v| {
        ["jaw_width_mm", "jaw_height_mm", "max_jaw_opening_mm"]
            .iter()
            .all(|k| v[*k].as_f64().is_some_and(|n| n.is_finite() && n > 0.))
    }) else {
        return Ok(original);
    };
    if resources["overrides"]["clamping"]
        .as_object()
        .is_some_and(|p| !p.is_empty())
    {
        let mut p = original;
        let mut trace = Trace::take(&mut p);
        trace.decide(
            RULE,
            "part",
            "none",
            "explicit clamping proposals retain their original stock and setup contract",
            json!({"chosen":{"option":"vise_flip_face","available":false}}),
        );
        trace.store(&mut p);
        return Ok(p);
    }
    let eligible = original["setups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["operations"].as_array().into_iter().flatten())
        .any(|o| o["web_mm"].as_f64().is_some_and(|w| w > 0.) || o["bottom"] == "through");
    if !eligible {
        return Ok(original);
    }
    let lo: Point =
        serde_json::from_value(original["part"]["min"].clone()).map_err(|e| e.to_string())?;
    let hi: Point =
        serde_json::from_value(original["part"]["max"].clone()).map_err(|e| e.to_string())?;
    let selected: Result<(f64, Value), String> = (|| {
        if !original["setups"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| {
                s["id"] == "top"
                    && s["operations"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|o| o["kind"] == "roughing")
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
            return Err("Op 1 tool cannot reach the whole outline; flip option withheld".into());
        }
        Ok((b, seat))
    })();
    let (b, seat) = match selected {
        Ok(v) => v,
        Err(reason) => {
            let mut p = original;
            let mut trace = Trace::take(&mut p);
            trace.decide(
                RULE,
                "part",
                "none",
                &reason,
                json!({"chosen":{"option":"vise_flip_face","available":false}}),
            );
            trace.store(&mut p);
            return Ok(p);
        }
    };
    let options = Options {
        bottom_stock: b,
        skin: 0.,
        ..defaults
    };
    let mut p = crate::plan::plan(analysis, Some(mesh), &options)?;
    let mut trace = Trace::take(&mut p);
    let chosen = trace.decide(RULE,"part","chosen", "vise flip-and-face option enabled: sacrifice bottom stock for Op 1 grip, side-cut clearance and facing allowance; through exits break into that stock and are finished by Op 2 facing; no outline remains below Op 1 reach",json!({"chosen":{"option":"vise_flip_face","available":true,"bottom_stock_mm":b,"seating":seat}}));
    let warning = trace.warn(RULE,"bottom","bottom setup starts from the part box plus bottom stock (camlib#121 / #45): it does not model Op 1's residual margin below the jaw line or the true boss-shaped underside. Facing must clear that residual margin; modeled edge supports seat on the finished plate top clear of the boss, not on the box bottom. Seating and contact are mesh checked; stock history and holding strength are not simulated".into());
    let setups = p["setups"].as_array_mut().unwrap();
    let top = setups.iter_mut().find(|s| s["id"] == "top").unwrap();
    top["flip_role"] = json!("sacrificial_grip");
    for op in top["operations"].as_array_mut().unwrap() {
        because(op, [chosen.clone()]);
    }
    // Preserve genuine bottom-entered features and their original part-zero frame.
    let index = setups.iter().position(|s| s["id"] == "bottom");
    let mut bottom = index.map(|i| setups.remove(i)).unwrap_or_else(|| json!({"id":"bottom","zero":"part","rotation":[[1,0,0],[0,-1,0],[0,0,-1]],"origin_model":[lo[0],hi[1],lo[2]],"stock":{"min":[0.,0.,r(lo[2]-hi[2])],"max":[r(hi[0]-lo[0]),r(hi[1]-lo[1]),0.]},"operations":[]}));
    bottom["stock"]["max"][2] = json!(b);
    // Both jobs carry the complete target surface for camlib's material check.
    if let Some(rough) = setups[0]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["kind"] == "roughing")
    {
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
        surface["positions"] = json!(positions);
        bottom["part_surface"] = surface;
    }
    bottom["flip_seating"] = seat;
    bottom["flip_seating"]["residual_margin_mm"] = json!(options.margin);
    let faced = trace.decide(
        "process.facing",
        "bottom:facing",
        "planned",
        "face sacrificial bottom stock to the part bottom at thickness",
        Value::Null,
    );
    let stock = trace.decide("setup.stock.part_box_with_margins", "bottom", "chosen", "bottom setup zeroed on the part bottom; part box plus sacrificial stock, not Op 1 stock history", json!({"zero":"part","stock":bottom["stock"]}));
    let order = trace.decide(
        "sequence.fixed_order",
        "bottom",
        "chosen",
        "face to thickness before genuine bottom-entered features",
        Value::Null,
    );
    bottom["operations"].as_array_mut().unwrap().insert(
        0,
        json!({"kind":"facing","floor_z":0.,"because":[chosen,warning,faced,stock,order]}),
    );
    setups.insert(1, bottom);
    trace.store(&mut p);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_vise_or_missing_dimensions_is_exactly_the_original_plan() {
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
            assert_eq!(plan(&a, &m, &resources).unwrap(), original);
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
