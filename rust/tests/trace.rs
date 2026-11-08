//! The decision trace: every rule the code applies is in `capp/rulebase/rules.json` and every rule
//! there is applied; every warning, skipped feature and operation goes back to decisions that
//! exist; and the same inputs give the same plan.
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::{fs, path::PathBuf};

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path)
}
fn read(path: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(repo(path)).unwrap()).unwrap()
}
fn rules() -> Value {
    serde_json::from_str(capp::trace::RULES).unwrap()
}

const STAGES: [&str; 8] = ["requirement", "setup", "process", "sequence", "feasibility", "tool", "execution", "machine"];

#[test]
fn the_rules_in_the_code_are_the_rules_in_the_inventory() {
    let mut used = BTreeSet::new();
    for entry in fs::read_dir(repo("rust/capp/src")).unwrap() {
        let text = fs::read_to_string(entry.unwrap().path()).unwrap();
        // Every string literal that is a dotted name in a stage's namespace.
        for piece in text.split('"').skip(1).step_by(2) {
            let parts: Vec<&str> = piece.split('.').collect();
            let ok = parts.len() >= 2 && parts.iter().all(|p| !p.is_empty()) && parts.last() != Some(&"json") && piece.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c == '.');
            if ok && STAGES.contains(&piece.split('.').next().unwrap()) {
                used.insert(piece.to_string());
            }
        }
    }
    // Rules whose logic is data: the process cases name the rule they skip by.
    let process: Value = serde_json::from_str(capp::rules::PROCESS).unwrap();
    for case in process["cases"].as_array().unwrap() {
        if let Some(rule) = case["then"]["skip"].as_str() {
            used.insert(rule.to_string());
        }
    }
    let listed: BTreeSet<String> = rules()["rules"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
    let unlisted: Vec<_> = used.difference(&listed).collect();
    let unused: Vec<_> = listed.difference(&used).collect();
    assert!(unlisted.is_empty(), "applied in the code but not in rules.json: {unlisted:?}");
    assert!(unused.is_empty(), "in rules.json but applied nowhere: {unused:?}");
    let selectors: Value = serde_json::from_str(capp::rules::TOOLS).unwrap();
    for rule in rules()["rules"].as_array().unwrap() {
        assert!(rule["rationale"].as_str().is_some_and(|r| !r.is_empty()), "{rule}");
        assert!(rule["sources"].as_array().is_some_and(|s| !s.is_empty()), "{rule}");
        for data in rule["data"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let (file, selector) = data.split_once('#').unwrap_or((data, ""));
            assert!(["rules/tools.json", "rules/sequence.json", "rules/process.json", "rules/workholding.json"].contains(&file), "{}: {data}", rule["id"]);
            if !selector.is_empty() {
                assert!(selectors["selectors"].as_array().unwrap().iter().any(|s| s["id"] == selector), "{}: no selector {selector}", rule["id"]);
            }
        }
    }
}

const PARTS: [&str; 8] = [
    "plate", "mounting-plate", "synthetic/stepped-plate", "synthetic/stepped-plate-chamfered",
    "synthetic/walls/channel", "synthetic/walls/island", "synthetic/walls/overhang", "synthetic/walls/round-boss",
];

/// Every decision cites a rule as listed; warnings and skipped features each have theirs; every
/// operation names its decisions, among them its tool's.
fn check(plan: &Value, part: &str) {
    let versions: HashMap<String, u64> = rules()["rules"].as_array().unwrap().iter().map(|r| (r["id"].as_str().unwrap().to_string(), r["version"].as_u64().unwrap())).collect();
    let decisions: HashMap<&str, &Value> = plan["decisions"].as_array().unwrap().iter().map(|d| (d["id"].as_str().unwrap(), d)).collect();
    for d in decisions.values() {
        let (id, version) = d["rule"].as_str().unwrap().split_once('@').unwrap();
        assert_eq!(versions.get(id).copied(), Some(version.parse().unwrap()), "{part}: {d}");
    }
    let warned: Vec<&str> = plan["decisions"].as_array().unwrap().iter().filter(|d| d["outcome"] == "warning").map(|d| d["says"].as_str().unwrap()).collect();
    let warnings: Vec<&str> = plan["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert_eq!(warned, warnings, "{part}: each warning, in order, is a decision");
    for s in plan["skipped"].as_array().unwrap() {
        let d = decisions[s["decision"].as_str().unwrap_or_else(|| panic!("{part}: {s} has no decision"))];
        assert_eq!((&d["subject"], &d["outcome"], &d["says"]), (&s["feature"], &Value::from("skipped"), &s["reason"]), "{part}");
    }
    for setup in plan["setups"].as_array().unwrap() {
        for op in setup["operations"].as_array().unwrap() {
            let because: Vec<&Value> = op["because"].as_array().unwrap_or_else(|| panic!("{part}: {} has no because", op["kind"])).iter().map(|id| decisions[id.as_str().unwrap()]).collect();
            assert!(because.iter().any(|d| d["stage"] == "process" && d["outcome"] == "planned"), "{part}: {}", op["kind"]);
            assert!(because.iter().any(|d| d["stage"] == "setup"), "{part}: {}", op["kind"]);
            let tool = because.iter().find(|d| d["outcome"] == "chosen" && d.get("considered").is_some() && d["stage"] != "setup");
            match op["tool"].as_str() {
                Some(t) => assert_eq!(tool.map(|d| &d["chosen"]), Some(&Value::from(t)), "{part}: {}", op["kind"]),
                None => assert!(because.iter().any(|d| d["outcome"] == "none"), "{part}: {}", op["kind"]),
            }
        }
    }
}

#[test]
fn every_warning_skip_and_operation_goes_back_to_its_rules() {
    for library in ["demo/tools.json", "examples/tools.example.json"] {
        for part in PARTS {
            let analysis = read(&format!("tests/fixtures/{part}.analysis.json"));
            let mesh = read(&format!("tests/fixtures/{part}.mesh.json"));
            let out = step2cam::process(&analysis, &mesh, &read(library)).unwrap();
            check(&out["plan"], &format!("{part} with {library}"));
        }
    }
}

#[test]
fn a_hole_says_which_setup_and_which_tool() {
    let analysis = read("tests/fixtures/plate.analysis.json");
    let mut plan = capp::plan::plan(&analysis, Some(&read("tests/fixtures/plate.mesh.json")), &Default::default()).unwrap();
    capp::finish(&mut plan, &read("examples/tools.example.json"), &[]).unwrap();
    let decisions: HashMap<&str, &Value> = plan["decisions"].as_array().unwrap().iter().map(|d| (d["id"].as_str().unwrap(), d)).collect();
    let holes = plan["setups"][0]["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "holes" && o["tool"] == "drill4.2").unwrap();
    let rules: Vec<&str> = holes["because"].as_array().unwrap().iter().map(|id| decisions[id.as_str().unwrap()]["rule"].as_str().unwrap()).collect();
    for rule in ["setup.stock.part_box_with_margins@1", "sequence.fixed_order@1", "process.holes@1", "setup.hole.first_reachable@1", "tool.hole.drill_or_bore@2", "process.hole.peck_by_depth@2"] {
        assert!(rules.contains(&rule), "{rule} not in {rules:?}");
    }
}

#[test]
fn the_same_inputs_give_the_same_plan() {
    let library = read("demo/tools.json");
    for part in PARTS {
        let analysis = read(&format!("tests/fixtures/{part}.analysis.json"));
        let mesh = read(&format!("tests/fixtures/{part}.mesh.json"));
        let once = || {
            let mut plan = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
            capp::finish(&mut plan, &library, &[]).unwrap();
            plan.to_string()
        };
        assert_eq!(once(), once(), "{part}");
        assert!(once().contains("\"rulebase\":{\"hash\":\"fnv1a64:"), "{part}: the plan says which rulebase made it");
    }
}

/// When finishing fails (here a library naming a roughing tool it does not have), the plan still
/// has what planning said and decided.
#[test]
fn a_failed_finish_keeps_the_trace() {
    let analysis = read("tests/fixtures/plate.analysis.json");
    let mut plan = capp::plan::plan(&analysis, Some(&read("tests/fixtures/plate.mesh.json")), &Default::default()).unwrap();
    let before = plan["decisions"].as_array().unwrap().len();
    let (skipped, warnings) = (plan["skipped"].clone(), plan["warnings"].clone());
    assert!(before > 0);
    let mut library = read("examples/tools.example.json");
    library["roughing_tool"] = serde_json::json!("nope");
    assert!(capp::finish(&mut plan, &library, &[]).is_err());
    assert!(plan["decisions"].as_array().unwrap().len() >= before, "decisions kept");
    assert_eq!(plan["skipped"], skipped);
    assert_eq!(plan["warnings"], warnings);
}

/// A refused operation's own decisions are superseded by the refusal; those it shares with the
/// rest of its setup are not, and one refusal is never linked to another.
#[test]
fn a_refusal_supersedes_only_its_operations_own_decisions() {
    let analysis = read("tests/fixtures/plate.analysis.json");
    let mut plan = capp::plan::plan(&analysis, Some(&read("tests/fixtures/plate.mesh.json")), &Default::default()).unwrap();
    let library = read("examples/tools.example.json");
    let refusals = [serde_json::json!({"operation": "top-3-holes", "message": "first"}), serde_json::json!({"operation": "top-3-holes", "message": "second"})];
    capp::finish(&mut plan, &library, &refusals).unwrap();
    let decisions = plan["decisions"].as_array().unwrap();
    let refused: Vec<&Value> = decisions.iter().filter(|d| d["rule"] == "execution.refused@1" && d["outcome"] == "removed").collect();
    assert_eq!(refused.len(), 2);
    assert!(refused.iter().all(|d| d.get("superseded_by").is_none()), "both refusals stand");
    for d in decisions.iter().filter(|d| d.get("superseded_by").is_some_and(|by| refused.iter().any(|r| r["id"] == *by))) {
        assert!(d["stage"] == "process" || d["stage"] == "tools", "{d}");
    }
    let order = decisions.iter().find(|d| d["rule"] == "sequence.fixed_order@1").unwrap();
    assert!(order.get("superseded_by").is_none(), "shared with the setup's other operations");
}
