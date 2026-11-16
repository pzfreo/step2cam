use serde_json::{Value, json};
use crate::common::{fixture, process_cached};

fn post_plate(name: &str) {
    let library = fixture("../../libraries/haas-vf2.json");
    let out = process_cached(&fixture(&format!("{name}.analysis.json")),
        &fixture(&format!("{name}.mesh.json")), &library).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let programs = out["programs"].as_array().unwrap();
    assert_eq!(programs.len(), if name == "plate" { 2 } else { 3 });
    let setups = out["plan"]["setups"].as_array().unwrap();
    assert_eq!(programs.len(), setups.len());
    for setup in setups {
        assert_eq!(programs.iter().filter(|p| p["setup"] == setup["id"]).count(), 1);
    }
    for program in programs {
        let setup = program["setup"].as_str().unwrap();
        let report = step2cam::post::post_setup(&program["job"], &library, "haas-vf2", setup);
        println!("{name}/{setup}: released={}, accepted={}, findings={}, refusal={}",
            report["released"], report["accepted"], report["findings"], report["refused"]);
        if setup == "top" {
            // Maintainer-approved shipping expectation: camlib cannot prove the
            // final top-setup moves gouge-free at 0.05 mm, with zero definite
            // gouge cells. Revisit when camlib's proof near walls improves.
            // A future top release must fail this test so that we notice it.
            assert_eq!(report["released"], false);
            assert!(report["program"].is_null());
            assert_eq!(report["accepted"], json!([]));
            assert_eq!(report["refused"]["statuses"], json!(["uncertain"]));
            let findings = report["findings"].as_array().unwrap();
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0]["status"], "uncertain");
            let material = findings[0]["findings"].as_array().unwrap();
            assert_eq!(material.len(), 1);
            let material = &material[0];
            assert_eq!(material["algorithm"], "surface_material_v1");
            assert_eq!(material["target_id"], "step2cam-top-part");
            assert_eq!(material["complete"], true);
            assert_eq!(material["gouge_tolerance_mm"], 0.05);
            assert_eq!(material["gouge"]["assessment"], "possible_excess");
            assert_eq!(material["gouge"]["definite_cells"], 0);
            assert!(material["gouge"]["possible_only_cells"].as_u64().unwrap() > 0);
            assert_eq!(material["sweep_proof"]["tolerance_mm"], 0.05);
            assert_eq!(material["sweep_proof"]["history_complete"], true);
            assert!(material["sweep_proof"]["unproven_moves"].as_u64().unwrap() > 0);
            continue;
        }
        assert!(matches!(setup, "bottom" | "front"), "unreviewed setup {setup}");
        assert_eq!(report["released"], true);
        assert!(report["refused"].is_null());
        assert_eq!(report["accepted"], json!(["unchecked_machine"]));
        assert_eq!(report["posting"]["checked_motions"], report["motions"]);
        assert!(report["motions"].as_u64().unwrap() > 0);
        let nc = report["program"].as_str().unwrap();
        assert!(!nc.is_empty());
        assert_eq!(nc.lines().filter(|l| l.starts_with('O')).count(), 1);
        assert_eq!(nc.matches("M30").count(), 1);
        for operation in program["job"]["operations"].as_array().unwrap() {
            let number = operation["tool_number"].as_u64().unwrap();
            assert!(nc.contains(&format!("T{number}\nM6\nG43 H{number}")), "missing tool {number}: {nc}");
        }
    }
}

#[test]
fn haas_complex_plate_releases_bottom_and_front_and_refuses_top() { post_plate("synthetic/stepped-plate"); }

#[test]
fn haas_simple_plate_releases_bottom_and_refuses_top() { post_plate("plate"); }

#[test]
fn declared_gouge_tolerance_reaches_material_proof_and_the_same_posted_job() {
    let library = fixture("../../libraries/haas-vf2.json");
    assert_eq!(capp::resources::Settings::new(&library).unwrap().gouge_tolerance_mm, Some(0.05));
    let out = process_cached(&fixture("plate.analysis.json"), &fixture("plate.mesh.json"), &library).unwrap();
    let program = out["programs"].as_array().unwrap().iter().find(|p| p["setup"] == "bottom").unwrap();
    assert_eq!(program["job"]["verification"]["material"][0]["options"]["gouge_tolerance_mm"], 0.05);
    assert_eq!(program["material"]["gouge_tolerance_mm"], 0.05);
    // This is camlib's actual proof report, not just the adapter's declared input.
    assert_eq!(program["material"]["sweep_proof"]["tolerance_mm"], 0.05);
    let report = step2cam::post::post_setup(&program["job"], &library, "haas-vf2", "bottom");
    assert_eq!(report["released"], true);
    assert_eq!(report["gouge_tolerance_mm"], 0.05);
    assert_eq!(report["accepted"], json!(["unchecked_machine"]));
    assert_eq!(report["posting"]["definition"]["machine"]["tolerance_mm"], 0.001);
    let mut stale = program["job"].clone();
    stale["verification"]["material"][0]["options"].as_object_mut().unwrap().remove("gouge_tolerance_mm");
    let report = step2cam::post::post_setup(&stale, &library, "haas-vf2", "bottom");
    assert_eq!(report["released"], false);
    assert!(report["refused"]["message"].as_str().unwrap().contains("differs from the library declaration"));
}

#[test]
fn lowering_reports_its_bound_and_refuses_an_excessive_chord_budget() {
    let library = fixture("../../libraries/haas-vf2.json");
    let out = process_cached(&fixture("plate.analysis.json"), &fixture("plate.mesh.json"), &library).unwrap();
    let manifest = &out["programs"][0]["job"];
    let report = step2cam::post::post_setup(manifest, &library, "haas-vf2", "top");
    assert_eq!(report["lowering_policy"]["chord_tolerance_mm"], 0.0001);
    assert_eq!(report["lowering_policy"]["native_arcs"], true);
    assert!(report["posting"]["arc_output"]["lines_machine"].as_u64().unwrap() > 0,
        "the linear Haas profile must report why source curves became checked lines");
    let mut invalid = library;
    invalid["machines"][0]["controller_settings"]["declared_premises"]["arc_line_chord_tolerance_mm"]["value"] = json!(0.001);
    let report = step2cam::post::post_setup(manifest, &invalid, "haas-vf2", "top");
    assert_eq!(report["released"], false);
    assert!(report["program"].is_null());
    assert!(report["refused"]["message"].as_str().unwrap().contains("at most half the output tolerance_mm"));
}

#[test]
fn invalid_input_is_reported_without_release() {
    let library = fixture("../../libraries/haas-vf2.json");
    let report = step2cam::post::post_setup(&json!({}), &library, "haas-vf2", "top");
    assert_eq!(report["released"], false);
    assert!(report["program"].is_null());
    assert_eq!(report["accepted"], json!([]));
    assert!(report["findings"][0]["findings"][0]["message"].as_str().unwrap().contains("gauge Z"));
    let report = step2cam::post::post_setup(&json!({"tool_change": {"gauge_z_mm": 100, "xy_mm": [0,0]}}),
        &library, "haas-vf2", "top");
    assert_eq!(report["released"], false);
    assert!(report["program"].is_null());
    assert!(report["refused"]["message"].as_str().unwrap().contains("post job input"), "{report}");
}

#[test]
fn other_libraries_refuse_before_reading_manifest() {
    for (path, machine, reason) in [("../../demo/tools.json", "carvera", "measured spindle-to-bed height"),
        ("../../libraries/linuxcnc-vmc.json", "linuxcnc-vmc", "LinuxCNC posting is not enabled")] {
        let library: Value = fixture(path);
        let report = step2cam::post::post_setup(&Value::Null, &library, machine, "top");
        assert_eq!(report["released"], false);
        assert!(report["program"].is_null());
        assert!(report["refused"]["message"].as_str().unwrap().contains(reason), "{report}");
    }
}
