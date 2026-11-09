//! Tap process selection and the pinned camlib rigid-tapping contract.
mod common;
use common::*;
use serde_json::{Value, json};

fn tools(tap: Value) -> Value {
    let mut tools = example_tools();
    tools["tools"].as_array_mut().unwrap().push(tap);
    tools["overrides"]["material"] = json!("6082-T6");
    tools
}
fn plan(name: &str) -> Value {
    capp::plan::plan(&fixture(name), None, &Default::default()).unwrap()
}
fn thread(plan: &Value) -> &Value {
    plan["setups"].as_array().unwrap().iter().flat_map(|s| list(&s["operations"])).find(|o| o["kind"] == "thread").unwrap()
}
fn decisions(plan: &Value) -> String { plan["decisions"].to_string() }

#[test]
fn plate_m5_tap_is_selected_and_camlib_completes_it() {
    let tools = tools(example_tap(5., 0.8, 2.));
    let p = finished(&plan("plate-m5.analysis.json"), &tools);
    assert_eq!(thread(&p)["tool"], "tap");
    assert!(decisions(&p).contains("matching pitch, hand and pre-drill"));
    let (ops, cutters, notes) = posted(&p, &tools);
    let op = ops.iter().find(|o| o["kind"] == "tapping").unwrap();
    assert_eq!(op["pitch_mm"], 0.8);
    assert_eq!(op["hand"], "right");
    assert_eq!(op["spindle_rpm"], 700.);
    assert_close(number(&op["depth_z_mm"]), number(&thread(&p)["bottom_z"]) - 1.6);
    assert!(op.get("feed_mm_min").is_none());
    let cutter = cutters.iter().find(|t| t["id"] == "tap").unwrap();
    assert_eq!(cutter["cutter"]["diameter_mm"], 4.2);
    assert!(any_note(&notes, "tapped M5"));
    // plate-m5 differs only in its saved answers; reuse the plate triangulation with
    // its own binding so this fixture exercises the real process/simulation path.
    let analysis = fixture("plate-m5.analysis.json");
    let mut mesh = fixture("plate.mesh.json");
    mesh["binding"] = analysis["binding"].clone();
    let out = step2cam::process(&analysis, &mesh, &tools).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let program = &out["programs"][0];
    let executed = list(&program["job"]["operations"]).iter().find(|o| o["kind"] == "tapping").unwrap_or_else(|| panic!("{}", out["plan"]["warnings"]));
    let tapping = list(&program["operations"]).iter().find(|o| o["id"] == executed["id"]).unwrap();
    assert!(number(&tapping["end"]) > number(&tapping["start"]), "{tapping}");
    let moves: Vec<_> = list(&program["moves"]).iter().filter(|m| m["kind"] == "tap").collect();
    assert_eq!(moves.len(), 2);
    assert!(moves.iter().all(|m| m["feed"] == 560.));
}

#[test]
fn stepped_m3_chamfer_and_form_predrill_are_checked_with_numbers() {
    let mut tools = tools(example_tap(3., 0.5, 1.));
    // The example library lacks the 2.5 drill; add one only to this test library.
    let mut drill = example_tools()["tools"].as_array().unwrap().iter().find(|t| t["type"] == "drill").unwrap().clone();
    drill["id"] = json!("drill2.5"); drill["diameter"] = json!(2.5); drill["flute_length"] = json!(30);
    tools["tools"].as_array_mut().unwrap().push(drill);
    let p = plan("synthetic/stepped-plate.analysis.json");
    let settled = finished(&p, &tools);
    let op = thread(&settled);
    assert_eq!(op["major"], 3.);
    assert_eq!(op["tool"], "tap", "{}", decisions(&settled));
    assert_close(number(&op["top_z"]) - number(&op["bottom_z"]), 8.5);
    let (jobs, _) = step2cam::job::jobs(&settled, &tools).unwrap();
    let tapping = jobs.iter().flat_map(|j| list(&j.manifest["operations"])).find(|o| o["kind"] == "tapping").unwrap();
    assert_close(number(&tapping["depth_z_mm"]), number(&op["bottom_z"]) - 0.5);
    let front = jobs.iter().find(|j| j.setup == "front").unwrap();
    let run = step2cam::simulate::simulate(&front.manifest.to_string()).unwrap();
    // The fixture represents two bores (face pairs 48/57 and 49/58), two tap moves each.
    assert_eq!(run.moves.iter().filter(|m| matches!(m.kind, camlib::motion::MoveKind::Tap)).count(), 4);

    // A plug lead of 4 pitches needs 8.5 + 2 + 0.5 = 11 mm; only 10 exists.
    tools["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["type"] == "tap").unwrap()["chamfer_pitches"] = json!(4);
    let rejected = finished(&p, &tools);
    let text = decisions(&rejected);
    assert!(text.contains("requires 11.00 mm; drilled full diameter is 10.00 mm"), "{text}");
    assert_ne!(thread(&rejected)["tool"], "tap");
    let t = tools["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["type"] == "tap").unwrap();
    t["tap_type"] = json!("form"); t["chamfer_pitches"] = json!(1);
    let text = decisions(&finished(&p, &tools));
    assert!(text.contains("form tap requires ⌀2.78 ±0.02 mm; model hole is ⌀2.5 mm"), "{text}");
}

#[test]
fn hard_material_prefers_milling_and_no_tap_preserves_the_plan() {
    let p = plan("plate-m5.analysis.json");
    let no_tap = example_tools();
    let mut with_tap = tools(example_tap(5., 0.8, 2.));
    with_tap["overrides"]["material"] = json!("hardened steel");
    let chosen = finished(&p, &with_tap);
    assert_eq!(thread(&chosen)["tool"], "tm3");
    assert!(decisions(&chosen).contains("thread milling preferred"));
    let original = finished(&p, &no_tap);
    let mut unusable = no_tap.clone();
    unusable["tools"].as_array_mut().unwrap().push(example_tap(6., 1., 2.));
    let other = finished(&p, &unusable);
    let (a, _, _) = posted(&original, &no_tap);
    let (b, _, _) = posted(&other, &unusable);
    assert_eq!(a, b);
    assert_eq!(thread(&original)["tool"], "tm3");
}

#[test]
fn missing_tool_offers_both_processes_and_plain_conservative_subtraction() {
    let mut tools = example_tools();
    tools["tools"].as_array_mut().unwrap().retain(|t| t["type"] != "thread_mill");
    let p = finished(&plan("plate-m5.analysis.json"), &tools);
    let text = thread(&p)["tool_note"].as_str().unwrap();
    assert!(text.contains("M5×0.8 cut tap"), "{text}");
    assert!(text.contains("or a thread mill"), "{text}");
    assert!(text.contains("cutter diameter − 0.82"), "{text}");
    assert!(!text.contains("+ -"));
}

#[test]
fn tap_schema_library_and_iso_aliases_validate() {
    let schema = jsonschema::validator_for(&fixture("../../schemas/resources-1.schema.json")).unwrap();
    let library = tools(example_tap(5., 0.8, 2.));
    assert!(schema.is_valid(&library), "{:?}", schema.iter_errors(&library).collect::<Vec<_>>());
    capp::resources::Library::new(&library).unwrap();
    for field in ["pitch", "tap_type", "chamfer_form", "chamfer_pitches", "thread_length", "shank_diameter"] {
        let mut bad = library.clone();
        bad["tools"].as_array_mut().unwrap().last_mut().unwrap().as_object_mut().unwrap().remove(field);
        assert!(!schema.is_valid(&bad), "{field}");
        assert!(capp::resources::Library::new(&bad).is_err(), "{field}");
    }
    let mut iso = library.clone();
    let t = iso["tools"].as_array_mut().unwrap().last_mut().unwrap().as_object_mut().unwrap();
    let oal = t.remove("overall_length").unwrap(); t.insert("OAL".into(), oal);
    let shank = t.remove("shank_diameter").unwrap(); t.insert("DCONMS".into(), shank);
    assert!(schema.is_valid(&iso));
    assert_eq!(capp::resources::normalize(&iso).unwrap().as_ref(), &library);
    let mut bad = library;
    bad["tools"].as_array_mut().unwrap().last_mut().unwrap()["feed_mm_min"] = json!(100);
    assert!(!schema.is_valid(&bad));
    assert!(capp::resources::Library::new(&bad).is_err());
}

#[test]
fn hand_reach_flutes_and_supplied_choices_use_the_tap_conditions() {
    let p = plan("plate-m5.analysis.json");
    let mut t = example_tap(5., 0.8, 2.);
    t["hand"] = json!("left");
    let mut library = tools(t);
    let rejected = finished(&p, &library);
    assert_eq!(thread(&rejected)["tool"], "tm3");
    assert!(decisions(&rejected).contains("thread hand does not match"));
    let mut left = p.clone();
    for s in left["setups"].as_array_mut().unwrap() {
        for op in s["operations"].as_array_mut().unwrap().iter_mut().filter(|o| o["kind"] == "thread") { op["hand"] = json!("left"); }
    }
    let (ops, _, _) = posted(&left, &library);
    assert_eq!(ops.iter().find(|o| o["kind"] == "tapping").unwrap()["hand"], "left");
    let tap = library["tools"].as_array_mut().unwrap().last_mut().unwrap();
    tap["hand"] = json!("right"); tap["thread_length"] = json!(12);
    let rejected = finished(&p, &library);
    assert_eq!(thread(&rejected)["tool"], "tm3");
    assert!(decisions(&rejected).contains("must reach 12.600"));
    library["tools"].as_array_mut().unwrap().last_mut().unwrap()["thread_length"] = json!(20);
    // The pre-existing supplied milling rule still works with a tap in the library.
    library["decisions"] = json!([{"rule":"tool.thread_mill", "subject":"top:thread", "choice":"tm3", "source":"user", "reason":"use milling"}]);
    let supplied = finished(&p, &library);
    assert_eq!(thread(&supplied)["tool"], "tm3");
    assert_eq!(supplied["supplied_decisions"][0]["accepted"], true);
    library["decisions"][0]["rule"] = json!("tool.thread.tap_or_mill");
    library["decisions"][0]["choice"] = json!("tap");
    assert_eq!(thread(&finished(&p, &library))["tool"], "tap");
}

#[test]
fn a_matching_form_predrill_can_be_tapped_and_an_obstructed_exit_is_refused() {
    let a = tapped(json!({"hole.function:holes:0":"tapped:M3@5", "part.material":"Aluminium"}),
        synthetic(&[down(0., 0., 2.78, 10., "drill_point")], ([-20., -20., -15.], [20., 20., 0.]), &[], &[]));
    let mut a = a;
    a["features"][0]["faces"] = json!([1]);
    let p = capp::plan::plan(&a, None, &top_stock(0.)).unwrap();
    let mut t = example_tap(3., 0.5, 1.);
    t["tap_type"] = json!("form");
    let mut library = tools(t);
    let mut drill = library["tools"].as_array().unwrap().iter().find(|t| t["type"] == "drill").unwrap().clone();
    drill["id"] = json!("drill2.78"); drill["diameter"] = json!(2.78);
    library["tools"].as_array_mut().unwrap().push(drill);
    let (ops, _, _) = posted(&p, &library);
    assert_eq!(ops.last().unwrap()["kind"], "tapping");
    let (jobs, _) = step2cam::job::jobs(&finished(&p, &library), &library).unwrap();
    let run = step2cam::simulate::simulate(&jobs[0].manifest.to_string()).unwrap();
    assert!(run.moves.iter().any(|m| matches!(m.kind, camlib::motion::MoveKind::Tap)));

    let library = tools(example_tap(5., 0.8, 2.));
    let mut p = finished(&plan("plate-m5.analysis.json"), &library);
    for setup in p["setups"].as_array_mut().unwrap() {
        for hole in setup["operations"].as_array_mut().unwrap().iter_mut().filter(|o| o["kind"] == "holes" && o["diameter"] == 4.2) { hole["exit_clear_mm"] = json!(1.5); }
    }
    let (jobs, _) = step2cam::job::jobs(&p, &library).unwrap();
    assert!(jobs.iter().all(|j| list(&j.manifest["operations"]).iter().all(|o| o["kind"] != "tapping")));
    assert!(jobs.iter().flat_map(|j| &j.notes).any(|s| s.contains("actual drilled floor") && s.contains("not tapped")));
}
