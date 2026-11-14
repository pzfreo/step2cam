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
    let mut library = read("tests/fixtures/placeholder-tools.json");
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
        reason.starts_with("left out: camlib refused the notch: ")
            && reason.contains("64.0 mm below the stock top")
            && reason.contains("em6's flutes are 20.0 mm"),
        "{reason}"
    );
    assert!(
        !reason.contains(refusal["subject"].as_str().unwrap()),
        "{reason}"
    );
    let operation = refusal["subject"].as_str().unwrap();
    let warning = plan["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|w| w.starts_with(&format!("{operation} left out: camlib refused the notch: ")))
        .expect("the warning names the operation once, outside the reason");
    assert_eq!(warning.matches(operation).count(), 1, "{warning}");
    assert!(
        warning.ends_with(reason.strip_prefix("left out: ").unwrap()),
        "{warning}"
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
fn long_stickout_wide_holder_clears_above_the_wall() {
    let out = run(80., 100.);
    assert_cut(&out);
    assert!(!out["plan"]["warnings"].to_string().contains("holder"));
}
// Roughing uses a long, wide cutter; the narrow pocket needs a second assembly.
fn local_pocket(flutes: f64, holder: f64) -> Value {
    let mut library = tools(flutes, holder);
    library["tools"][0]["stickout"] = json!(30.);
    let mut rougher = read("tests/fixtures/placeholder-tools.json")["tools"].as_array().unwrap()
        .iter().find(|t| t["id"] == "em12").unwrap().clone();
    rougher["flute_length"] = json!(80.);
    rougher["stickout"] = json!(90.);
    rougher["overall_length"] = json!(130.);
    library["roughing_tool"] = json!("em12");
    library["tools"].as_array_mut().unwrap().push(rougher);
    let out = step2cam::process(&read("tests/fixtures/synthetic/tall-wall-pocket.analysis.json"),
        &read("tests/fixtures/synthetic/tall-wall-pocket.mesh.json"), &library).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    out
}
#[test]
fn local_pocket_holder_and_flutes_are_judged_by_camlib() {
    for (flutes, holder, limit) in [(2.,12.,"cutting_length"), (20.,100.,"holder")] {
        let out = local_pocket(flutes, holder);
        let d = out["plan"]["decisions"].as_array().unwrap().iter().find(|d|
            d["rule"] == "execution.refused@1" && d["outcome"] == "removed"
                && d["subject"].as_str().is_some_and(|s| s.ends_with("-notch")))
            .expect("camlib must refuse this pocket assembly");
        let evidence = camlib::reach::ReachRefusal::parse(d["message"].as_str().unwrap()).unwrap();
        assert_eq!(serde_json::to_value(evidence.limit).unwrap(), limit);
        assert_eq!(evidence.tool, "em6");
        let reasons = out["plan"]["skipped"].to_string();
        assert!(reasons.contains(if limit == "holder" { "em6's holder" } else { "em6's flutes" }), "{reasons}");
        assert!(reasons.contains(": needs "), "{reasons}");
    }
    let fitting = local_pocket(20.,12.);
    assert_cut(&fitting);
    let operation = fitting["programs"].as_array().unwrap().iter()
        .flat_map(|p| p["job"]["operations"].as_array().unwrap()).find(|o|
            o["id"].as_str().is_some_and(|id| id.ends_with("-notch"))).unwrap();
    let planned = fitting["plan"]["setups"].as_array().unwrap().iter()
        .flat_map(|s| s["operations"].as_array().unwrap()).find(|o| o["kind"] == "notch").unwrap();
    assert_eq!(operation["top_z_mm"], planned["wall_top"], "record the mouth camlib actually admitted");
    assert!(operation["top_z_mm"].as_f64().unwrap() < planned["top_z"].as_f64().unwrap());
}
#[test]
fn fitting_tool_cuts_the_pocket() {
    assert_cut(&run(80., 12.));
}

#[test]
fn supplied_short_pocket_and_drill_choices_still_go_to_camlib() {
    let mut library = tools(20., 20.);
    let mut drill = read("tests/fixtures/placeholder-tools.json")["tools"]
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
    // Supplied short tools remain explicit choices even when a normal alternative qualifies.
    let mut alternate = read("tests/fixtures/placeholder-tools.json")["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "em1.5")
        .unwrap()
        .clone();
    alternate["flute_length"] = json!(80.);
    alternate["stickout"] = json!(90.);
    alternate["overall_length"] = json!(130.);
    library["tools"].as_array_mut().unwrap().push(alternate);
    library["decisions"] = json!([
        {"rule":"tool.notch.pocket_mill@1", "subject":"top:notch", "choice":"em6", "source":"agent:shop", "reason":"test supplied reach"},
        {"rule":"tool.hole.drill_or_bore@1", "subject":"top:holes", "choice":drill_id, "source":"agent:shop", "reason":"test supplied reach"}
    ]);
    let mut plan = json!({"setups":[{"id":"top", "stock":{"min":[0,0,-70],"max":[60,40,0]}, "operations":[
        {"kind":"notch", "width":8., "length":8., "open":false, "outer":[], "top_z":0., "floor_z":-64., "wall_top":0., "bottom":"blind", "features":["pockets:0"]},
        {"kind":"holes", "diameter":diameter, "top_z":0., "bore_top_z":0., "bottom_z":-64., "bottom":"blind", "centers":[[0.,0.]], "countersinks":[null], "features":["holes:0"]}
    ]}], "covered_by_roughing":[], "skipped":[], "warnings":[]});
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

#[test]
fn short_drill_preserves_a_cuttable_bore_and_is_deferred_without_one() {
    for can_bore in [true, false] {
        let mut library = read("tests/fixtures/placeholder-tools.json");
        library.as_object_mut().unwrap().remove("machines");
        library["tools"]
            .as_array_mut()
            .unwrap()
            .retain(|t| t["id"] == "drill4.2" || (can_bore && t["id"] == "em3"));
        let drill = library["tools"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|t| t["id"] == "drill4.2")
            .unwrap();
        drill["flute_length"] = json!(2.);
        let out = step2cam::process(
            &read("tests/fixtures/plate.analysis.json"),
            &read("tests/fixtures/plate.mesh.json"),
            &library,
        )
        .unwrap();
        assert!(out["error"].is_null(), "{}", out["error"]);
        if can_bore {
            let setup = &out["plan"]["setups"][0];
            let (index, hole) = setup["operations"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .find(|(_, o)| o["kind"] == "holes" && o["diameter"] == 4.2)
                .expect("a usable bore must keep the hole");
            assert_eq!(hole["tool"], "em3");
            let operation = format!("{}-{}-holes", setup["id"].as_str().unwrap(), index + 1);
            assert!(
                out["programs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p["operations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|o| o["id"] == operation)),
                "the bore must be simulated and cut"
            );
        } else {
            let refusal = out["plan"]["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| {
                    d["rule"] == "execution.refused@1"
                        && d["outcome"] == "removed"
                        && d["message"].as_str().is_some_and(|m| {
                            m.contains("hole depth") && m.contains("exceeds the cutting length 2")
                        })
                })
                .expect("camlib must judge the only available short drill");
            assert!(
                refusal["says"]
                    .as_str()
                    .unwrap()
                    .contains("camlib refused the holes: ")
            );
            assert!(
                refusal["says"]
                    .as_str()
                    .unwrap()
                    .contains("drill4.2's flutes are 2.0 mm")
            );
            assert!(
                out["plan"]["tools_needed"]
                    .as_array()
                    .is_some_and(|ns| !ns.is_empty())
            );
        }
    }
}

#[test]
fn haas_complex_plate_holder_sentence_and_tool_query_share_camlib_evidence() {
    let out = step2cam::process(
        &read("tests/fixtures/synthetic/stepped-plate.analysis.json"),
        &read("tests/fixtures/synthetic/stepped-plate.mesh.json"),
        &read("libraries/haas-vf2.json"),
    ).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let need = "an end mill under ⌀5.95 with flutes of at least 13 mm and its holder at least 19 mm above the tip";
    let sentence = format!("left out: camlib refused the notch: em3's holder section 0 cannot clear the remaining stock beside the notch at Z-1 (tip Z-19.5): needs {need}");
    let skipped: Vec<_> = out["plan"]["skipped"].as_array().unwrap().iter().filter(|s|
        s["reason"].as_str().is_some_and(|r| r.contains("em3's holder"))).collect();
    assert_eq!(skipped.len(), 7);
    for feature in skipped { assert_eq!(feature["reason"], sentence); }
    assert!(out["plan"]["tools_needed"].as_array().unwrap().iter().any(|n| n["description"] == need));
    let refusal = out["plan"]["decisions"].as_array().unwrap().iter().find(|d|
        d["rule"] == "execution.refused@1" && d["outcome"] == "removed"
            && d["message"].as_str().is_some_and(|m| m.contains("\"limit\":\"holder\""))).unwrap();
    let evidence = camlib::reach::ReachRefusal::parse(refusal["message"].as_str().unwrap()).unwrap();
    assert_eq!(evidence.top_z_mm, -0.99999);
    assert_eq!(evidence.depth_needed_mm, 18.50001);
}
