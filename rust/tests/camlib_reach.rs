//! Reach and holder verdicts belong to camlib, including when roughing cannot run.
use serde_json::{Value, json};
fn read(path: &str) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(path),
        )
        .unwrap(),
    )
    .unwrap()
}
fn tools(flutes: f64, holder: f64) -> Value {
    let mut library = read("demo/tools.json");
    library.as_object_mut().unwrap().remove("machines");
    library["roughing_tool"] = json!("em6");
    library["tools"]
        .as_array_mut()
        .unwrap()
        .retain(|t| t["id"] == "em6");
    let tool = &mut library["tools"][0];
    tool["flute_length"] = json!(flutes);
    tool["stickout"] = json!(90.);
    tool["overall_length"] = json!(130.);
    tool["holder_diameter"] = json!(holder);
    library
}
fn run(flutes: f64, holder: f64) -> Value {
    let analysis = read("tests/fixtures/synthetic/tall-wall-pocket.analysis.json");
    assert!(
        analysis["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["family"] == "pockets")
    );
    let out = step2cam::process(
        &analysis,
        &read("tests/fixtures/synthetic/tall-wall-pocket.mesh.json"),
        &tools(flutes, holder),
    )
    .unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    out
}
#[test]
fn short_tool_reaches_camlib_and_relays_its_refusal() {
    let out = run(20., 20.);
    let plan = &out["plan"];
    let refusal = plan["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| {
            d["rule"] == "execution.refused@1"
                && d["outcome"] == "removed"
                && d["subject"].as_str().unwrap().ends_with("-notch")
        })
        .expect("camlib must judge the notch");
    let original = refusal["message"].as_str().unwrap();
    assert!(
        original.contains("pocket depth 64 exceeds the cutting length 20"),
        "{original}"
    );
    assert!(!original.contains(": needs "));
    let reason = plan["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["feature"].as_str().unwrap().starts_with("pockets:"))
        .unwrap()["reason"]
        .as_str()
        .unwrap();
    assert!(
        reason.contains("camlib refused")
            && reason.contains("64.0 mm below the stock top")
            && reason.contains("em6's flutes are 20.0 mm"),
        "{reason}"
    );
    assert!(
        reason.contains(": needs an end mill under ⌀7.95 with flutes of at least 65 mm"),
        "{reason}"
    );
    assert!(
        plan["tools_needed"]
            .to_string()
            .contains("flutes of at least 65 mm")
    );
    assert!(plan["decisions"].as_array().unwrap().iter().any(|d| {
        d["says"]
            .as_str()
            .is_some_and(|s| s.contains("reach is left to camlib"))
    }));
}
fn assert_cut(out: &Value) {
    assert!(
        out["programs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["operations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["id"].as_str().unwrap().ends_with("-notch"))),
        "{}",
        out["plan"]["warnings"]
    );
    assert!(
        !out["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["rule"] == "execution.refused@1"
                && d["subject"].as_str().is_some_and(|s| s.ends_with("-notch")))
    );
}
#[test]
fn wide_holder_is_judged_by_camlib() {
    let out = run(80., 100.);
    assert_cut(&out);
    assert!(!out["plan"]["warnings"].to_string().contains("holder"));
}
#[test]
fn fitting_tool_cuts_the_pocket() {
    assert_cut(&run(80., 12.));
}

#[test]
fn supplied_short_pocket_and_drill_choices_still_go_to_camlib() {
    let mut library = tools(20., 20.);
    let mut drill = read("demo/tools.json")["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["type"] == "drill")
        .unwrap()
        .clone();
    drill["flute_length"] = json!(2.);
    let drill_id = drill["id"].clone();
    let diameter = drill["diameter"].clone();
    library["tools"].as_array_mut().unwrap().push(drill);
    library["decisions"] = json!([
        {"rule":"tool.notch.pocket_mill@1", "subject":"top:notch", "choice":"em6", "source":"agent:shop", "reason":"test supplied reach"},
        {"rule":"tool.hole.drill_or_bore@1", "subject":"top:holes", "choice":drill_id, "source":"agent:shop", "reason":"test supplied reach"}
    ]);
    let mut plan = json!({"setups":[{"id":"top", "stock":{"min":[0,0,-70],"max":[60,40,0]}, "operations":[
        {"kind":"notch", "width":8., "top_z":0., "floor_z":-64., "features":["pockets:0"]},
        {"kind":"holes", "diameter":diameter, "top_z":0., "bottom_z":-64., "centers":[[0.,0.]], "countersinks":[null], "features":["holes:0"]}
    ]}]});
    capp::finish(&mut plan, &library, &[]).unwrap();
    assert!(
        plan["supplied_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["accepted"] == true)
    );
    for (kind, tool) in [("notch", json!("em6")), ("holes", drill_id)] {
        let op = plan["setups"][0]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["kind"] == kind)
            .unwrap();
        assert_eq!(op["tool"], tool);
        let decision = plan["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["chosen"] == tool && d["source"] == "agent:shop")
            .unwrap();
        assert!(
            decision["says"]
                .as_str()
                .unwrap()
                .contains("reach is left to camlib")
        );
        assert!(
            decision["considered"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["candidate"] == tool
                    && c["rejected"].as_str().unwrap().contains("flutes"))
        );
    }
}

#[test]
fn mesh_blocked_roughing_does_not_withhold_the_pocket() {
    let analysis = read("tests/fixtures/synthetic/tall-wall-pocket.analysis.json");
    let mut mesh = read("tests/fixtures/synthetic/tall-wall-pocket.mesh.json");
    // Without per-face triangulation ranges the planner must use specify-core's conservative
    // chordal bound, over camlib's limit for this tall part. The triangles stay unchanged.
    mesh["faces"] = json!([]);
    let out = step2cam::process(&analysis, &mesh, &tools(80., 12.)).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    assert!(
        out["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["rule"] == "process.roughing.mesh_error@1")
    );
    assert_cut(&out);
}
