//! The part's material: Specify's answer or the resources' override, its ISO 513 group, and the
//! cutting data a tool states for that group.
use capp::reference::iso513_group;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn read(path: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path)).unwrap()).unwrap()
}

#[test]
fn materials_fall_in_their_groups() {
    let group = |m: &str| iso513_group(m).map(|(g, _)| g);
    assert_eq!(group("Aluminium 6082-T6").as_deref(), Some("N"));
    assert_eq!(group("Stainless steel 303").as_deref(), Some("M"), "stainless before steel");
    assert_eq!(group("Steel EN8 (080M40)").as_deref(), Some("P"));
    assert_eq!(group("Brass CZ121").as_deref(), Some("N"));
    assert_eq!(group("Titanium Grade 5").as_deref(), Some("S"));
    assert_eq!(group("Acetal (POM)").as_deref(), Some("N"));
    assert_eq!(group("3D printing: PLA"), None);
    assert_eq!(group("Unobtainium"), None);
}

fn plate(material: Option<&str>) -> Value {
    let mut analysis = read("tests/fixtures/plate.analysis.json");
    if let Some(m) = material {
        analysis["resume"] = json!({"answers": {"part.material": m}});
    }
    analysis
}

/// The end mill states its cutting data for aluminium; the drills do not.
fn library() -> Value {
    let mut library = read("examples/tools.example.json");
    for tool in library["tools"].as_array_mut().unwrap() {
        if tool["type"] == "end_mill" {
            tool["cutting_data"] = json!({"N": {"feed_mm_min": 1500, "spindle_rpm": 12000}});
        }
    }
    library
}

fn finished(analysis: &Value, library: &Value) -> Value {
    let mut plan = capp::plan::plan(analysis, Some(&read("tests/fixtures/plate.mesh.json")), &Default::default()).unwrap();
    capp::finish(&mut plan, library, &[]).unwrap();
    plan
}

#[test]
fn a_tool_cuts_aluminium_with_its_aluminium_data() {
    let plan = finished(&plate(Some("Aluminium 6082-T6")), &library());
    assert_eq!(plan["material"]["iso513"], "N");
    let ops = plan["setups"][0]["operations"].as_array().unwrap();
    let rough = ops.iter().find(|o| o["kind"] == "roughing").unwrap();
    assert_eq!(rough["technology"]["source"], "cutting_data.N");
    assert_eq!(rough["technology"]["feed_mm_min"], 1500);
    assert_eq!(rough["technology"]["stepdown_mm"], library()["tools"].as_array().unwrap().iter().find(|t| t["id"] == rough["tool"]).unwrap()["stepdown_mm"], "what the group does not state stays the tool's");
    let drilled = ops.iter().find(|o| o["kind"] == "holes" && o["iso14649"]["operation"] == "drilling").unwrap();
    assert_eq!(drilled["technology"]["source"], "tool");
    // And the G-code says so.
    let (jobs, _) = step2cam::job::jobs(&plan, &library()).unwrap();
    let manifest_rough = jobs[0].manifest["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "roughing").unwrap();
    assert_eq!(manifest_rough["feed_mm_min"], 1500.0);
    assert_eq!(manifest_rough["spindle_rpm"], 12000.0);
}

#[test]
fn the_resources_can_override_the_material() {
    let mut library = library();
    library["overrides"] = json!({"material": "Steel EN8"});
    let plan = finished(&plate(Some("Aluminium 6082-T6")), &library);
    assert_eq!(plan["material"]["used"], "Steel EN8");
    assert_eq!(plan["material"]["iso513"], "P");
    let rough = plan["setups"][0]["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "roughing").unwrap();
    assert_eq!(rough["technology"]["source"], "tool", "no data for P: the tool's own");
    let said = plan["decisions"].as_array().unwrap().iter().find(|d| d["rule"] == "requirement.material.group@1").unwrap();
    assert!(said["says"].as_str().unwrap().contains("override"), "{said}");
}

#[test]
fn with_no_material_the_tools_own_data_are_used() {
    let plan = finished(&plate(None), &library());
    assert_eq!(plan["material"]["iso513"], Value::Null);
    let rough = plan["setups"][0]["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "roughing").unwrap();
    assert_eq!(rough["technology"]["source"], "tool");
}
