//! mounting-plate.step: 100 × 70 × 8, with an 8 × 30 round-ended through slot and 0.8 mm edge
//! rounds. Its fixtures are specify-core's analysis and mesh of that part (see
//! tests/fixtures/mounting-plate.LICENSE).
use serde_json::Value;
use std::{fs, path::PathBuf};

fn fixture(path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
fn process() -> Value {
    let analysis = fixture("tests/fixtures/mounting-plate.analysis.json");
    let mesh = fixture("tests/fixtures/mounting-plate.mesh.json");
    // The demo's tools on no particular machine: this is about planning, not the Carvera.
    let mut tools = fixture("demo/tools.json");
    tools.as_object_mut().unwrap().remove("machines");
    step2cam::process(&analysis, &mesh, &tools).unwrap()
}
/// The points of camlib's fed moves (not rapids) for which `keep` holds.
fn fed_points(out: &Value, keep: &dyn Fn(&Value) -> bool) -> Vec<[f64; 3]> {
    let moves = out["programs"][0]["moves"].as_array().unwrap();
    let point = |p: &Value| [0, 1, 2].map(|k| p[k].as_f64().unwrap());
    moves.iter().filter(|m| m["kind"] != "rapid" && keep(m)).flat_map(|m| m["points"].as_array().unwrap().iter().skip(1).map(point)).collect()
}

#[test]
fn the_plate_is_roughed_and_its_slot_cut_to_size() {
    let out = process();
    assert!(out["error"].is_null(), "{}", out["error"]);
    assert!(out["plan"]["warnings"].as_array().unwrap().is_empty(), "{}", out["plan"]["warnings"]);
    let ops = out["plan"]["setups"][0]["operations"].as_array().unwrap();
    let kinds: Vec<&str> = ops.iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["facing", "roughing", "holes", "holes", "slot", "wall_finish", "edge_chamfer"]);
    // Measured on its tori, the mesh is well inside camlib's 0.1 mm.
    assert!(ops[1]["surface_error_mm"].as_f64().unwrap() < 0.05);

    let slot = &ops[4];
    assert_eq!(slot["bottom"], "through");
    assert_eq!(slot["a"], serde_json::json!([84.0, 26.0]));
    assert_eq!(slot["b"], serde_json::json!([84.0, 48.0]));

    // ⌀6 in an 8 wide slot: its centre runs ±1 across, and round the ends to 1 from their tips.
    let points: Vec<[f64; 3]> = fed_points(&out, &|m| m["op"].as_str().unwrap().ends_with("-slot")).into_iter().filter(|p| p[2] < -1.).collect();
    let x: Vec<f64> = points.iter().map(|p| p[0]).collect();
    let y: Vec<f64> = points.iter().map(|p| p[1]).collect();
    let z: Vec<f64> = points.iter().map(|p| p[2]).collect();
    let range = |v: &[f64]| (v.iter().copied().fold(f64::MAX, f64::min), v.iter().copied().fold(f64::MIN, f64::max));
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    let (x, y, z) = (range(&x), range(&y), range(&z));
    assert!(near(x.0, 83.) && near(x.1, 85.), "{x:?}");
    assert!(near(y.0, 25.) && near(y.1, 49.), "{y:?}");
    assert!(near(z.0, -9.), "{z:?}");
}

#[test]
fn what_is_not_cut_is_said_to_be_not_cut() {
    let out = process();
    let plan = &out["plan"];
    let skipped = plan["skipped"].as_array().unwrap();
    let rounds = skipped.iter().filter(|s| s["reason"].as_str().unwrap().contains("r0.8 edge round")).count();
    assert_eq!(rounds, 10); // the bottom edges
    // The outline's r5 corners are walls, which roughing follows; the slot's walls are the slot's.
    let covered: Vec<&str> = plan["covered_by_roughing"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    assert_eq!(covered, ["fillets:15", "fillets:1", "fillets:37", "fillets:4"]);
}

#[test]
fn nothing_is_covered_by_roughing_that_does_not_run() {
    // A curved face it cannot measure: the bound on it (0.12 mm) is more than camlib accepts.
    let mut analysis = fixture("tests/fixtures/mounting-plate.analysis.json");
    let mesh = fixture("tests/fixtures/mounting-plate.mesh.json");
    let faces = analysis["faces"].as_array_mut().unwrap();
    faces.iter_mut().find(|x| x["kind"] == "torus").unwrap()["kind"] = serde_json::json!("bspline");
    let plan = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    assert!(plan["covered_by_roughing"].as_array().unwrap().is_empty());
    assert!(plan["skipped"].to_string().contains("roughing is not planned"));
}

#[test]
fn a_wall_in_line_with_a_slot_is_not_material_under_it() {
    // A vertical triangle whose plane runs through the slot's centre line, far from the slot.
    let wall = [[10.0, 200.0, 0.0], [10.0, 300.0, 0.0], [10.0, 300.0, 5.0]];
    assert!(!capp::plan::disk_meets_for_test((10.0, 0.0), 4.0, &wall));
}

#[test]
fn the_top_edges_are_chamfered_where_they_meet_the_round_and_never_inside_it() {
    let out = process();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let ops = out["plan"]["setups"][0]["operations"].as_array().unwrap();
    let chamfer = ops.iter().find(|o| o["kind"] == "edge_chamfer").expect("a chamfer");
    assert_eq!(chamfer["boundaries"], "all");
    assert_eq!(chamfer["width"], 0.4686);
    assert_eq!(chamfer["features"].as_array().unwrap().len(), 10);
    // Outer wall X102 in the setup, top Z-1. The cone (⌀0.5 tip, 90°) touches the wall 0.4686 below
    // the top from outside: its axis is tip radius + 0.5 clearance out, its tip 0.9686 below the top.
    let points = fed_points(&out, &|m| m["tool"] == "cm6");
    let lowest = points.iter().map(|p| p[2]).fold(f64::MAX, f64::min);
    let most_x = points.iter().filter(|p| p[2] < -1.).map(|p| p[0]).fold(f64::MIN, f64::max);
    assert!((most_x - 102.75).abs() < 0.01, "{most_x}");
    assert!((lowest + 1.9686).abs() < 0.01, "{lowest}");
}

/// Roughing covers features that have no operation of their own, until it is refused: then they
/// must not be reported as covered.
#[test]
fn nothing_is_covered_by_a_roughing_that_was_refused() {
    let mut library = fixture("demo/tools.json");
    let analysis = fixture("tests/fixtures/mounting-plate.analysis.json");
    let mesh = fixture("tests/fixtures/mounting-plate.mesh.json");
    let mut normal = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    capp::finish(&mut normal, &library, &[]).unwrap();
    let covered = normal["covered_by_roughing"].as_array().unwrap();
    assert!(!covered.is_empty(), "the normal plan must cover features by roughing");
    for tool in library["tools"].as_array_mut().unwrap() {
        if tool["type"] == "end_mill" {
            // Reach is capped before execution; invalid cutting data force camlib's refusal.
            tool["stepdown_mm"] = 0.into();
        }
    }
    let out = step2cam::process(&analysis, &mesh, &library).unwrap();
    assert!(out["plan"]["warnings"].to_string().contains("roughing left out"), "{}", out["plan"]["warnings"]);
    assert_eq!(out["plan"]["covered_by_roughing"], serde_json::json!([]));
    assert!(out["plan"]["skipped"].to_string().contains("roughing was refused"), "{}", out["plan"]["skipped"]);
    for feature in covered {
        let skipped = out["plan"]["skipped"].as_array().unwrap().iter().find(|s| s["feature"] == *feature && s["reason"].as_str().unwrap().contains("roughing was refused")).unwrap_or_else(|| panic!("{feature} must be skipped after roughing was refused"));
        let decision = out["plan"]["decisions"].as_array().unwrap().iter().find(|d| d["subject"] == *feature && d["outcome"] == "covered").unwrap();
        assert_eq!(decision["superseded_by"], skipped["decision"]);
    }
}
