//! Real Makera inventory, rather than the generic placeholder regression fixture.
use serde_json::Value;
use std::{fs, path::PathBuf};

fn read(path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn real_carvera_fixtures_plan_and_pass_camlib_checks() {
    let library = read("demo/tools.json");
    let tools = library["tools"].as_array().unwrap();
    let machine = &library["machines"][0];
    assert_eq!(machine["spindle"]["max_tool_diameter_mm"], 3.175);
    assert_eq!(machine["spindle"]["max_rpm"], 15000);
    assert_eq!(machine["tool_changer"]["pockets"], 6);
    for tool in tools {
        assert!(tool["shank_diameter"].as_f64().unwrap() <= 3.175, "{tool}");
        assert!(tool["spindle_rpm"].as_f64().unwrap() <= 15000., "{tool}");
        assert!(tool["flute_length"].as_f64().unwrap_or(0.) <= tool["overall_length"].as_f64().unwrap());
    }
    for name in ["synthetic/stepped-plate", "plate"] {
        let out = step2cam::process(
            &read(&format!("tests/fixtures/{name}.analysis.json")),
            &read(&format!("tests/fixtures/{name}.mesh.json")),
            &library,
        ).unwrap();
        assert!(out["error"].is_null(), "{name}: {}", out["error"]);
        let setups = out["plan"]["setups"].as_array().unwrap();
        assert!(!setups.is_empty());
        for setup in setups {
            assert_eq!(setup["machine"], "carvera", "{name}: {setup}");
            assert!(!setup["operations"].as_array().unwrap().is_empty());
            for op in setup["operations"].as_array().unwrap() {
                if let Some(id) = op["tool"].as_str() {
                    let tool = tools.iter().find(|t| t["id"] == id).unwrap_or_else(|| panic!("unknown selected tool {id}"));
                    assert!(tool["shank_diameter"].as_f64().unwrap() <= 3.175);
                    assert!(op["technology"]["spindle_rpm"].as_f64().unwrap() <= 15000.);
                }
            }
        }
        if name == "synthetic/stepped-plate" {
            let slot = setups.iter().flat_map(|s| s["operations"].as_array().unwrap())
                .find(|op| op["kind"] == "slot" && op["width"] == 5.0 && op["tool"].is_string());
            if let Some(slot) = slot {
                let tool = tools.iter().find(|t| t["id"] == slot["tool"]).unwrap();
                assert!(tool["diameter"].as_f64().unwrap() <= 2.4, "{slot}");
            } else {
                // The kit's 2 x 8 mm metal cutter cannot reach this 19 mm deep slot.
                assert!(out["plan"]["tools_needed"].as_array().unwrap().iter().any(|need|
                    need["features"].as_array().unwrap().iter().any(|f| f == "slots:19.20")
                    && need["description"].as_str().unwrap().contains("19 mm")));
            }
        }
        assert!(!out["plan"]["warnings"].to_string().contains("camlib"), "{name}: {}", out["plan"]["warnings"]);
        let programs = out["programs"].as_array().unwrap();
        assert_eq!(programs.len(), setups.len(), "{name}: {}", out["log"]);
        let mut compared = 0;
        for program in programs {
            // process removes refused operations. Require every selected job to complete.
            for job in program["job"]["operations"].as_array().unwrap() {
                let id = job["id"].as_str().unwrap();
                assert!(program["operations"].as_array().unwrap().iter().any(|op| {
                    let actual = op["id"].as_str().unwrap();
                    actual == id || actual.starts_with(&format!("{id}-"))
                }), "{name}: camlib did not complete {id}: {}", out["log"]);
            }
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
