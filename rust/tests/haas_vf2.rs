//! The opt-in industrial library must load, place both fixtures and run camlib.
use serde_json::Value;
use std::{fs, path::PathBuf};

fn read(path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn industrial_fixtures_plan_and_simulate_without_definite_gouges() {
    let library = read("libraries/haas-vf2.json");
    for name in ["synthetic/stepped-plate", "mounting-plate"] {
        let out = step2cam::process(
            &read(&format!("tests/fixtures/{name}.analysis.json")),
            &read(&format!("tests/fixtures/{name}.mesh.json")),
            &library,
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
