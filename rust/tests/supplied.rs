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

fn at_diameter(plan: &Value, diameter: f64) -> &Value {
    list(&plan["setups"][0]["operations"]).iter().find(|o| o["kind"] == "holes" && o["diameter"] == diameter).unwrap()
}
fn add_tool(tools: &mut Value, from: &str, name: &str, fields: &[(&str, f64)]) {
    let mut tool = list(&tools["tools"]).iter().find(|t| t["id"] == from).unwrap().clone();
    tool["id"] = json!(name);
    for (key, value) in fields { tool[*key] = json!(value); }
    tools["tools"].as_array_mut().unwrap().push(tool);
}
fn spot_for<'a>(plan: &'a Value, feature: &Value) -> Option<&'a Value> {
    list(&plan["setups"][0]["operations"]).iter().find(|o| o["kind"] == "spot_drill" && list(&o["features"]).contains(feature))
}
// Use the rule-only plan with this tool preferred as the oracle for finished-plan numbering.
// Spot insertion can change that number; a pre-spot or old-plan index is not the contract.
fn with_hole_tool(diameter: f64, tools: &Value, name: &str, oracle_tools: &Value) -> Value {
    let mut oracle = fresh();
    capp::finish(&mut oracle, oracle_tools, &[]).unwrap();
    let target = at_diameter(&oracle, diameter);
    assert_eq!(target["tool"], name);
    let number = list(&oracle["setups"][0]["operations"]).iter().position(|o| o == target).unwrap() + 1;
    let mut entry = supplied(json!(name));
    entry["operation"] = json!(format!("top-{number}-holes"));
    let mut tools = tools.clone();
    tools["decisions"] = json!([entry]);
    let mut plan = fresh();
    capp::finish(&mut plan, &tools, &[]).unwrap();
    assert_eq!(at_diameter(&plan, diameter)["tool"], name, "{}", plan["supplied_decisions"]);
    assert_eq!(plan["supplied_decisions"][0]["accepted"], true);
    let actual = list(&plan["setups"][0]["operations"]).iter().position(|o| o == at_diameter(&plan, diameter)).unwrap() + 1;
    assert_eq!(plan["supplied_decisions"][0]["operation"], format!("top-{actual}-holes"));
    let without_trace = |p: &Value| list(&p["setups"][0]["operations"]).iter().cloned().map(|mut op| { op.as_object_mut().unwrap().remove("because"); op }).collect::<Vec<_>>();
    assert_eq!(without_trace(&plan), without_trace(&oracle), "same operations and cutting data as the rules with that tool preferred");
    let mut replay = fresh();
    replay["supplied_decisions"] = plan["supplied_decisions"].clone();
    tools.as_object_mut().unwrap().remove("decisions");
    capp::finish(&mut replay, &tools, &[]).unwrap();
    assert_eq!(replay, plan);
    plan
}

#[test]
fn supplied_boring_mill_removes_spots_for_its_hole_features() {
    let tools = example_tools();
    let mut oracle = tools.clone();
    oracle["tools"].as_array_mut().unwrap().retain(|t| t["id"] != "drill4.2");
    let plan = with_hole_tool(4.2, &tools, "em3", &oracle);
    for feature in list(&at_diameter(&plan, 4.2)["features"]) { assert!(spot_for(&plan, feature).is_none()); }
    for feature in list(&at_diameter(&plan, 6.6)["features"]) { assert!(spot_for(&plan, feature).is_some()); }
}

#[test]
fn supplied_drill_spots_a_previously_bored_plate_hole() {
    let mut tools = example_tools();
    let mut before = fresh();
    capp::finish(&mut before, &tools, &[]).unwrap();
    assert_eq!(at_diameter(&before, 8.)["tool"], "em6");
    for feature in list(&at_diameter(&before, 8.)["features"]) { assert!(spot_for(&before, feature).is_none()); }
    add_tool(&mut tools, "drill6.6", "drill8", &[("diameter", 8.)]);
    let plan = with_hole_tool(8., &tools, "drill8", &tools);
    for feature in list(&at_diameter(&plan, 8.)["features"]) { assert!(spot_for(&plan, feature).is_some()); }
}

#[test]
fn supplied_drill_point_angle_selects_a_covering_spot_and_final_operation_number() {
    let mut tools = example_tools();
    add_tool(&mut tools, "drill4.2", "drill140", &[("point_angle", 140.)]);
    add_tool(&mut tools, "spot8", "spot140", &[("point_angle", 140.)]);
    let mut oracle = tools.clone();
    oracle["tools"].as_array_mut().unwrap().retain(|t| t["id"] != "drill4.2");
    let plan = with_hole_tool(4.2, &tools, "drill140", &oracle);
    for feature in list(&at_diameter(&plan, 4.2)["features"]) {
        let spot = spot_for(&plan, feature).unwrap();
        assert_eq!(spot["tool"], "spot140");
        assert!(number(&spot["angle"]) >= 140.);
        let tool = list(&tools["tools"]).iter().find(|t| t["id"] == spot["tool"]).unwrap();
        assert!(number(&tool["point_angle"]) >= 140.);
    }
}

/// Spot drilling no tool cuts is not in the finished plan, so it does not number the holes after it.
#[test]
fn supplied_hole_numbers_skip_spot_drilling_no_tool_cuts() {
    let mut tools = example_tools();
    tools["tools"].as_array_mut().unwrap().retain(|t| t["type"] != "spot_drill");
    add_tool(&mut tools, "drill4.2", "reserved", &[]);
    let mut oracle = tools.clone();
    oracle["tools"].as_array_mut().unwrap().retain(|t| t["id"] != "drill4.2");
    let plan = with_hole_tool(4.2, &tools, "reserved", &oracle);
    assert!(list(&plan["setups"][0]["operations"]).iter().all(|o| o["kind"] != "spot_drill"));
    assert!(list(&plan["skipped"]).iter().any(|s| s["reason"].as_str().unwrap().starts_with("spot drilling left out: ")));
    // A supplied 140° drill no spot drill covers: its spots are left out, the other holes' stay.
    let mut tools = example_tools();
    add_tool(&mut tools, "drill4.2", "drill140", &[("point_angle", 140.)]);
    let mut oracle = tools.clone();
    oracle["tools"].as_array_mut().unwrap().retain(|t| t["id"] != "drill4.2");
    let plan = with_hole_tool(4.2, &tools, "drill140", &oracle);
    for feature in list(&at_diameter(&plan, 4.2)["features"]) { assert!(spot_for(&plan, feature).is_none()); }
    for feature in list(&at_diameter(&plan, 6.6)["features"]) { assert_eq!(spot_for(&plan, feature).unwrap()["tool"], "spot8"); }
}

#[test]
fn removing_the_last_spot_uses_the_finished_hole_number_and_refuses_inconsistent_names() {
    let mut tools = example_tools();
    let fresh = || capp::plan::plan(&holes_only(&[down(0., 0., 4.2, 8., "flat")]), None, &top_stock(0.)).unwrap();
    for (number, accepted) in [(1, true), (2, false)] {
        let mut entry = supplied(json!("em3"));
        entry["operation"] = json!(format!("top-{number}-holes"));
        tools["decisions"] = json!([entry]);
        let mut plan = fresh();
        capp::finish(&mut plan, &tools, &[]).unwrap();
        assert_eq!(plan["supplied_decisions"][0]["accepted"], accepted);
        let ops = list(&plan["setups"][0]["operations"]);
        if accepted {
            assert_eq!(ops.len(), 1);
            assert_eq!(ops[0]["tool"], "em3");
            let mut replay = fresh();
            replay["supplied_decisions"] = plan["supplied_decisions"].clone();
            let mut replay_tools = tools.clone();
            replay_tools.as_object_mut().unwrap().remove("decisions");
            capp::finish(&mut replay, &replay_tools, &[]).unwrap();
            assert_eq!(replay, plan);
        } else {
            assert_eq!(ops.len(), 2);
            assert_eq!(ops[1]["tool"], "drill4.2");
            assert!(plan["supplied_decisions"][0]["refusal_reason"].as_str().unwrap().contains("numbering"));
        }
    }
}

#[test]
fn an_unavailable_channel_reports_its_feasibility_reason() {
    let mut tools = example_tools();
    tools["decisions"] = json!([{"rule":"tool.v_channel.v_bit", "subject":"top:v_channel", "choice":"absent", "source":"user", "reason":"test"}]);
    let mut plan = json!({"setups":[{"id":"top", "stock":{"min":[-20,-20,-10],"max":[20,20,0]}, "operations":[{"kind":"v_channel", "angle":90., "width":4., "depth":2., "length":40., "a":[-20.,0.], "b":[20.,0.], "top_z":0., "root_z":-2., "on_flat":true, "features":[]}]}],
        "covered_by_roughing":[], "skipped":[], "warnings":[]});
    capp::finish(&mut plan, &tools, &[]).unwrap();
    let reason = plan["supplied_decisions"][0]["refusal_reason"].as_str().unwrap();
    assert!(reason.contains("operation unavailable") && reason.contains("flat it is cut in has not been cut"), "{reason}");
}

#[test]
fn an_excluded_spot_is_not_recreated_for_a_supplied_drill() {
    let mut tools = example_tools();
    add_tool(&mut tools, "drill4.2", "drill140", &[("point_angle", 140.)]);
    add_tool(&mut tools, "spot8", "spot140", &[("point_angle", 140.)]);
    tools["decisions"] = json!([supplied(json!("drill140"))]);
    let fresh = || capp::plan::plan(&holes_only(&[down(0., 0., 4.2, 8., "flat")]), None, &top_stock(0.)).unwrap();
    let mut initial = fresh();
    capp::finish(&mut initial, &tools, &[]).unwrap();
    assert_eq!(initial["setups"][0]["operations"][0]["tool"], "spot140");
    let mut plan = fresh();
    capp::finish(&mut plan, &tools, &[json!({"operation":"top-1-spot_drill", "message":"spot refused"})]).unwrap();
    let ops = list(&plan["setups"][0]["operations"]);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["tool"], "drill140");
    assert_eq!(plan["supplied_decisions"][0]["accepted"], true);
}
