use serde_json::{Value, json};
use crate::common::{fixture, process_cached};

fn post_plate(name: &str) {
    let library = fixture("../../libraries/haas-vf2.json");
    let out = process_cached(&fixture(&format!("{name}.analysis.json")),
        &fixture(&format!("{name}.mesh.json")), &library).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let programs = out["programs"].as_array().unwrap();
    assert!(!programs.is_empty());
    let mut refused = vec![];
    for program in programs {
        let setup = program["setup"].as_str().unwrap();
        let report = step2cam::post::post_setup(&program["job"], &library, "haas-vf2", setup);
        println!("{name}/{setup}: released={}, accepted={}, findings={}, refusal={}",
            report["released"], report["accepted"], report["findings"], report["refused"]);
        if report["released"] != true {
            refused.push(format!("{setup}: {report}"));
            continue;
        }
        assert_eq!(report["accepted"], json!(["unchecked_machine"]));
        let nc = report["program"].as_str().unwrap();
        assert!(!nc.is_empty());
        assert_eq!(nc.lines().filter(|l| l.starts_with('O')).count(), 1);
        assert_eq!(nc.matches("M30").count(), 1);
        for operation in program["job"]["operations"].as_array().unwrap() {
            let number = operation["tool_number"].as_u64().unwrap();
            assert!(nc.contains(&format!("T{number}\nM6\nG43 H{number}")), "missing tool {number}: {nc}");
        }
    }
    assert!(refused.is_empty(), "camlib refused unchanged {name} plans: {}", refused.join("\n"));
}

#[test]
fn haas_complex_plate_every_setup_releases() { post_plate("synthetic/stepped-plate"); }

#[test]
fn haas_simple_plate_every_setup_releases() { post_plate("plate"); }

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
