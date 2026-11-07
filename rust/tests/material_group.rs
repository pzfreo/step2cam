//! The part's material: Specify's answer or the resources' override, its ISO 513 group, and the
//! cutting data a tool states for that group.
use fettle_planner::reference::iso513_group;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn read(path: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path)).unwrap()).unwrap()
}

/// Names as specify-core offers them (its src/materials.rs), with the group each belongs in.
#[test]
fn specify_cores_materials_fall_in_their_groups() {
    let group = |m: &str| iso513_group(m).map(|(g, _)| g);
    let cases: &[(&str, Option<&str>)] = &[
        ("Aluminium 6082-T6", Some("N")), ("Aluminium MIC-6 cast plate", Some("N")),
        ("Stainless steel 303 (1.4305)", Some("M")), ("Stainless steel 17-4PH (1.4542)", Some("M")),
        ("Steel 4140 (42CrMo4)", Some("P")), ("Silver steel (115CrV3)", Some("P")), ("Tool steel D2 (1.2379)", Some("P")),
        ("Brass CZ121 (CW614N)", Some("N")), ("Aluminium bronze CA104 (CW307G)", Some("N")), ("Beryllium copper C17200 (CW101C)", Some("N")),
        ("Titanium grade 5 Ti-6Al-4V (3.7165)", Some("S")), ("Inconel 718 (2.4668)", Some("S")), ("Monel 400 (2.4360)", Some("S")), ("Hastelloy C-276 (2.4819)", Some("S")),
        ("Cast iron EN-GJL-250", Some("K")), ("Ductile iron EN-GJS-500-7", Some("K")),
        ("Magnesium AZ31B", Some("N")), ("Zinc alloy Zamak 3", Some("N")),
        ("POM-C acetal copolymer", Some("N")), ("PA66 30% glass-filled", Some("N")), ("PEEK", Some("N")), ("PVC rigid", Some("N")), ("PET", Some("N")),
        ("PPSU", Some("N")), ("Garolite G-10 / FR-4", Some("N")), ("Tufnol Whale (paper phenolic)", Some("N")), ("Polyimide (Vespel)", Some("N")),
        ("UHMW-PE", Some("N")), ("PP polypropylene", Some("N")), ("PTFE, glass-filled (Rulon)", Some("N")),
        // Printed: not cut from stock as these groups mean.
        ("PLA (FDM)", None), ("ABS (FDM)", None), ("PA12 (MJF)", None), ("Tough resin (SLA)", None),
        ("Stainless steel 316L (DMLS)", None), ("Titanium Ti-6Al-4V (DMLS)", None), ("Maraging steel MS1 (DMLS)", None),
        // Nothing places it; and whole words only.
        ("Tungsten heavy alloy", None), ("Unobtainium", None), ("Competition grade", None),
    ];
    for (name, want) in cases {
        assert_eq!(group(name).as_deref(), *want, "{name}");
    }
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
    let mut plan = fettle_planner::plan::plan(analysis, Some(&read("tests/fixtures/plate.mesh.json")), &Default::default()).unwrap();
    fettle_planner::finish(&mut plan, library, &[]).unwrap();
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
