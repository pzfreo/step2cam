//! The opt-in industrial library must load, place both fixtures and run camlib.
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn read(path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn industrial_fixtures_plan_and_simulate_without_definite_gouges() {
    let library = read("libraries/haas-vf2.json");
    for name in ["synthetic/stepped-plate", "mounting-plate"] {
        let mut shop = library.clone();
        if name == "mounting-plate" {
            let standard = step2cam::process(&read(&format!("tests/fixtures/{name}.analysis.json")), &read(&format!("tests/fixtures/{name}.mesh.json")), &library).unwrap();
            assert!(standard["plan"]["setups"][0]["operations"].as_array().unwrap().is_empty());
            assert!(standard["plan"]["warnings"].to_string().contains("no standard parallels height"));
            // Thin plate seating requires a shop height between the standard set's steps.
            shop["machines"][0]["workholding"][0]["parallels_heights_mm"] = json!([40.569]);
        }
        let out = step2cam::process(
            &read(&format!("tests/fixtures/{name}.analysis.json")),
            &read(&format!("tests/fixtures/{name}.mesh.json")),
            &shop,
        ).unwrap();
        assert!(out["error"].is_null(), "{name}: {}", out["error"]);
        let setups = out["plan"]["setups"].as_array().unwrap();
        assert!(!setups.is_empty(), "{name}: no setups");
        for setup in setups {
            assert_eq!(setup["machine"], "haas-vf2", "{name}: {setup}");
            assert!(!setup["operations"].as_array().unwrap().is_empty(), "{name}: {setup}");
            assert!(setup["operations"].as_array().unwrap().iter().any(|op| op["tool"].is_string()), "{name}: no selected tools");
        }
        let programs = out["programs"].as_array().unwrap();
        assert!(!programs.is_empty(), "{name}: {}", out["log"]);
        let mut compared = 0;
        for program in programs {
            // The stepped fixture's front mesh does not support a material comparison.
            // Keep that gap visible; null must never be counted as a zero-gouge report.
            if program["material"].is_null() {
                assert_eq!(name, "synthetic/stepped-plate");
                assert_eq!(program["setup"], "front");
                assert!(out["log"].as_array().unwrap().iter().any(|line| line.as_str().unwrap().contains("setup front: the finished stock is not compared")));
                continue;
            }
            compared += 1;
            assert_eq!(program["material"]["gouge"]["definite_cells"], 0, "{name}: {}", program["material"]);
        }
        assert!(compared > 0, "{name}: no material comparisons");
    }
}

#[test]
fn catalogue_m5_cut_tap_is_used_and_checked_on_the_threaded_plate() {
    let library = read("libraries/haas-vf2.json");
    let analysis = read("tests/fixtures/plate-m5.analysis.json");
    let mut mesh = read("tests/fixtures/plate.mesh.json");
    mesh["binding"] = analysis["binding"].clone();
    let out = step2cam::process(&analysis, &mesh, &library).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let tap = out["programs"].as_array().unwrap().iter()
        .flat_map(|p| p["job"]["operations"].as_array().unwrap())
        .find(|op| op["kind"] == "tapping").unwrap_or_else(|| panic!("{}", out["plan"]["warnings"]));
    assert_eq!(tap["tool"], "tapM5");
    assert_eq!(tap["pitch_mm"], 0.8);
    assert_eq!(tap["spindle_rpm"], 381.);
    assert!(out["programs"].as_array().unwrap().iter().flat_map(|p| p["moves"].as_array().unwrap())
        .any(|m| m["kind"] == "tap" && (m["feed"].as_f64().unwrap() - 381. * 0.8).abs() < 1e-8));
}

#[test]
fn the_120_degree_spot_drill_spots_the_118_degree_drills_on_the_stepped_plate() {
    // Review M5: with only the 90° spot drill no 118° drill hole was spotted.
    let library = read("libraries/haas-vf2.json");
    let analysis = read("tests/fixtures/synthetic/stepped-plate.analysis.json");
    let mut plan = capp::plan::plan(&analysis, Some(&read("tests/fixtures/synthetic/stepped-plate.mesh.json")), &Default::default()).unwrap();
    let unspotted = {
        let mut lib = library.clone();
        lib["tools"].as_array_mut().unwrap().retain(|t| t["id"] != "spot10-120");
        let mut p = plan.clone();
        capp::finish(&mut p, &lib, &[]).unwrap();
        p
    };
    capp::finish(&mut plan, &library, &[]).unwrap();
    let ops = |p: &Value| -> Vec<Value> { p["setups"].as_array().unwrap().iter().flat_map(|s| s["operations"].as_array().unwrap().clone()).collect() };
    assert!(!ops(&unspotted).iter().any(|o| o["kind"] == "spot_drill" && o["tool"].is_string()));
    let mut spotted = 0;
    for setup in plan["setups"].as_array().unwrap() {
        let ops = setup["operations"].as_array().unwrap();
        let tool = |o: &Value| o["tool"].as_str().unwrap_or("").to_string();
        let Some(first_drill) = ops.iter().position(|o| o["kind"] == "holes" && tool(o).starts_with("drill")) else { continue };
        let spot = ops.iter().position(|o| o["kind"] == "spot_drill").unwrap_or_else(|| panic!("{}: drilled but not spotted", setup["id"]));
        assert!(spot < first_drill, "{}: spot after drill", setup["id"]);
        assert_eq!(ops[spot]["tool"], "spot10-120");
        assert_eq!(ops[spot]["angle"], 118.);
        spotted += 1;
    }
    assert!(spotted > 0);
}
