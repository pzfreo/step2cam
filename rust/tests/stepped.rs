//! A synthetic part with NIST CTC 01's mix of features at about 1/8 of its size, small enough for
//! the demo's tools and camlib's mesh limit: tests/fixtures/synthetic/stepped_plate.py.
use serde_json::Value;
use std::{fs, path::PathBuf};

fn fixture(path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
fn process_part(name: &str) -> Value {
    let analysis = fixture(&format!("tests/fixtures/synthetic/{name}.analysis.json"));
    let mesh = fixture(&format!("tests/fixtures/synthetic/{name}.mesh.json"));
    step2cam::process(&analysis, &mesh, &fixture("demo/tools.json")).unwrap()
}
fn process() -> Value {
    process_part("stepped-plate")
}
fn operations<'a>(out: &'a Value, setup: &str) -> Vec<&'a Value> {
    out["plan"]["setups"].as_array().unwrap().iter().filter(|s| s["id"] == setup).flat_map(|s| s["operations"].as_array().unwrap()).collect()
}

#[test]
fn it_is_roughed_drilled_and_slotted_from_the_top() {
    let out = process();
    assert!(out["post_error"].is_null(), "{}", out["post_error"]);
    let top = operations(&out, "top");
    let kinds: Vec<&str> = top.iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"roughing"), "{kinds:?} {}", out["plan"]["warnings"]);
    assert_eq!(kinds.iter().filter(|k| **k == "holes").count(), 3, "{kinds:?} {}", out["plan"]["warnings"]);
    // The round-ended slot gets its own cut, which ends at the plate's bottom; the square one does not.
    let slots: Vec<&&Value> = top.iter().filter(|o| o["kind"] == "slot").collect();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0]["width"], 5.0);
    assert_eq!(slots[0]["bottom"], "through");
    let analysis = fixture("tests/fixtures/synthetic/stepped-plate.analysis.json");
    let square = analysis["features"].as_array().unwrap().iter().find(|f| f["family"] == "slots" && f["record"]["end_radius"].is_null()).unwrap();
    assert!(out["plan"]["covered_by_roughing"].to_string().contains(square["id"].as_str().unwrap()));
}

#[test]
fn the_side_holes_are_drilled_from_the_front() {
    let out = process();
    let front = operations(&out, "front");
    let holes: Vec<&&Value> = front.iter().filter(|o| o["kind"] == "holes").collect();
    assert_eq!(holes.len(), 1, "{}", out["plan"]["warnings"]);
    assert_eq!(holes[0]["diameter"], 2.5);
    assert_eq!(holes[0]["bottom"], "drill_point");
    assert_eq!(holes[0]["centers"].as_array().unwrap().len(), 2);
}

/// The top outline is the boss's: a finish following it down to the floor would cut the plate.
#[test]
fn a_raised_boss_outline_is_not_followed_down_through_the_plate() {
    let out = process();
    let kinds: Vec<String> = out["plan"]["setups"].as_array().unwrap().iter().flat_map(|s| s["operations"].as_array().unwrap()).map(|o| o["kind"].as_str().unwrap().to_string()).collect();
    assert!(!kinds.iter().any(|k| k == "wall_finish"), "{kinds:?}");
    assert!(out["plan"]["skipped"].to_string().contains("r0.8 edge round"));
}

/// camlib once refused this shape: rounded corners, a chamfered top edge and a boss together
/// (stevecad/camlib#70).
#[test]
fn a_chamfered_plate_with_a_boss_is_roughed() {
    let out = process_part("stepped-plate-chamfered");
    assert!(out["post_error"].is_null(), "{}", out["post_error"]);
    let kinds: Vec<&str> = operations(&out, "top").iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"roughing"), "{kinds:?} {}", out["plan"]["warnings"]);
    assert!(out["plan"]["skipped"].to_string().contains("chamfer: no operation cuts it"));
}
