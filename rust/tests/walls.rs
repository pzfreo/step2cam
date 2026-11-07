//! Finishing walls and flats: shapes an outline alone would cut wrongly, in tests/fixtures/synthetic/walls.
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use step2cam::post::{Machine, post};

fn fixture(path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
fn tools() -> Value {
    fixture("demo/tools.json")
}
fn machine() -> Machine {
    serde_json::from_value(tools()["machine"].clone()).unwrap()
}
fn part(name: &str) -> (Value, Value) {
    (fixture(&format!("tests/fixtures/synthetic/{name}.analysis.json")), fixture(&format!("tests/fixtures/synthetic/{name}.mesh.json")))
}
fn process(name: &str) -> Value {
    let (analysis, mesh) = part(name);
    let out = step2cam::process(&analysis, &mesh, &tools()).unwrap();
    assert!(out["post_error"].is_null(), "{name}: {}", out["post_error"]);
    out
}
fn walls(out: &Value) -> Vec<Value> {
    out["plan"]["setups"][0]["operations"].as_array().unwrap().iter().filter(|o| o["kind"] == "wall_finish").cloned().collect()
}
fn warnings(out: &Value) -> String {
    out["plan"]["warnings"].to_string()
}

/// A cap wider than the tool: its section encloses the stem's and looks like room.
#[test]
fn a_wall_under_an_overhang_is_left_alone_and_the_part_still_posts() {
    let out = process("walls/overhang");
    assert!(warnings(&out).contains("0 mm beside the wall is too narrow"), "{}", warnings(&out));
    assert_eq!(walls(&out).len(), 1, "only the plate's");
    assert_eq!(out["programs"][0]["material"]["gouge"]["definite_cells"], 0);
}

/// The island's vertices are all far from the pocket's long walls: only their spans are near.
#[test]
fn a_wall_too_close_to_another_is_measured_along_that_wall() {
    let out = process("walls/island");
    assert!(warnings(&out).contains("11.7 mm beside the wall is too narrow"), "{}", warnings(&out));
    assert_eq!(walls(&out).len(), 1, "only the block's");
}

/// The cutter fits, but roughing, with its allowance on both sides, does not get in.
#[test]
fn a_wall_needs_room_for_the_allowance_opposite_too() {
    let out = process("walls/channel");
    assert!(warnings(&out).contains("12.3 mm beside the wall is too narrow for em12 (it needs 12.6)"), "{}", warnings(&out));
}

/// A rib flush with the plate's edge is the plate's own wall, not something in its way.
#[test]
fn a_rib_flush_with_a_wall_does_not_stop_that_wall() {
    let out = process("walls/channel");
    let walls = walls(&out);
    assert_eq!(walls.len(), 1);
    assert!(walls[0]["room"].is_null(), "{}", walls[0]);
}

#[test]
fn a_round_boss_is_finished_as_a_full_circle() {
    let out = process("walls/round-boss");
    assert!(!warnings(&out).contains("bulge"), "{}", warnings(&out));
    assert_eq!(walls(&out).len(), 2);
    let circle = walls(&out).into_iter().find(|w| w["on_flat"] == true).unwrap();
    assert!(circle["outer"].as_array().unwrap().iter().all(|v| v[2].as_f64().unwrap().abs() <= 1.));
    assert!(out["programs"][0]["nc"].as_str().unwrap().contains("G2 "), "climb milled, clockwise round the outside");
}

/// With no roughing a contour would slot through solid stock, unchecked.
#[test]
fn with_roughing_refused_no_wall_or_flat_is_cut() {
    let mut library = tools();
    for tool in library["tools"].as_array_mut().unwrap() {
        if tool["type"] == "end_mill" {
            tool["flute_length"] = 1.into();
            tool["stickout"] = 1.5.into();
        }
    }
    let (analysis, mesh) = part("stepped-plate");
    let out = step2cam::process(&analysis, &mesh, &library).unwrap();
    assert!(warnings(&out).contains("roughing left out"), "{}", warnings(&out));
    let kinds = out["plan"]["setups"][0]["operations"].to_string();
    assert!(!kinds.contains("wall_finish") && !kinds.contains("flats"), "{kinds}");
    assert!(warnings(&out).contains("no roughing has cleared the stock"), "{}", warnings(&out));
}

/// camlib proves a flat clear of the part, not how deep it goes. Where roughing did not reach, it
/// would take the whole thickness in one pass at finishing feed.
#[test]
fn a_flat_where_roughing_did_not_reach_is_refused() {
    let (analysis, mesh) = part("stepped-plate");
    let plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&plan, &tools()).unwrap();
    let mut manifest = jobs[0].manifest.clone();
    for op in manifest["operations"].as_array_mut().unwrap() {
        if op["kind"] == "roughing" {
            op["floor_z_mm"] = (-1.5).into();
        }
    }
    let Err(failure) = post(&manifest.to_string(), machine()) else { panic!("a flat through uncut stock passed") };
    assert!(failure.message.contains("a flat would cut"), "{}", failure.message);
    assert!(failure.operation.as_deref().is_some_and(|id| id.ends_with("flats")), "{:?}", failure.operation);
}

#[test]
fn a_flat_on_a_roughed_part_stays_within_its_limit_and_enters_from_the_retract_height() {
    let out = process("stepped-plate");
    assert!(out["programs"][0]["operations"].to_string().contains("flats"));
    let (analysis, mesh) = part("stepped-plate");
    let plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&plan, &tools()).unwrap();
    let flat = jobs[0].manifest["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "flat").unwrap().clone();
    assert!(flat["retract_z_mm"].is_number() && flat["feed_z_mm"].is_number(), "{flat}");
}

/// A chamfer is planned for a wall at size. Without that wall it would leave a step.
#[test]
fn a_chamfer_goes_with_its_wall() {
    let analysis = fixture("tests/fixtures/mounting-plate.analysis.json");
    let mesh = fixture("tests/fixtures/mounting-plate.mesh.json");
    let mut plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let ops = plan["setups"][0]["operations"].as_array_mut().unwrap();
    let needed = ops.iter().find(|o| o["kind"] == "edge_chamfer" && !o["needs_wall"].is_null()).expect("an outside chamfer")["needs_wall"].clone();
    let wall = ops.iter_mut().find(|o| o["wall"] == needed).unwrap();
    wall["room"] = json!(1.0);
    step2cam::job::settle(&mut plan, &tools()).unwrap();
    let ops = plan["setups"][0]["operations"].as_array().unwrap();
    assert!(!ops.iter().any(|o| o["wall"] == needed), "the wall stayed");
    for chamfer in ops.iter().filter(|o| o["kind"] == "edge_chamfer") {
        assert_eq!(chamfer["boundaries"], "holes", "{chamfer}");
    }
    assert!(plan["skipped"].to_string().contains("its wall keeps its roughing allowance"), "{}", plan["skipped"]);
    assert!(plan["warnings"].to_string().contains("too narrow"), "{}", plan["warnings"]);
}

/// Climb: the part on the cutter's right, clockwise round the outside.
#[test]
fn the_wall_finish_climbs() {
    let (analysis, mesh) = part("walls/round-boss");
    let plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&plan, &tools()).unwrap();
    let contours: Vec<&Value> = jobs[0].manifest["operations"].as_array().unwrap().iter().filter(|o| o["kind"] == "contour").collect();
    assert!(!contours.is_empty());
    assert!(contours.iter().all(|c| c["reverse"] == true));
}
