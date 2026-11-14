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
/// Why the thread no tool cuts is left out, as its first feature's skip says.
fn untooled_thread(plan: &Value) -> &str {
    assert!(plan["setups"].as_array().unwrap().iter().flat_map(|s| list(&s["operations"])).all(|o| o["kind"] != "thread"), "an untooled thread is not an operation");
    list(&plan["skipped"]).iter().filter_map(|s| s["reason"].as_str()).find(|r| r.starts_with("thread left out: ")).unwrap()
}

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
    // The drawing asks for 8.5 mm of full thread (hole.full_thread).
    let p = plan("synthetic/stepped-plate.analysis.json");
    let settled = finished(&p, &tools);
    let op = thread(&settled);
    assert!(op.get("default_depth").is_none(), "{op}");
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

    // A plug lead of 4 pitches needs 8.5 + 2 + 0.5 = 11 mm; only 10 exists. Without the
    // full-thread answer the 8.5 mm is the default full thread, so the tap cuts what fits:
    // 10 - 2 - 0.5 = 7.5 mm.
    tools["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["type"] == "tap").unwrap()["chamfer_pitches"] = json!(4);
    let mut analysis = fixture("synthetic/stepped-plate.analysis.json");
    analysis["resume"]["answers"].as_object_mut().unwrap().remove("hole.full_thread:hole_patterns.set:48.49.57.58");
    let default = capp::plan::plan(&analysis, None, &Default::default()).unwrap();
    let shortened = finished(&default, &tools);
    assert_eq!(thread(&shortened)["tool"], "tap", "{}", decisions(&shortened));
    assert_close(number(&thread(&shortened)["top_z"]) - number(&thread(&shortened)["bottom_z"]), 7.5);
    assert!(decisions(&shortened).contains("the default 8.5 mm full thread is cut 7.5 mm deep"), "{}", decisions(&shortened));
    // The same 8.5 mm asked for is refused.
    let mut asked = default.clone();
    for setup in asked["setups"].as_array_mut().unwrap() {
        for op in setup["operations"].as_array_mut().unwrap().iter_mut().filter(|o| o["kind"] == "thread") {
            assert_eq!(op["default_depth"], true);
            op.as_object_mut().unwrap().remove("default_depth");
        }
    }
    let rejected = finished(&asked, &tools);
    let text = decisions(&rejected);
    assert!(text.contains("requires 11.00 mm; drilled full diameter is 10.00 mm"), "{text}");
    // No thread operation is tapped (here no tool cuts it, so it is left out of the plan).
    assert!(rejected["setups"].as_array().unwrap().iter().flat_map(|s| list(&s["operations"])).all(|o| o["kind"] != "thread" || o["tool"] != "tap"));
    // As is the drawing's own full-thread answer.
    let text = decisions(&finished(&p, &tools));
    assert!(text.contains("requires 11.00 mm; drilled full diameter is 10.00 mm"), "{text}");
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
    let text = untooled_thread(&p);
    assert!(text.contains("an M5×0.8 cut tap (⌀4.2 tap drill) with thread length at least 11.01 mm plus its chamfer, or a thread mill"), "{text}");
    assert!(text.contains("cutter diameter − 0.82"), "{text}");
    assert!(!text.contains("+ -"));
    assert!(!text.contains("subject to"));
    assert!(!text.contains("0.0 mm"));

    let stepped = plan("synthetic/stepped-plate.analysis.json");
    let p = finished(&stepped, &tools);
    let text = untooled_thread(&p);
    // 8.5 full + 1 maximum lead + camlib guard, rounded upwards to 0.01 mm.
    assert!(text.contains("an M3×0.5 cut tap (⌀2.5 tap drill) with a chamfer of at most 1 mm (2 pitches) and thread length at least 9.51 mm, or a thread mill"), "{text}");
    assert!(text.contains("cutter diameter − 0.52"), "{text}");
    assert!(!text.contains("0.0 mm"));
    // The advertised dimensions actually pass the tap rule, including its reach guard.
    let mut matching = tools.clone();
    let mut tap = example_tap(3., 0.5, 2.);
    tap["thread_length"] = json!(9.51);
    matching["tools"].as_array_mut().unwrap().push(tap);
    assert_eq!(thread(&finished(&stepped, &matching))["tool"], "tap");

    for material in ["hardened steel", "unknown stock"] {
        tools["overrides"]["material"] = json!(material);
        let p = finished(&stepped, &tools);
        let text = untooled_thread(&p);
        assert!(!text.contains("cut tap"), "{material}: {text}");
        assert!(text.contains("needs a thread mill"), "{text}");
    }
    tools["overrides"]["material"] = json!("Aluminium");
    let mut wrong_drill = stepped;
    for setup in wrong_drill["setups"].as_array_mut().unwrap() {
        for op in setup["operations"].as_array_mut().unwrap().iter_mut().filter(|o| o["kind"] == "thread") { op["start_diameter"] = json!(2.8); }
    }
    let p = finished(&wrong_drill, &tools);
    assert!(!untooled_thread(&p).contains("cut tap"));

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

#[test]
fn through_tap_travel_preserves_other_operations_and_the_real_comparison_base() {
    let analysis = fixture("plate-m5.analysis.json");
    let mut mesh = fixture("plate.mesh.json");
    mesh["binding"] = analysis["binding"].clone();
    let p = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let no_tap = fixture("placeholder-tools.json");
    let mut with_tap = no_tap.clone();
    with_tap["tools"].as_array_mut().unwrap().push(example_tap(5., 0.8, 4.));
    let (before, _) = step2cam::job::jobs(&finished(&p, &no_tap), &no_tap).unwrap();
    let settled = finished(&p, &with_tap);
    assert_eq!(thread(&settled)["tool"], "tap");
    let (after, _) = step2cam::job::jobs(&settled, &with_tap).unwrap();
    assert_eq!(before.len(), after.len());
    let mut compared = 0;
    for (before, after) in before.iter().zip(&after) {
        let a = list(&before.manifest["operations"]);
        let b = list(&after.manifest["operations"]);
        let tapping = b.iter().find(|o| o["kind"] == "tapping").unwrap();
        let own_drill = b.iter().find(|o| o["kind"] == "drilling" && o["centers_mm"] == tapping["centers_mm"] && o["tool"] == "drill4.2").unwrap();
        for op in a.iter().filter(|o| o["kind"] != "thread_mill" && o["id"] != own_drill["id"]) {
            assert_eq!(Some(op), b.iter().find(|candidate| candidate["id"] == op["id"]), "unrelated {} changed", op["id"]);
            compared += 1;
        }
        assert_eq!(a.len(), b.len());
        let base = &before.manifest["verification"]["material"][0]["target"]["base_z_mm"];
        assert_eq!(base, -12.);
        assert_eq!(base, &after.manifest["verification"]["material"][0]["target"]["base_z_mm"]);
        for id in ["top-5-holes", "top-6-holes"] {
            assert_eq!(b.iter().find(|o| o["id"] == id).unwrap()["floor_z_mm"], -11.);
        }
        assert_close(number(&tapping["depth_z_mm"]), -14.2);
        assert_close(number(&own_drill["depth_z_mm"]), -15. - capp::resources::DRILLING_GUARD);
        let drill = with_tap["tools"].as_array().unwrap().iter().find(|t| t["id"] == own_drill["tool"]).unwrap();
        let tip = number(&own_drill["depth_z_mm"]) - capp::resources::drill_tip_length(drill).unwrap();
        // Breakthrough allowance is still measured from the real -12 mm stock bottom.
        assert_close(number(&own_drill["allow_breakthrough_mm"]), -12. - tip + 0.01);
        assert!(any_note(&after.notes, "tap tip goes 3.2 mm and its drill tip 5.27 mm below the part's bottom (drill full diameter 4.01 mm below): keep the hole clear of parallels"), "{:?}", after.notes);
        let run = step2cam::simulate::simulate_job(after).unwrap();
        assert_eq!(run.moves.iter().filter(|m| matches!(m.kind, camlib::motion::MoveKind::Tap)).count(), 2);
    }
    assert!(compared >= 6, "must compare the unrelated bores and the rest of the job");
}

#[test]
fn review_left_hand_fallback_mills_the_requested_helix() {
    let library = tools(example_tap(5., 0.8, 2.)); // right-hand tap only
    let mut p = plan("plate-m5.analysis.json");
    for setup in p["setups"].as_array_mut().unwrap() {
        for op in setup["operations"].as_array_mut().unwrap().iter_mut().filter(|o| o["kind"] == "thread") {
            op["hand"] = json!("left");
        }
    }
    let settled = finished(&p, &library);
    assert_eq!(thread(&settled)["tool"], "tm3");
    assert!(decisions(&settled).contains("thread hand does not match"));
    let (jobs, _) = step2cam::job::jobs(&settled, &library).unwrap();
    let job = &jobs[0];
    let op = list(&job.manifest["operations"]).iter().find(|o| o["kind"] == "thread_mill").unwrap();
    assert_eq!(op["right_hand"], false);
    assert_eq!(op["climb"], true);
    assert!(any_note(&job.notes, "pitch 0.8, left hand"));
    let run = step2cam::simulate::simulate_job(job).unwrap();
    let helices: Vec<_> = run.moves.iter().filter(|m| m.op == op["id"] && m.curve == "helix").collect();
    assert!(!helices.is_empty());
    let center = &op["centers_mm"][0];
    for helix in helices {
        let dz = helix.points.last().unwrap()[2] - helix.points[0][2];
        assert!(dz < 0., "left-hand internal climb milling must descend");
        let angle: f64 = helix.points.windows(2).map(|pair| {
            let xy = |p: [f64; 3]| [p[0] - number(&center[0]), p[1] - number(&center[1])];
            let (a, b) = (xy(pair[0]), xy(pair[1]));
            (a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1])
        }).sum();
        assert!(angle > 0., "internal climb milling must remain counter-clockwise");
        assert!((dz / angle + 0.8 / std::f64::consts::TAU).abs() < 1e-4, "wrong helix pitch/hand");
    }
}

#[test]
fn review_limited_exit_selects_milling_before_the_job_is_built() {
    let library = tools(example_tap(5., 0.8, 2.));
    for clearance in [2., 3., 4.] {
        // One M5 breaks into a cavity; an otherwise identical second hole has a clear exit.
        let mut a = tapped(json!({"hole.function:holes:0":"tapped:M5", "hole.function:holes:1":"tapped:M5", "part.material":"Aluminium"}),
            holes_only(&[down(0., 0., 4.2, 6., "through"), down(10., 0., 4.2, 6., "through")]));
        a["features"][0]["faces"] = json!([1]);
        a["features"][1]["faces"] = json!([2]);
        let mesh = mesh_of(&square(-6. - clearance, 3., 0., 0.), BOX);
        let p = capp::plan::plan(&a, Some(&mesh), &top_stock(0.)).unwrap();
        let threads: Vec<_> = list(&p["setups"][0]["operations"]).iter().filter(|o| o["kind"] == "thread").collect();
        assert_eq!(threads.len(), 2, "different exit clearances must not merge");
        let limited = threads.iter().find(|o| o["features"] == json!(["holes:0"])).unwrap();
        assert_eq!(limited["exit_clear_mm"], clearance, "copy the measured hole exit clearance");
        let settled = finished(&p, &library);
        let ops = list(&settled["setups"][0]["operations"]);
        let limited = ops.iter().find(|o| o["kind"] == "thread" && o["features"] == json!(["holes:0"])).unwrap();
        let clear = ops.iter().find(|o| o["kind"] == "thread" && o["features"] == json!(["holes:1"])).unwrap();
        assert_eq!(limited["tool"], if clearance < 4. { "tm3" } else { "tap" });
        assert_eq!(clear["tool"], "tap");
        if clearance < 4. {
            let trace = decisions(&settled);
            assert!(trace.contains("through tapping needs 2.400161 mm"), "{trace}");
            assert!(trace.contains("drill point"), "{trace}");
        }
        let (jobs, _) = step2cam::job::jobs(&settled, &library).unwrap();
        let job = &jobs[0];
        let output = list(&job.manifest["operations"]);
        let milled = output.iter().find(|o| o["kind"] == "thread_mill");
        if clearance < 4. {
            assert_eq!(milled.unwrap()["centers_mm"], limited["centers"]);
        } else { assert!(milled.is_none()); }
        assert_eq!(output.iter().filter(|o| o["kind"] == "tapping").count(), if clearance < 4. { 1 } else { 2 });
        assert!(!any_note(&job.notes, "not tapped"));
        if clearance < 4. {
            let mut no_mill = library.clone();
            no_mill["tools"].as_array_mut().unwrap().retain(|t| t["type"] != "thread_mill");
            let missing = finished(&p, &no_mill);
            let note = list(&missing["skipped"]).iter().find(|s| s["feature"] == "holes:0" && s["reason"].as_str().unwrap().starts_with("thread left out: ")).unwrap()["reason"].as_str().unwrap();
            if clearance == 2. { assert!(!note.contains("cut tap"), "no lead fits: {note}"); }
            else { assert!(note.contains("chamfer of at most 0.73 mm (0.91 pitches) for the exit clearance"), "{note}"); }
        }
        let validator = jsonschema::validator_for(&fixture("../../schemas/plan-1.schema.json")).unwrap();
        assert!(validator.is_valid(&settled), "{:?}", validator.iter_errors(&settled).collect::<Vec<_>>());
        let run = step2cam::simulate::simulate_job(job).unwrap();
        if let Some(milled) = milled {
            assert!(run.operations.iter().any(|o| o.id == milled["id"]));
            assert!(run.moves.iter().any(|m| m.op == milled["id"] && m.curve == "helix"));
        } else {
            assert_eq!(run.moves.iter().filter(|m| matches!(m.kind, camlib::motion::MoveKind::Tap)).count(), 4);
        }
    }
}

#[test]
fn a_default_blind_thread_leaves_room_for_the_library_taps_lead_and_an_asked_one_does_not() {
    // Review M7: the default full thread (depth less three pitches) left no room for a form C
    // tap's three-pitch lead plus one pitch of clearance, so no library tap could cut it.
    let library = read_library("libraries/haas-vf2.json");
    let part = |answer: &str| {
        let mut a = tapped(json!({"hole.function:holes:0": answer, "part.material": "Aluminium"}),
            holes_only(&[down(0., 0., 4.2, 10., "drill_point")]));
        a["features"][0]["faces"] = json!([1]);
        capp::plan::plan(&a, None, &top_stock(0.)).unwrap()
    };
    let planned = part("tapped:M5");
    // specify-core's default: floor((10 - 3 × 0.8) / 0.5) × 0.5 = 7.5 mm.
    assert_close(number(&thread(&planned)["top_z"]) - number(&thread(&planned)["bottom_z"]), 7.5);
    assert_eq!(thread(&planned)["default_depth"], true);
    let settled = finished(&planned, &library);
    let op = thread(&settled);
    assert_eq!(op["tool"], "tapM5", "{}", decisions(&settled));
    // tapM5 has a three-pitch lead: 10 - (3 + 1) × 0.8 = 6.8, rounded down to 6.5 mm.
    assert_close(number(&op["top_z"]) - number(&op["bottom_z"]), 6.5);
    assert!(decisions(&settled).contains("the default 7.5 mm full thread is cut 6.5 mm deep"), "{}", decisions(&settled));
    let (jobs, _) = step2cam::job::jobs(&settled, &library).unwrap();
    let tapping = jobs.iter().flat_map(|j| list(&j.manifest["operations"])).find(|o| o["kind"] == "tapping").unwrap();
    assert_close(number(&tapping["depth_z_mm"]), number(&op["bottom_z"]) - 3. * 0.8);
    assert!(!jobs.iter().flat_map(|j| &j.notes).any(|s| s.contains("not tapped")));

    // The same depth asked for keeps its depth and is refused by the tap.
    let asked = part("tapped:M5@7.5");
    assert!(thread(&asked).get("default_depth").is_none());
    let settled = finished(&asked, &library);
    assert_ne!(thread(&settled)["tool"], "tapM5");
    assert_close(number(&thread(&settled)["top_z"]) - number(&thread(&settled)["bottom_z"]), 7.5);
    assert!(decisions(&settled).contains("requires 10.70 mm; drilled full diameter is 10.00 mm"), "{}", decisions(&settled));
}

fn read_library(path: &str) -> Value {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}
