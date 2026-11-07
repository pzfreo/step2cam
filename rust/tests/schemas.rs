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
    for path in ["demo/tools.json", "examples/tools.example.json"] {
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
        capp::finish(&mut planned, &read("demo/tools.json"), &[]).unwrap();
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
