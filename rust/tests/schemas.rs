//! step2cam's own documents against their schemas in `schemas/`: the tool libraries as shipped,
//! and the plan every fixture plans to, as plan::plan writes it and as the frozen goldens hold it.
use serde_json::Value;
use std::{fs, path::PathBuf};
use capp::plan::{Options, plan};

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path)
}
fn read(path: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(repo(path)).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap()
}
fn check(schema: &str, what: &str, doc: &Value) {
    let validator = jsonschema::validator_for(&read(schema)).unwrap();
    let errors: Vec<String> = validator.iter_errors(doc).map(|e| format!("{} at {}", e, e.instance_path())).take(5).collect();
    assert!(errors.is_empty(), "{what} does not match {schema}:\n{}", errors.join("\n"));
}

#[test]
fn tool_libraries_match_the_resources_schema() {
    for path in ["demo/tools.json", "examples/tools.example.json", "libraries/haas-vf2.json", "libraries/linuxcnc-vmc.json"] {
        check("schemas/resources-1.schema.json", path, &read(path));
    }
}

#[test]
fn plans_match_the_plan_schema() {
    let fixtures = [
        ("tests/fixtures/plate", true),
        ("tests/fixtures/mounting-plate", true),
        ("tests/fixtures/spindle", false),
        ("tests/fixtures/synthetic/stepped-plate", true),
        ("tests/fixtures/synthetic/stepped-plate-chamfered", true),
        ("tests/fixtures/synthetic/stepped-plate-large-chamfer", true),
        ("tests/fixtures/synthetic/stepped-plate-chamfer-mouths", true),
        ("tests/fixtures/synthetic/square-slots", true),
        ("tests/fixtures/synthetic/walls/channel", true),
        ("tests/fixtures/synthetic/walls/island", true),
        ("tests/fixtures/synthetic/walls/overhang", true),
        ("tests/fixtures/synthetic/walls/round-boss", true),
    ];
    for (name, meshed) in fixtures {
        let analysis = read(&format!("{name}.analysis.json"));
        let mesh = meshed.then(|| read(&format!("{name}.mesh.json")));
        let mut planned = plan(&analysis, mesh.as_ref(), &Options::default()).unwrap();
        check("schemas/plan-1.schema.json", name, &planned);
        capp::finish(&mut planned, &read("tests/fixtures/placeholder-tools.json"), &[]).unwrap();
        check("schemas/plan-1.schema.json", &format!("{name}, finished"), &planned);
    }
    for golden in ["plate", "plate-thread", "spindle"] {
        let path = format!("tests/fixtures/rust-golden/{golden}.plan.json");
        check("schemas/plan-1.schema.json", &path, &read(&path));
    }
}

#[test]
fn a_mistyped_tool_field_is_caught() {
    let mut library = read("demo/tools.json");
    library["tools"][0]["stepdwon_mm"] = serde_json::json!(4);
    let validator = jsonschema::validator_for(&read("schemas/resources-1.schema.json")).unwrap();
    assert!(!validator.is_valid(&library));
}

#[test]
fn controller_metadata_is_optional_and_machine_typos_are_rejected() {
    let mut library = read("libraries/haas-vf2.json");
    let validator = jsonschema::validator_for(&read("schemas/resources-1.schema.json")).unwrap();
    assert!(validator.is_valid(&library));
    // Controller-free entries remain valid, but cannot retain Haas-only settings.
    library["machines"][0].as_object_mut().unwrap().remove("controller_settings");
    library["machines"][0].as_object_mut().unwrap().remove("controller");
    assert!(validator.is_valid(&library));
    library["machines"][0]["controller"] = serde_json::json!(42);
    assert!(!validator.is_valid(&library));
    library["machines"][0]["controller"] = serde_json::json!("haas_ngc");
    library["machines"][0]["controler"] = serde_json::json!("haas_ngc");
    assert!(!validator.is_valid(&library));
}

#[test]
fn spindle_collet_change_mode_is_optional_but_checked() {
    let mut library = read("demo/tools.json");
    let validator = jsonschema::validator_for(&read("schemas/resources-1.schema.json")).unwrap();
    assert!(validator.is_valid(&library));
    library["machines"][0]["spindle"].as_object_mut().unwrap().remove("collet_change");
    assert!(validator.is_valid(&library));
    library["machines"][0]["spindle"]["collet_change"] = serde_json::json!("automatic");
    assert!(validator.is_valid(&library));
    library["machines"][0]["spindle"]["collet_change"] = serde_json::json!("maybe");
    assert!(!validator.is_valid(&library));
}

#[test]
fn simulated_workholding_candidates_match_schema_and_catch_missing_costs() {
    let out=step2cam::process(&read("tests/fixtures/synthetic/stepped-plate.analysis.json"),&read("tests/fixtures/synthetic/stepped-plate.mesh.json"),&read("demo/tools.json")).unwrap();
    check("schemas/plan-1.schema.json","simulated workholding",&out["plan"]);
    let mut bad=out["plan"].clone();
    bad["workholding"]["candidates"][0].as_object_mut().unwrap().remove("estimated_seconds");
    assert!(!jsonschema::validator_for(&read("schemas/plan-1.schema.json")).unwrap().is_valid(&bad));
    let validator=jsonschema::validator_for(&read("schemas/plan-1.schema.json")).unwrap();
    let mut obsolete=out["plan"].clone();
    obsolete["workholding"]["candidates"][0]["simulation"]=serde_json::json!("camlib passed");
    assert!(!validator.is_valid(&obsolete),"a blanket simulation claim is not part of the candidate schema");
    let mut missing=out["plan"].clone();
    missing["workholding"]["candidates"][0].as_object_mut().unwrap().remove("tools_needed");
    assert!(!validator.is_valid(&missing));
    let mut malformed=out["plan"].clone();
    malformed["workholding"]["candidates"][0]["tools_needed"]=serde_json::json!([{"description":"a thread mill"}]);
    assert!(!validator.is_valid(&malformed),"tool needs must retain features and operations");
}

#[test]
fn controller_settings_are_typed_and_haas_settings_require_haas_identity() {
    let validator = jsonschema::validator_for(&read("schemas/resources-1.schema.json")).unwrap();
    let mut library = read("libraries/haas-vf2.json");
    assert!(validator.is_valid(&library));
    for controller in [serde_json::json!("linuxcnc"), serde_json::json!("carvera"), Value::Null] {
        library["machines"][0]["controller"] = controller;
        assert!(!validator.is_valid(&library), "Haas option must require Haas identity");
    }
    library["machines"][0].as_object_mut().unwrap().remove("controller");
    assert!(!validator.is_valid(&library));
    library["machines"][0]["controller"] = serde_json::json!("haas_ngc");
    for value in [serde_json::json!(-1), Value::Null, serde_json::json!("16500")] {
        library["machines"][0]["controller_settings"]["max_cutting_feed_mm_min"] = value;
        assert!(!validator.is_valid(&library));
    }
    let mut library = read("libraries/linuxcnc-vmc.json");
    library["machines"][0]["controller_settings"]["tool_change_location"] = serde_json::json!("machine_z_home");
    assert!(!validator.is_valid(&library));
    library["machines"][0]["controller_settings"]["tool_change_gauge_home_z_mm"] = serde_json::json!(0);
    assert!(validator.is_valid(&library));
    library["machines"][0]["controller_settings"]["highest_tool_numer"] = serde_json::json!(24);
    assert!(!validator.is_valid(&library));
}

#[test]
fn operation_kinds_are_the_schemas_and_the_sequences() {
    use capp::kind::OpKind;
    let names: Vec<&str> = OpKind::ALL.iter().map(|k| k.name()).collect();
    let schema = read("schemas/plan-1.schema.json");
    let mut kinds: Vec<&str> = schema["$defs"]["operation"]["oneOf"].as_array().unwrap().iter()
        .map(|op| op["properties"]["kind"]["const"].as_str().unwrap()).collect();
    kinds.sort();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(kinds, sorted, "the schema's operation kinds");
    let sequence: Vec<&str> = capp::rules::sequence()["order"].as_array().unwrap().iter()
        .flat_map(|group| group.as_array().unwrap()).map(|k| k.as_str().unwrap()).collect();
    assert_eq!(sequence, names, "rulebase/rules/sequence.json, in order");
    for kind in OpKind::ALL {
        assert_eq!(serde_json::to_value(kind).unwrap(), kind.name());
        assert_eq!(serde_json::from_value::<OpKind>(kind.name().into()).unwrap(), kind);
    }
}
