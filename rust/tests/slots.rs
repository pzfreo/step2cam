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
    step2cam::process(&analysis, &mesh, &fixture("demo/tools.json")).unwrap()
}
fn word(line: &str, letter: char) -> Option<f64> {
    line.split(' ').find_map(|w| w.strip_prefix(letter)?.parse().ok())
}

#[test]
fn the_plate_is_roughed_and_its_slot_cut_to_size() {
    let out = process();
    assert!(out["post_error"].is_null(), "{}", out["post_error"]);
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
    let nc = out["programs"][0]["nc"].as_str().unwrap();
    let cut = nc.split_once("T3\n").expect("the ⌀6 mill").1;
    let cut = cut.split_once("\nT").map_or(cut, |c| c.0);
    let (mut x, mut y, mut z) = (vec![], vec![], vec![]);
    let (mut at, mut depth) = ([0., 0.], 0.);
    for line in cut.lines() {
        if !["G0 ", "G1 ", "G2 ", "G3 "].iter().any(|code| line.starts_with(code)) {
            continue;
        }
        let end = [word(line, 'X').unwrap_or(at[0]), word(line, 'Y').unwrap_or(at[1])];
        depth = word(line, 'Z').unwrap_or(depth);
        if !line.starts_with("G0 ") && depth < -1. {
            x.push(end[0]);
            y.push(end[1]);
            z.push(depth);
            if let (Some(i), Some(j)) = (word(line, 'I'), word(line, 'J')) {
                let radius = i.hypot(j);
                y.push(at[1] + j + radius);
                y.push(at[1] + j - radius);
            }
        }
        at = end;
    }
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
    let plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    assert!(plan["covered_by_roughing"].as_array().unwrap().is_empty());
    assert!(plan["skipped"].to_string().contains("roughing is not planned"));
}

#[test]
fn a_wall_in_line_with_a_slot_is_not_material_under_it() {
    // A vertical triangle whose plane runs through the slot's centre line, far from the slot.
    let wall = [[10.0, 200.0, 0.0], [10.0, 300.0, 0.0], [10.0, 300.0, 5.0]];
    assert!(!step2cam::plan::disk_meets_for_test((10.0, 0.0), 4.0, &wall));
}

#[test]
fn the_top_edges_are_chamfered_where_they_meet_the_round_and_never_inside_it() {
    let out = process();
    assert!(out["post_error"].is_null(), "{}", out["post_error"]);
    let ops = out["plan"]["setups"][0]["operations"].as_array().unwrap();
    let chamfer = ops.iter().find(|o| o["kind"] == "edge_chamfer").expect("a chamfer");
    assert_eq!(chamfer["boundaries"], "all");
    assert_eq!(chamfer["width"], 0.4686);
    assert_eq!(chamfer["features"].as_array().unwrap().len(), 10);
    // Outer wall X102 in the setup, top Z-1. The cone (⌀0.5 tip, 90°) touches the wall 0.4686 below
    // the top from outside: its axis is tip radius + 0.5 clearance out, its tip 0.9686 below the top.
    let nc = out["programs"][0]["nc"].as_str().unwrap();
    let number = fixture("demo/tools.json")["tools"].as_array().unwrap().iter().find(|t| t["id"] == "cm6").unwrap()["number"].as_u64().unwrap();
    let cut = nc.split_once(&format!("T{number}\n")).expect("the chamfer mill").1;
    let (mut most_x, mut lowest) = (f64::MIN, f64::MAX);
    let mut depth: f64 = 0.;
    for line in cut.lines().filter(|l| ["G1 ", "G2 ", "G3 "].iter().any(|code| l.starts_with(code))) {
        depth = word(line, 'Z').unwrap_or(depth);
        lowest = lowest.min(depth);
        if depth < -1. {
            most_x = most_x.max(word(line, 'X').unwrap_or(f64::MIN));
        }
    }
    assert!((most_x - 102.75).abs() < 0.01, "{most_x}");
    assert!((lowest + 1.9686).abs() < 0.01, "{lowest}");
}
