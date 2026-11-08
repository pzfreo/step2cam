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
    assert!(out["error"].is_null(), "{}", out["error"]);
    let top = operations(&out, "top");
    let kinds: Vec<&str> = top.iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["facing", "roughing", "flats", "spot_drill", "holes", "holes", "holes", "slot", "notch", "notch", "notch", "v_channel", "wall_finish", "wall_finish"], "{}", out["plan"]["warnings"]);
    assert_eq!(kinds.iter().filter(|k| **k == "holes").count(), 3, "{kinds:?} {}", out["plan"]["warnings"]);
    // The round-ended slot gets its own cut, which ends at the plate's bottom.
    let slots: Vec<&&Value> = top.iter().filter(|o| o["kind"] == "slot").collect();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0]["width"], 5.0);
    assert_eq!(slots[0]["bottom"], "through");
    let analysis = fixture("tests/fixtures/synthetic/stepped-plate.analysis.json");
    let square = analysis["features"].as_array().unwrap().iter().find(|f| f["family"] == "slots" && f["record"]["end_radius"].is_null()).unwrap();
    let id = square["id"].as_str().unwrap();
    assert!(top.iter().any(|o| o["kind"] == "notch" && o["features"].to_string().contains(id)), "the square one is a pocket, to size");
    assert!(!out["plan"]["covered_by_roughing"].to_string().contains(id));
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

/// The plate is 12 mm thick and its boss 6 mm above that, from a stock top 1 mm above the boss, so
/// the plate's top is at Z-7. The boss's wall stops there; the plate's runs on to the floor.
#[test]
fn the_plate_and_the_boss_are_finished_at_their_own_heights() {
    let out = process();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let top = operations(&out, "top");
    let flats: Vec<&&Value> = top.iter().filter(|o| o["kind"] == "flats").collect();
    assert_eq!(flats.len(), 1);
    assert_eq!(flats[0]["levels"], serde_json::json!([-7.0]));
    let walls: Vec<f64> = top.iter().filter(|o| o["kind"] == "wall_finish").map(|o| o["floor_z"].as_f64().unwrap()).collect();
    // The lower wall stops 1 mm above the jaws (stock bottom -19 + 20% grip).
    assert_eq!(walls, [-14.2, -7.0]);
    let ids: Vec<&str> = out["programs"][0]["operations"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap()).collect();
    assert!(ids.iter().filter(|i| i.ends_with("wall_finish")).count() == 2 && ids.iter().any(|i| i.ends_with("flats")), "{ids:?} {}", out["log"]);
    // A top edge that stays only rounded is still said so.
    assert!(out["plan"]["skipped"].to_string().contains("r0.8 edge round"));
}

/// camlib once refused this shape: rounded corners, a chamfered top edge and a boss together
/// (stevecad/camlib#70).
#[test]
fn a_chamfered_plate_with_a_boss_is_roughed() {
    let out = process_part("stepped-plate-chamfered");
    assert!(out["error"].is_null(), "{}", out["error"]);
    let kinds: Vec<&str> = operations(&out, "top").iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["facing", "roughing", "flats", "spot_drill", "holes", "holes", "holes", "slot", "notch", "notch", "notch", "wall_finish", "wall_finish", "edge_chamfer"], "{}", out["plan"]["warnings"]);
    let edges = operations(&out, "top").into_iter().find(|o| o["kind"] == "edge_chamfer").expect("the modelled 1 mm top chamfer");
    assert!(edges["tool"].is_null(), "cm6's 4 mm flutes cannot satisfy camlib's stock-top reach check");
    assert!(edges["tool_note"].as_str().unwrap().contains("flutes of at least 8.51 mm"));
    assert_eq!(edges["width"], 1.0);
    assert_eq!(edges["features"].as_array().unwrap().len(), 12);
    assert!(!out["plan"]["skipped"].to_string().contains("chamfers:"));
    assert_eq!(out["programs"][0]["material"]["gouge"]["definite_cells"], 0);
}
