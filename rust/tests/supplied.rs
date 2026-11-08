//! Supplied choices use the same selectors, preserve provenance and replay without a caller.
mod common;
use common::*;
use serde_json::{Value, json};

fn fresh() -> Value {
    capp::plan::plan(&fixture("plate.analysis.json"), Some(&fixture("plate.mesh.json")), &Default::default()).unwrap()
}
fn supplied(choice: Value) -> Value {
    json!({"rule":"tool.hole.drill_or_bore@1", "subject":"top:holes", "choice":choice, "source":"agent:shop", "reason":"use the reserved drill"})
}
fn hole(plan: &Value) -> &Value {
    list(&plan["setups"][0]["operations"]).iter().find(|o| o["kind"] == "holes" && o["diameter"] == 4.2).unwrap()
}
fn input(plan: &Value, choice: Value) -> Value {
    let index = list(&plan["setups"][0]["operations"]).iter().position(|o| o == hole(plan)).unwrap();
    let mut d = supplied(choice);
    d["operation"] = json!(format!("top-{}-holes", index + 1));
    d
}
fn tool_decision<'a>(plan: &'a Value, op: &Value) -> &'a Value {
    list(&plan["decisions"]).iter().find(|d| d["rule"].as_str().unwrap().starts_with("tool.hole.drill_or_bore@") && list(&op["because"]).contains(&d["id"])).unwrap()
}
#[test]
fn supplied_drill_is_used_with_its_cutting_data_and_replayed() {
    let mut tools = example_tools();
    let mut alternate = list(&tools["tools"]).iter().find(|t| t["id"] == "drill4.2").unwrap().clone();
    alternate["id"] = json!("reserved");
    alternate["feed_mm_min"] = json!(123.);
    tools["tools"].as_array_mut().unwrap().push(alternate);
    let mut baseline = fresh();
    capp::finish(&mut baseline, &tools, &[]).unwrap();
    assert!(list(&baseline["decisions"]).iter().all(|d| d["source"] == "rule"));
    tools["decisions"] = json!([input(&baseline, json!("reserved"))]);
    assert!(jsonschema::validator_for(&fixture("../../schemas/resources-1.schema.json")).unwrap().is_valid(&tools));
    let mut plan = fresh();
    capp::finish(&mut plan, &tools, &[]).unwrap();
    assert_eq!(hole(&plan)["tool"], "reserved");
    assert_eq!(hole(&plan)["technology"]["feed_mm_min"], 123.);
    let d = tool_decision(&plan, hole(&plan));
    assert_eq!(d["source"], "agent:shop");
    assert_eq!(d["reason"], "use the reserved drill");
    assert!(list(&d["considered"]).iter().any(|c| c["candidate"] == "drill4.2"));
    assert_eq!(plan["supplied_decisions"][0]["decision"], d["id"]);
    assert!(jsonschema::validator_for(&fixture("../../schemas/plan-1.schema.json")).unwrap().is_valid(&plan));
    tools.as_object_mut().unwrap().remove("decisions");
    let mut replay = fresh();
    replay["supplied_decisions"] = plan["supplied_decisions"].clone();
    capp::finish(&mut replay, &tools, &[]).unwrap();
    assert_eq!(hole(&replay)["tool"], hole(&plan)["tool"]);
    assert_eq!(tool_decision(&replay, hole(&replay))["source"], "agent:shop");
    tools["decisions"] = json!([input(&baseline, json!("drill4.2"))]);
    let mut replaced = fresh();
    replaced["supplied_decisions"] = plan["supplied_decisions"].clone();
    capp::finish(&mut replaced, &tools, &[]).unwrap();
    assert_eq!(hole(&replaced)["tool"], "drill4.2");
    assert_eq!(list(&replaced["supplied_decisions"]).len(), 1);
}
#[test]
fn rejected_tools_use_selector_reasons_and_rule_fallback() {
    let mut baseline = fresh();
    capp::finish(&mut baseline, &example_tools(), &[]).unwrap();
    for choice in ["drill6.6", "spot8", "absent"] {
        let mut tools = example_tools();
        tools["decisions"] = json!([input(&baseline, json!(choice))]);
        let mut plan = fresh();
        capp::finish(&mut plan, &tools, &[]).unwrap();
        assert_eq!(hole(&plan)["tool"], "drill4.2");
        assert_eq!(plan["supplied_decisions"][0]["accepted"], false);
        let reason = plan["supplied_decisions"][0]["refusal_reason"].as_str().unwrap();
        match choice {
            "drill6.6" => {
                let considered = capp::resources::Library::new(&tools).unwrap().considered("drill", &[("diameter", 4.2), ("depth", 12.)]);
                assert!(reason.ends_with(considered.iter().find(|c| c["candidate"] == choice).unwrap()["rejected"].as_str().unwrap()));
            }
            "spot8" => assert!(reason.contains("wrong tool type")),
            _ => assert!(reason.contains("not in the library")),
        }
        assert!(warnings(&plan).iter().any(|w| w.contains(reason) && w.contains("rule's own choice is used")));
        assert_eq!(tool_decision(&plan, hole(&plan))["source"], "rule");
    }
}
#[test]
fn malformed_is_error_but_unsupported_unmatched_and_ambiguous_are_refused() {
    let mut bad_source = supplied(json!("em6"));
    bad_source["source"] = json!("robot");
    for entry in [json!(null), json!({}), json!({"rule":"unknown.rule","subject":"top","choice":"x","source":"user","reason":"test"}), bad_source] {
        let mut tools = example_tools();
        tools["decisions"] = json!([entry]);
        assert!(capp::finish(&mut fresh(), &tools, &[]).is_err());
    }
    for (rule, subject, reason) in [("tool.clearing", "top:roughing", "not suppliable"), ("tool.hole.drill_or_bore", "typo:holes", "matches no decision"), ("tool.hole.drill_or_bore", "top:holes", "more than one operation")] {
        let mut tools = example_tools();
        let mut entry = supplied(json!("em6"));
        entry["rule"] = json!(rule);
        entry["subject"] = json!(subject);
        tools["decisions"] = json!([entry]);
        let mut plan = fresh();
        capp::finish(&mut plan, &tools, &[]).unwrap();
        assert!(plan["supplied_decisions"][0]["refusal_reason"].as_str().unwrap().contains(reason));
    }
}
