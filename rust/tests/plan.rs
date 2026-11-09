//! The plan from an analysis (and mesh): what tests/test_step2cam.py checked in its `Rotations` and
//! `Plan` classes, under the same names. The thread tests are in threads.rs.
mod common;
use common::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use capp::plan::{Options, plan};

fn plate() -> Value {
    fixture("plate.analysis.json")
}
fn plate_mesh() -> Value {
    fixture("plate.mesh.json")
}
fn operations(plan: &Value, setup: usize) -> &Vec<Value> {
    list(&plan["setups"][setup]["operations"])
}
fn of_kind<'a>(plan: &'a Value, setup: usize, kind: &str) -> Vec<&'a Value> {
    operations(plan, setup).iter().filter(|o| o["kind"] == kind).collect()
}
fn ids(plan: &Value) -> Vec<&str> {
    list(&plan["setups"]).iter().map(|s| s["id"].as_str().unwrap()).collect()
}
fn kinds(plan: &Value, setup: usize) -> Vec<&str> {
    operations(plan, setup).iter().map(|o| o["kind"].as_str().unwrap()).collect()
}
fn roughing(plan: &Value) -> &Value {
    of_kind(plan, 0, "roughing")[0]
}
fn no_mesh(analysis: &Value, options: &Options) -> Value {
    plan(analysis, None, options).unwrap()
}
/// (diameter, top, floor) of each counterbore, widest first.
fn counterbores(plan: &Value) -> Value {
    let mut found: Vec<[f64; 3]> = of_kind(plan, 0, "counterbore").iter().map(|o| [number(&o["diameter"]), number(&o["top_z"]), number(&o["floor_z"])]).collect();
    found.sort_by(|a, b| b[0].total_cmp(&a[0]));
    json!(found)
}

#[test]
fn every_setup_is_a_proper_rotation_taking_its_axis_down() {
    // A blind hole along each direction brings that setup, and its rotation, into the plan.
    let table: [(&str, [i64; 3]); 6] = [("top", [0, 0, -1]), ("bottom", [0, 0, 1]), ("front", [0, 1, 0]), ("back", [0, -1, 0]), ("left", [1, 0, 0]), ("right", [-1, 0, 0])];
    let holes: Vec<Value> = table.iter().map(|(_, a)| hole(a.map(|x| x as f64), [0., 0., 0.], 3., 2., "flat")).collect();
    let plan = no_mesh(&holes_only(&holes), &Default::default());
    assert_eq!(ids(&plan), table.map(|t| t.0));
    let dot = |a: [i64; 3], b: [i64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    for (setup, (name, axis)) in list(&plan["setups"]).iter().zip(table) {
        let rows: Vec<[i64; 3]> = list(&setup["rotation"]).iter().map(|row| {
            let row = list(row);
            assert_eq!(row.len(), 3, "{name}");
            [0, 1, 2].map(|i| row[i].as_i64().unwrap())
        }).collect();
        let ([a, b, c], [d, e, f], [g, h, i]) = (rows[0], rows[1], rows[2]);
        assert_eq!(a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g), 1, "{name}");
        assert_eq!([dot(rows[0], axis), dot(rows[1], axis), dot(rows[2], axis)], [0, 0, -1], "{name}");
        for i in 0..3 {
            for j in 0..3 {
                assert_eq!(dot(rows[i], rows[j]), (i == j) as i64, "{name}");
            }
        }
    }
}

#[test]
fn plate_from_the_top() {
    let plan = no_mesh(&plate(), &Default::default());
    assert_eq!(plan["format"], "specify-cam-plan");
    assert_eq!(ids(&plan), ["top"]);
    let top = &plan["setups"][0];
    // Part 100 x 60 x 10, 2 mm margin, 1 mm on top: Z0 is the stock top.
    assert_same(top["stock"].clone(), json!({"min": [0, 0, -11], "max": [104, 64, 0]}));
    assert_eq!(kinds(&plan, 0), ["facing", "holes", "holes", "holes"]);
    assert_eq!(number(&top["operations"][0]["floor_z"]), -1.);
    let holes = of_kind(&plan, 0, "holes");
    let by_d: HashMap<String, &&Value> = holes.iter().map(|o| (o["diameter"].to_string(), o)).collect();
    let mut diameters: Vec<f64> = holes.iter().map(|o| number(&o["diameter"])).collect();
    diameters.sort_by(f64::total_cmp);
    assert_eq!(diameters, [4.2, 6.6, 8.0]);
    assert_eq!(list(&by_d["6.6"]["centers"]).len(), 6);
    for op in by_d.values() {
        assert_same(json!([op["top_z"], op["bottom_z"], op["bottom"]]), json!([-1, -11, "through"]));
    }
    assert_same(by_d["8.0"]["centers"].clone(), json!([[52, 32]]));
    assert_eq!(plan["skipped"], json!([]));
    assert_eq!(top["zero"], "stock");
}

#[test]
fn the_mesh_adds_roughing_in_the_setup_frame() {
    let with_mesh = plan(&plate(), Some(&plate_mesh()), &Default::default()).unwrap();
    let top = &with_mesh["setups"][0];
    // The mesh's exact box and the analysis's frame-carried box agree.
    assert_same(top["stock"].clone(), no_mesh(&plate(), &Default::default())["setups"][0]["stock"].clone());
    let rough = roughing(&with_mesh);
    assert_same(json!([rough["top_z"], rough["floor_z"]]), json!([0, -10.5])); // 0.5 mm skin
    let p: Vec<f64> = list(&rough["positions"]).iter().map(number).collect();
    let (xs, zs): (Vec<f64>, Vec<f64>) = p.chunks_exact(3).map(|c| (c[0], c[2])).unzip();
    assert_close(zs.iter().copied().fold(f64::MIN, f64::max), -1.);
    assert_close(zs.iter().copied().fold(f64::MAX, f64::min), -11.);
    assert_close(xs.iter().copied().fold(f64::MAX, f64::min), 2.);
    assert_eq!(list(&rough["indices"]).len() % 3, 0);
}

#[test]
fn a_mesh_of_another_part_is_refused() {
    let mut mesh = plate_mesh();
    mesh["binding"]["source_sha256"] = json!("0".repeat(64));
    assert!(plan(&plate(), Some(&mesh), &Default::default()).is_err());
}

#[test]
fn a_mesh_with_untriangulated_faces_is_not_roughed() {
    let mut mesh = plate_mesh();
    mesh["missing"] = json!([3]);
    let plan = plan(&plate(), Some(&mesh), &Default::default()).unwrap();
    assert!(!kinds(&plan, 0).contains(&"roughing"));
    assert_eq!(plan["skipped"][0]["feature"], "roughing");
}

#[test]
fn side_holes_get_their_own_setup_inside_the_stock() {
    let plan = no_mesh(&fixture("spindle.analysis.json"), &Default::default());
    let names = ids(&plan);
    assert_eq!(names[0], "top");
    assert!(names.len() > 1);
    for setup in list(&plan["setups"]) {
        let (lo, hi) = (&setup["stock"]["min"], &setup["stock"]["max"]);
        assert_eq!(number(&hi[2]), 0.);
        for op in list(&setup["operations"]) {
            for c in op["centers"].as_array().into_iter().flatten() {
                let (x, y) = (number(&c[0]), number(&c[1]));
                assert!(number(&lo[0]) <= x && x <= number(&hi[0]) && number(&lo[1]) <= y && y <= number(&hi[1]), "{} {x} {y}", setup["id"]);
            }
            if op["kind"] == "holes" {
                assert!(number(&lo[2]) <= number(&op["bottom_z"]) && number(&op["bottom_z"]) < number(&op["top_z"]) && number(&op["top_z"]) <= 0., "{op}");
            }
        }
    }
    assert!(list(&plan["skipped"]).iter().any(|s| s["reason"] == "turned feature: make it on a lathe"));
}

#[test]
fn a_through_hole_is_entered_from_the_preferred_side() {
    // quiddity may present a through hole from either mouth: here from the bottom.
    let a = holes_only(&[hole([0., 0., 1.], [5., 0., -10.], 6., 10., "through")]);
    let plan = no_mesh(&a, &top_stock(0.));
    assert_eq!(ids(&plan), ["top"]);
    let op = &plan["setups"][0]["operations"][0];
    assert_same(json!([op["top_z"], op["bottom_z"]]), json!([0, -10]));
    assert_same(op["centers"].clone(), json!([[27, 22]]));
}

#[test]
fn a_blind_hole_from_below_needs_the_bottom_setup_zeroed_on_the_part() {
    let a = holes_only(&[hole([0., 0., 1.], [5., 3., -10.], 5., 4., "flat")]);
    let plan = no_mesh(&a, &top_stock(1.));
    let bottom = list(&plan["setups"]).iter().find(|s| s["id"] == "bottom").unwrap();
    // By the time the part is turned over, the stock around it has been roughed away: the
    // setup is zeroed on the part's own box, not on the stock's.
    assert_eq!(bottom["zero"], "part");
    assert_same(bottom["stock"].clone(), json!({"min": [0, 0, -10], "max": [40, 40, 0]}));
    let op = &bottom["operations"][0];
    assert_same(json!([op["top_z"], op["bottom_z"], op["bottom"]]), json!([0, -4, "flat"]));
    // Turned over about X: model y=3 lands at y = 20 - 3.
    assert_same(op["centers"].clone(), json!([[25, 17]]));
    // The origin recorded for the bottom setup maps back to the model point.
    assert_same(bottom["origin_model"].clone(), json!([-20, 20, -10]));
}

#[test]
fn counterbores_become_their_own_operation_above_the_bore() {
    let cbore = json!({"diameter": 11, "depth": 3.5});
    let a = holes_only(&[
        with(down(0., 0., 6.6, 6.5, "through"), "cbore", cbore.clone()),
        with(down(10., 0., 6.6, 6.5, "through"), "cbore", cbore),
    ]);
    let plan = no_mesh(&a, &top_stock(0.));
    let (holes, cbores) = (of_kind(&plan, 0, "holes"), of_kind(&plan, 0, "counterbore"));
    assert_eq!(holes.len(), 1);
    assert_same(json!([holes[0]["bore_top_z"], holes[0]["bottom_z"]]), json!([-3.5, -10]));
    assert_eq!(cbores.len(), 1);
    assert_same(json!([cbores[0]["diameter"], cbores[0]["top_z"], cbores[0]["floor_z"]]), json!([11, 0, -3.5]));
    assert_eq!(list(&cbores[0]["centers"]).len(), 2);
}

#[test]
fn a_spotface_is_the_outermost_step_above_a_counterbore() {
    // quiddity: steps narrow inwards; the shallow spotface is usually the outermost.
    let record = with(with(down(0., 0., 6.6, 3.5, "through"), "cbore", json!({"diameter": 11, "depth": 6})), "spotface", json!({"diameter": 20, "depth": 0.5}));
    let plan = no_mesh(&holes_only(&[record]), &top_stock(0.));
    assert_same(counterbores(&plan), json!([[20, 0, -0.5], [11, -0.5, -6.5]]));
    let holes = of_kind(&plan, 0, "holes")[0];
    assert_same(json!([holes["bore_top_z"], holes["bottom_z"]]), json!([-6.5, -10]));
}

#[test]
fn outer_steps_come_before_the_counterbore() {
    let record = with(with(down(0., 0., 5., 5., "through"), "cbore", json!({"diameter": 9, "depth": 3})), "outer_steps", json!([{"diameter": 16, "depth": 2}]));
    let plan = no_mesh(&holes_only(&[record]), &top_stock(0.));
    assert_same(counterbores(&plan), json!([[16, 0, -2], [9, -2, -5]]));
}

#[test]
fn a_through_hole_with_a_capped_exit_is_not_entered_from_it() {
    let record = with(hole([0., 0., 1.], [5., 0., -10.], 6., 10., "through"), "exit_obstructions", json!([{"location": [5, 0, 0], "faces": [3], "covered_fraction": 0.4}]));
    let plan = no_mesh(&holes_only(&[record]), &top_stock(0.));
    assert_eq!(ids(&plan), ["bottom"]);
    assert!(!plan["setups"].to_string().contains("entered_from_exit"));
}

#[test]
fn material_above_the_opening_sends_the_hole_to_its_other_end() {
    // A ledge 5 mm above the top opening of a plain through hole.
    let bounds = ([-20., -20., -10.], [20., 20., 5.]);
    let a = synthetic(&[down(0., 0., 6., 10., "through")], bounds, &[], &[]);
    let plan = plan(&a, Some(&mesh_of(&square(5., 5., 0., 0.), bounds)), &top_stock(0.)).unwrap();
    let bottom: Vec<&Value> = list(&plan["setups"]).iter().filter(|s| s["id"] == "bottom").collect();
    assert_eq!(bottom.len(), 1);
    assert_eq!(bottom[0]["operations"][0]["features"], json!(["holes:0"]));
    assert!(!kinds(&plan, 0).contains(&"holes"));
}

#[test]
fn a_blind_hole_under_material_is_skipped() {
    let bounds = ([-20., -20., -10.], [20., 20., 5.]);
    let a = synthetic(&[down(0., 0., 6., 4., "flat")], bounds, &[], &[]);
    let plan = plan(&a, Some(&mesh_of(&square(5., 5., 0., 0.), bounds)), &Default::default()).unwrap();
    let reason = plan["skipped"][0]["reason"].as_str().unwrap();
    assert!(reason.starts_with("part material above its opening from top"), "{reason}");
}

#[test]
fn material_beyond_a_through_exit_is_measured() {
    // A through hole that breaks into a cavity, with its far wall 2 mm below the exit.
    let a = holes_only(&[down(0., 0., 6.6, 6., "through")]);
    let plan = plan(&a, Some(&mesh_of(&square(-8., 5., 0., 0.), BOX)), &top_stock(0.)).unwrap();
    let op = of_kind(&plan, 0, "holes")[0];
    assert_same(json!([op["bottom_z"], op["exit_clear_mm"]]), json!([-6, 2]));
    assert!(warnings(&plan).iter().any(|w| w.contains("ends 4.0 mm short of the part's far face")), "{}", plan["warnings"]);
}

#[test]
fn an_exit_on_the_parts_face_has_nothing_beyond_it() {
    let plan = plan(&plate(), Some(&plate_mesh()), &Default::default()).unwrap();
    for op in of_kind(&plan, 0, "holes") {
        assert!(op["exit_clear_mm"].is_null(), "{}", op["exit_clear_mm"]);
    }
    assert_eq!(plan["warnings"], json!([]));
}

#[test]
fn without_a_mesh_an_inner_exit_counts_as_blocked() {
    let plan = no_mesh(&holes_only(&[down(0., 0., 6.6, 6., "through")]), &top_stock(0.));
    assert_eq!(number(&plan["setups"][0]["operations"][0]["exit_clear_mm"]), 0.);
    assert!(warnings(&plan)[0].starts_with("no mesh"), "{}", plan["warnings"]);
}

#[test]
fn roughing_covers_only_features_it_can_reach() {
    let faces = [
        json!({"id": 0, "kind": "plane", "direction": [0, 0, 1]}),
        json!({"id": 1, "kind": "plane", "direction": [0, 0, -1]}),
        json!({"id": 2, "kind": "plane", "direction": [1, 0, 0]}),
    ];
    let features = [
        json!({"id": "pockets:0", "family": "pockets", "record": {}, "faces": [0, 2]}), // opens up
        json!({"id": "pockets:1", "family": "pockets", "record": {}, "faces": [1, 2]}), // opens down
        json!({"id": "fillets:3", "family": "fillets", "record": {}, "faces": [2]}),
        json!({"id": "hole_patterns:4", "family": "hole_patterns", "record": {}, "faces": []}),
    ];
    let a = synthetic(&[], BOX, &features, &faces);
    let skipped = |p: &Value| -> Vec<String> { list(&p["skipped"]).iter().map(|s| s["feature"].as_str().unwrap().to_string()).collect() };
    let roughed = plan(&a, Some(&mesh_of(&square(-10., 20., 0., 0.), BOX)), &Default::default()).unwrap();
    assert_eq!(roughed["covered_by_roughing"], json!(["pockets:0", "fillets:3"]));
    assert_eq!(skipped(&roughed), ["pockets:1"]);
    // Without the mesh nothing is roughed, and every family is listed.
    assert_eq!(skipped(&no_mesh(&a, &Default::default())), ["pockets:0", "pockets:1", "fillets:3"]);
}

#[test]
fn the_mesh_error_is_measured_on_flats_and_cylinders() {
    // The plate is flat faces and holes: its triangles stray only by the holes' chords,
    // far less than specify-core's 0.1%-of-diagonal bound (0.117 mm here), which camlib
    // (at most 0.1 mm) would refuse.
    let (analysis, mesh) = (plate(), plate_mesh());
    let plan = plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let error = number(&roughing(&plan)["surface_error_mm"]);
    assert!(0.01 < error && error < 0.02, "{error}");
    // Checked by sampling each cylinder triangle against its cylinder.
    let faces: HashMap<u64, &Value> = list(&analysis["faces"]).iter().map(|f| (f["id"].as_u64().unwrap(), f)).collect();
    let p: Vec<f64> = list(&mesh["positions"]).iter().map(number).collect();
    let point = |i: &Value| {
        let i = i.as_u64().unwrap() as usize;
        [p[3 * i], p[3 * i + 1], p[3 * i + 2]]
    };
    let tris: Vec<[[f64; 3]; 3]> = list(&mesh["indices"]).chunks_exact(3).map(|t| [point(&t[0]), point(&t[1]), point(&t[2])]).collect();
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let vector = |v: &Value| [number(&v[0]), number(&v[1]), number(&v[2])];
    let mut sampled: f64 = 0.;
    let mut cylinders = 0;
    for (index, range) in list(&mesh["faces"]).iter().enumerate() {
        let face = faces[&(index as u64)];
        if face["kind"] != "cylinder" {
            continue;
        }
        cylinders += 1;
        let (origin, radius) = (vector(&face["origin"]), number(&face["radius"]));
        let direction = vector(&face["direction"]);
        let length = dot(direction, direction).sqrt();
        let axis = direction.map(|x| x / length);
        let (start, count) = (range[0].as_u64().unwrap() as usize, range[1].as_u64().unwrap() as usize);
        for t in &tris[start..start + count] {
            for i in 0..6 {
                for j in 0..6 - i {
                    let w = [i as f64 / 5., j as f64 / 5., 1. - i as f64 / 5. - j as f64 / 5.];
                    let q = [0, 1, 2].map(|n| (0..3).map(|k| w[k] * t[k][n]).sum::<f64>() - origin[n]);
                    let along = dot(q, axis);
                    let radial = (dot(q, q) - along * along).max(0.).sqrt();
                    sampled = sampled.max((radius - radial).abs());
                }
            }
        }
    }
    assert!(cylinders > 0);
    assert!(sampled <= error, "{sampled} > {error}");
}

#[test]
fn a_mesh_too_coarse_for_camlib_is_not_roughed() {
    // A curved face of a kind not measured here takes specify-core's bound: 0.1% of a
    // 200 mm diagonal is over camlib's 0.1 mm.
    let bounds = ([0., 0., -10.], [150., 130., 0.]);
    let a = synthetic(&[], bounds, &[], &[json!({"id": 0, "kind": "bspline"})]);
    let mut mesh = mesh_of(&square(-5., 10., 50., 50.), bounds);
    mesh["faces"] = json!([[0, 2]]);
    let plan = plan(&a, Some(&mesh), &Default::default()).unwrap();
    assert!(!kinds(&plan, 0).contains(&"roughing"));
    let reason = list(&plan["skipped"]).iter().find(|s| s["feature"] == "roughing").unwrap()["reason"].as_str().unwrap();
    assert!(reason.contains("camlib accepts at most 0.1 mm"), "{reason}");
}

#[test]
fn roughing_says_what_web_it_leaves() {
    let plan = plan(&plate(), Some(&plate_mesh()), &Default::default()).unwrap();
    assert_eq!(number(&roughing(&plan)["web_mm"]), 0.5);
}

#[test]
fn roughing_from_another_side() {
    let options = Options { rough_from: "bottom".into(), ..Default::default() };
    let plan = plan(&plate(), Some(&plate_mesh()), &options).unwrap();
    let first = &plan["setups"][0];
    assert_eq!((first["id"].as_str(), first["zero"].as_str()), (Some("bottom"), Some("stock")));
    // The plate's through holes are plain, so they are drilled from the roughing side.
    assert_eq!(of_kind(&plan, 0, "holes").iter().map(|o| list(&o["centers"]).len()).sum::<usize>(), 8);
    assert_eq!(list(&plan["setups"]).len(), 1);
}

#[test]
fn oblique_holes_and_countersinks_are_listed_not_cut() {
    let csink = json!({"major_diameter": 8, "included_angle": 90, "axis": [0, 0, -1], "location": [5, 5, 0], "drill_diameter": 4, "depth": 2});
    let a = holes_only(&[hole([0.6, 0., -0.8], [0., 0., 0.], 4., 5., "flat"), with(down(5., 5., 4., 10., "through"), "csink", csink)]);
    let plan = no_mesh(&a, &Default::default());
    let reasons: Vec<&str> = list(&plan["skipped"]).iter().map(|s| s["reason"].as_str().unwrap()).collect();
    assert!(reasons.iter().any(|r| r.starts_with("oblique axis")), "{reasons:?}");
    assert!(reasons.iter().any(|r| r.starts_with("countersink")), "{reasons:?}");
}

#[test]
fn not_an_analysis() {
    assert!(plan(&json!({"hello": 1}), None, &Default::default()).is_err());
}

/// FromStep's round trip: tests/fixtures/plate-m5.analysis.json is specify-core's analysis of
/// tests/fixtures/plate-m5.step, the plate after specify-core wrote `hole.function:holes:12 =
/// tapped:M5` into it (tests/fixtures/answers/plate-m5.answers.json).
#[test]
fn a_thread_written_by_specify_is_milled() {
    let plan = no_mesh(&fixture("plate-m5.analysis.json"), &Default::default());
    let ops = of_kind(&plan, 0, "thread");
    assert_eq!(ops.len(), 1, "{}", plan["warnings"]);
    assert_same(json!([ops[0]["spec"], ops[0]["features"], ops[0]["through"]]), json!(["M5", ["holes:12"], true]));
}

#[test]
fn refusing_a_spot_does_not_supersede_the_drills_shared_feature_warning() {
    let analysis = holes_only(&[down(0., 0., 4.2, 3., "through")]);
    let mesh = mesh_of(&square(-5., 5., 0., 0.), BOX);
    let mut plan = plan(&analysis, Some(&mesh), &top_stock(0.)).unwrap();
    capp::finish(&mut plan, &fixture("../../tests/fixtures/placeholder-tools.json"), &[]).unwrap();
    let warning = list(&plan["decisions"]).iter().find(|d| d["rule"] == "process.hole.short_of_far_face@1").unwrap().clone();
    assert_eq!(warning["subject"], "holes:0");
    assert!(warning.get("superseded_by").is_none());
    let ops = operations(&plan, 0);
    let spot_index = ops.iter().position(|o| o["kind"] == "spot_drill").unwrap();
    let spot = &ops[spot_index];
    let drill = ops.iter().find(|o| o["kind"] == "holes").unwrap().clone();
    assert!(spot["tool"].is_string());
    assert!(drill["tool"].is_string());
    assert!(list(&spot["because"]).contains(&warning["id"]));
    assert!(list(&drill["because"]).contains(&warning["id"]));
    let own: Vec<_> = list(&plan["decisions"]).iter().filter(|d| list(&spot["because"]).contains(&d["id"])
        && d["subject"] == "top:spot_drill" && (d["stage"] == "process" || d["stage"] == "tools"))
        .map(|d| d["id"].clone()).collect();
    assert!(!own.is_empty());
    let operation = format!("top-{}-spot_drill", spot_index + 1);
    capp::exclude(&mut plan, &operation, "spot refused").unwrap();
    let decisions = list(&plan["decisions"]);
    let still_warned = decisions.iter().find(|d| d["id"] == warning["id"]).unwrap();
    assert_eq!(*still_warned, warning, "a refusal must leave a borrowed feature warning unchanged");
    let refusal = decisions.iter().find(|d| d["subject"] == operation && d["outcome"] == "removed").unwrap();
    for id in own {
        let d = decisions.iter().find(|d| d["id"] == id).unwrap();
        assert_eq!(d["superseded_by"], refusal["id"], "the spot's own decisions still cease to stand");
    }
    assert_eq!(*of_kind(&plan, 0, "holes")[0], drill, "the drill still runs with its warning link");
    assert!(of_kind(&plan, 0, "spot_drill").is_empty());
    assert!(list(&plan["warnings"]).contains(&warning["says"]));
}
