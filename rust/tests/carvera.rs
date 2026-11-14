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
    assert!(library.get("decisions").is_none(), "a shared library has no fixture decisions");
    assert!(library.get("cell_size_mm").is_none(), "a shared library has no fixture stock-grid override");
    assert!(!tools.iter().any(|t| t["type"] == "tap"), "no documented Carvera rigid tapping");
    assert_eq!(machine["spindle"]["max_tool_diameter_mm"], 6.35);
    assert_eq!(machine["spindle"]["taper"], "1/4\" collet");
    assert_eq!(machine["spindle"]["max_rpm"], 15000);
    assert_eq!(machine["tool_changer"]["pockets"], 6);
    for tool in tools {
        assert!(tool["shank_diameter"].as_f64().unwrap() <= 6.35, "{tool}");
        assert!(tool["spindle_rpm"].as_f64().unwrap() <= 15000., "{tool}");
        assert!(tool["flute_length"].as_f64().unwrap_or(0.) <= tool["overall_length"].as_f64().unwrap());
    }
    for name in ["synthetic/stepped-plate", "plate"] {
        let analysis = read(&format!("tests/fixtures/{name}.analysis.json"));
        let mesh = read(&format!("tests/fixtures/{name}.mesh.json"));
        let standard = step2cam::process(&analysis, &mesh, &library).unwrap();
        assert!(standard["error"].is_null(), "{standard}");
        if name == "synthetic/stepped-plate" {
            let front = standard["plan"]["setups"].as_array().unwrap().iter().find(|s|s["id"]=="front").unwrap();
            assert!(!front["operations"].as_array().unwrap().is_empty());
            assert_eq!(front["clamping"]["parallels_size"]["kind"], "computed");
            assert!(standard["plan"]["warnings"].to_string().contains("not a standard size"));
        }
        let out = standard;
        assert!(out["error"].is_null(), "{name}: {}", out["error"]);
        let setups = out["plan"]["setups"].as_array().unwrap();
        assert!(!setups.is_empty());
        for setup in setups {
            assert_eq!(setup["machine"], "carvera", "{name}: {setup}");
            assert!(!setup["operations"].as_array().unwrap().is_empty());
            for op in setup["operations"].as_array().unwrap() {
                if let Some(id) = op["tool"].as_str() {
                    let tool = tools.iter().find(|t| t["id"] == id).unwrap_or_else(|| panic!("unknown selected tool {id}"));
                    assert!(tool["shank_diameter"].as_f64().unwrap() <= 6.35);
                    assert!(op["technology"]["spindle_rpm"].as_f64().unwrap() <= 15000.);
                }
            }
        }
        if name == "synthetic/stepped-plate" {
            // #94: on the real library the rule still chooses flip and face, which finishes the part.
            assert_eq!(out["plan"]["workholding"]["chosen"], "vise_flip_face");
            assert_eq!(out["plan"]["workholding"]["source"], "rule");
            let notch = setups.iter().flat_map(|s| s["operations"].as_array().unwrap())
                .find(|op| op["kind"] == "notch" && op["features"].as_array().is_some_and(|f| f.iter().any(|id| id == "slots:37.40.41.44")))
                .expect("the 6 mm notch is cut");
            assert_eq!(notch["corner_radius"], 1.2, "the r1.2 corners belong to the notch");
            let tool = tools.iter().find(|t| t["id"] == notch["tool"]).unwrap();
            assert_eq!(tool["diameter"], 4, "largest fitting/reaching tool, not corner-aware selection");
            assert!(out["plan"]["decisions"].as_array().unwrap().iter().any(|d|
                d["rule"] == "process.notch.corner_radius@1" && d["radius"] == 2.0
                && d["model_radius"] == 1.2 && d["features"].as_array().is_some_and(|f| f.iter().any(|id| id == "slots:37.40.41.44"))),
                "the r2 outcome must be reported against model r1.2");
            // M6: the 30° sharp bit in the 61.9275° channel, 1.8 mm wide and 1.5 deep, leaves
            // 0.9 - 1.5 × tan 15° = 0.4981 mm across the top of each wall: the channel is partly cut.
            let channel = setups.iter().flat_map(|s| s["operations"].as_array().unwrap())
                .find(|op| op["kind"] == "v_channel").expect("the V-channel is cut");
            assert_eq!(channel["tool"], "vb30-sharp");
            let stock = out["plan"]["decisions"].as_array().unwrap().iter()
                .find(|d| d["rule"] == "process.v_channel.wall_stock@1" && d["features"] == channel["features"])
                .expect("the V-channel's wall stock is reported");
            assert_eq!(stock["outcome"], "warning");
            assert_eq!(stock["wall_stock_mm"], 0.4981);
            assert!(stock["says"].as_str().unwrap().contains("left up to 0.498 mm thick at the top by vb30-sharp, a 30° bit in the 61.9275° channel"), "{stock}");
            assert!(channel["because"].as_array().unwrap().contains(&stock["id"]));
            let cut_operations = setups.iter().flat_map(|s| s["operations"].as_array().unwrap())
                .filter(|op| op["tool"].is_string()).count();
            // Entry-aware grid refinement recovers both front threads while retaining ordinary coverage.
            assert!(cut_operations >= 17, "complex plate: only {cut_operations} cut operations");
            let left_out = out["plan"]["skipped"].as_array().unwrap().len();
            assert!(left_out <= 20, "complex plate: {left_out} left-out entries");
        }
        let programs = out["programs"].as_array().unwrap();
        assert_eq!(programs.len(), setups.len(), "{name}: {}", out["log"]);
        if name == "synthetic/stepped-plate" {
            let front = programs.iter().find(|p|p["setup"]=="front").expect("Carvera front job");
            let drilled: Vec<&Value> = front["job"]["operations"].as_array().unwrap().iter().filter(|o|o["kind"]=="drilling" && o["tool"]=="drill2.5").collect();
            assert_eq!(drilled.len(), 1);
            assert_eq!(drilled[0]["centers_mm"].as_array().unwrap().len(), 2);
            let manifest: camlib::stream::StreamManifest = serde_json::from_value(front["job"].clone()).unwrap();
            let stock = manifest.stock.initial_stock(&||false).unwrap();
            let stock = stock.height().unwrap();
            // The carried envelope was exported on the previous setup's grid.
            // Refining the receiving grid cannot shrink that conservative margin.
            let previous = programs.iter().position(|p| p["setup"] == "front").unwrap() - 1;
            let cell = programs[previous]["job"]["stock"]["cell_size_mm"].as_f64().unwrap();
            assert!(stock.spec().cell_size_mm < cell, "front entry refines the receiving grid");
            assert!(stock.cells().iter().all(|c|c.upper_mm <= cell + 1e-4));
            assert!(front["job"]["stock"]["mesh"]["vertices"].as_array().unwrap().iter().all(|p|
                p[0].as_f64().unwrap() >= -cell-1e-4 && p[0].as_f64().unwrap() <=100.+cell+1e-4
                && p[1].as_f64().unwrap() >= -cell-1e-4 && p[1].as_f64().unwrap() <=18.+cell+1e-4));
            assert!(setups.iter().find(|s|s["id"]=="front").unwrap()["stock_note"].as_str().unwrap().contains("the part is free"));
        }
        let mut compared = 0;
        for program in programs {
            // Earlier candidates can legitimately be refused for insufficient catalogue reach.
            // Recheck the final executable job, rather than treating that diagnostic as a gouge.
            assert!(step2cam::simulate::simulate(&program["job"].to_string()).is_ok(),
                "{name}: final {} job failed camlib checks", program["setup"]);
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
