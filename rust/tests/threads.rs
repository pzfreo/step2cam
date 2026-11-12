//! Threads from Specify's answers, on the plate (its ⌀4.2 hole is holes:12, through, 10 deep).
mod common;
use common::{any_note, finished, assert_same, down, example_tools, fixture, holes_only, list, number, posted, synthetic, tapped, top_stock};
use serde_json::{Value, json};

fn plate(answers: Value) -> Value {
    let mut analysis = fixture("plate.analysis.json");
    analysis["resume"] = json!({"answers": answers});
    analysis
}
fn plan_of(analysis: &Value) -> Value {
    capp::plan::plan(analysis, None, &Default::default()).unwrap()
}
fn threads(plan: &Value) -> Vec<Value> {
    plan["setups"].as_array().unwrap().iter().flat_map(|s| s["operations"].as_array().unwrap().iter().filter(|o| o["kind"] == "thread").cloned()).collect()
}
fn blind(analysis: &mut Value, hole: &str) {
    let feature = analysis["features"].as_array_mut().unwrap().iter_mut().find(|f| f["id"] == hole).unwrap();
    feature["record"]["bottom"] = json!("flat");
}

#[test]
fn a_tapped_answer_threads_its_hole() {
    let ops = threads(&plan_of(&plate(json!({"hole.function:holes:12": "tapped:M5"}))));
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["spec"], "M5");
    assert_eq!(ops[0]["major"], 5.0);
    assert_eq!(ops[0]["pitch"], 0.8);
    assert_eq!(ops[0]["start_diameter"], 4.2);
    assert_eq!(ops[0]["through"], true);
    assert_eq!(ops[0]["features"], json!(["holes:12"]));
    assert_same(json!([ops[0]["top_z"], ops[0]["bottom_z"], ops[0]["centers"]]), json!([-1, -11, [[97, 32]]]));
    let plan = plan_of(&plate(json!({"hole.function:holes:12": "tapped:M5"})));
    let kinds: Vec<&str> = list(&plan["setups"][0]["operations"]).iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert!(kinds.iter().position(|k| *k == "thread").unwrap() > kinds.iter().rposition(|k| *k == "holes").unwrap(), "{kinds:?}");
}

#[test]
fn a_pattern_answer_threads_every_member() {
    let analysis = fixture("plate.analysis.json");
    let pattern = analysis["features"].as_array().unwrap().iter().find(|f| f["family"] == "hole_patterns").unwrap()["id"].as_str().unwrap().to_string();
    let plan = plan_of(&plate(json!({format!("hole.function:{pattern}"): "tapped:M8"})));
    let ops = threads(&plan);
    assert_eq!(ops.len(), 1);
    let mut features: Vec<&str> = list(&ops[0]["features"]).iter().map(|f| f.as_str().unwrap()).collect();
    features.sort_unstable();
    assert_eq!(features, ["holes:10", "holes:11", "holes:13", "holes:6", "holes:7", "holes:8"]);
    assert_same(json!([ops[0]["major"], ops[0]["pitch"]]), json!([8, 1.25]));
    assert_eq!(pattern, "hole_patterns.grid:6.7.8.10.11.13");
}

#[test]
fn a_fine_pitch_is_read() {
    let ops = threads(&plan_of(&plate(json!({"hole.function:holes:12": "tapped:M5x0.5"}))));
    assert_eq!(ops[0]["spec"], "M5x0.5");
    assert_eq!(ops[0]["pitch"], 0.5);
    let ops = threads(&plan_of(&plate(json!({"hole.function:holes:9": "tapped:M10x1.25"}))));
    assert_eq!(ops.len(), 1);
    assert_same(json!([ops[0]["major"], ops[0]["pitch"]]), json!([10, 1.25]));
}

#[test]
fn a_blind_thread_is_as_deep_as_asked_or_specify_cores_default() {
    let top_and_bottom = |answer: &str| {
        let mut analysis = plate(json!({"hole.function:holes:12": answer}));
        blind(&mut analysis, "holes:12");
        let op = threads(&plan_of(&analysis))[0].clone();
        op["top_z"].as_f64().unwrap() - op["bottom_z"].as_f64().unwrap()
    };
    // 10 deep, less three 0.8 pitches, rounded down to 0.5.
    assert_eq!(top_and_bottom("tapped:M5"), 7.5);
    assert_eq!(top_and_bottom("tapped:M5@5.5"), 5.5);

    let mut a = holes_only(&[down(0., 0., 4.2, 10., "flat"), down(9., 0., 4.2, 10., "flat")]);
    a["features"][0]["faces"] = json!([1]);
    a["features"][1]["faces"] = json!([2]);
    let a = tapped(json!({"hole.function:holes:0": "tapped:M5", "hole.function:holes:1": "tapped:M5@5.5"}), a);
    let plan = capp::plan::plan(&a, None, &top_stock(0.)).unwrap();
    let mut ends: Vec<[f64; 2]> = threads(&plan).iter().map(|o| [number(&o["top_z"]), number(&o["bottom_z"])]).collect();
    ends.sort_by(|a, b| a[1].total_cmp(&b[1]));
    // Default: depth less three pitches, down to 0.5 mm: 10 - 2.4 = 7.6 -> 7.5.
    assert_eq!(ends, [[0., -7.5], [0., -5.5]]);
}

#[test]
fn threads_in_one_setup_come_out_in_thread_key_order() {
    // The key sorts by spec text first (so "M10" before "M5"), then major, pitch, start diameter, top and bottom z, through, exit clearance.
    let specs = ["M5", "M5@10", "M5", "M5@5.5", "M5@5.5", "M5x0.5", "M6", "M5", "M10"];
    let holes = [
        down(-15., -15., 4.2, 10., "through"), down(-5., -15., 4.2, 10., "flat"), down(5., -15., 4.2, 10., "flat"),
        down(15., -15., 4.2, 10., "flat"), down(-15., 0., 4.3, 10., "flat"), down(-5., 0., 4.5, 10., "flat"),
        down(5., 0., 5., 10., "through"), down(15., 0., 4.2, 10., "flat"), down(-15., 15., 8.5, 10., "through"),
    ];
    let mut a = holes_only(&holes);
    let mut answers = json!({});
    for (i, spec) in specs.iter().enumerate() {
        a["features"][i]["faces"] = json!([i + 1]);
        answers[format!("hole.function:holes:{i}")] = json!(format!("tapped:{spec}"));
    }
    let plan = capp::plan::plan(&tapped(answers, a), None, &top_stock(0.)).unwrap();
    assert_eq!(plan["setups"].as_array().unwrap().iter().filter(|s| list(&s["operations"]).iter().any(|o| o["kind"] == "thread")).count(), 1);
    let order: Vec<Value> = threads(&plan).iter().map(|o| json!([o["spec"], o["start_diameter"], o["bottom_z"], o["through"], o["features"]])).collect();
    assert_same(json!(order), json!([
        ["M10", 8.5, -10, true, ["holes:8"]],
        ["M5", 4.2, -10, false, ["holes:1"]],
        ["M5", 4.2, -10, true, ["holes:0"]],
        ["M5", 4.2, -7.5, false, ["holes:2", "holes:7"]],
        ["M5", 4.2, -5.5, false, ["holes:3"]],
        ["M5", 4.3, -5.5, false, ["holes:4"]],
        ["M5x0.5", 4.5, -8.5, false, ["holes:5"]],
        ["M6", 5, -10, true, ["holes:6"]],
    ]));
}

#[test]
fn a_thread_deeper_than_its_hole_is_a_warning() {
    let mut a = holes_only(&[down(0., 0., 4.2, 6., "flat")]);
    a["features"][0]["faces"] = json!([1]);
    let a = tapped(json!({"hole.function:holes:0": "tapped:M5@8"}), a);
    let plan = capp::plan::plan(&a, None, &top_stock(0.)).unwrap();
    assert!(threads(&plan).is_empty());
    assert!(plan["warnings"].to_string().contains("deeper than the hole"), "{}", plan["warnings"]);
}

#[test]
fn answers_that_cannot_be_threaded_are_warnings() {
    for (answers, expected) in [
        (json!({"hole.function:holes:12": "tapped:UNC1/4"}), "not a metric size"),
        (json!({"hole.function:holes:12": "tapped:M5@deep"}), "unreadable depth"),
        (json!({"hole.function:holes:99": "tapped:M5"}), "no such feature"),
    ] {
        let plan = plan_of(&plate(answers));
        assert!(threads(&plan).is_empty());
        assert!(plan["warnings"].to_string().contains(expected), "{}", plan["warnings"]);
    }
    let mut analysis = plate(json!({"hole.function:holes:12": "tapped:M5@50"}));
    blind(&mut analysis, "holes:12");
    let plan = plan_of(&analysis);
    assert!(threads(&plan).is_empty());
    assert!(plan["warnings"].to_string().contains("deeper than the hole"), "{}", plan["warnings"]);

    let plan = plan_of(&plate(json!({"hole.function:holes:12": "tapped:M7", "hole.function:holes:99": "tapped:M5", "hole.function:holes:9": "clearance:M8"})));
    assert!(threads(&plan).is_empty());
    let text = plan["warnings"].to_string();
    assert!(text.contains("M7 is not a metric size"), "{text}");
    assert!(text.contains("holes:99: tapped M5, but the part has no such feature"), "{text}");
}

#[test]
fn a_thread_another_tool_wrote_is_a_warning() {
    let mut analysis = fixture("plate.analysis.json");
    let faces = analysis["features"].as_array().unwrap().iter().find(|f| f["id"] == "holes:12").unwrap()["faces"].clone();
    analysis["existing"] = json!([{"type": "internal thread", "faces": faces}]);
    let plan = plan_of(&analysis);
    assert!(threads(&plan).is_empty());
    assert!(plan["warnings"].to_string().contains("states no size"), "{}", plan["warnings"]);

    let mut analysis = fixture("plate.analysis.json");
    analysis["existing"] = json!([{"kind": "note", "type": "internal thread", "faces": [12]}]);
    let plan = plan_of(&analysis);
    assert!(threads(&plan).is_empty());
    assert!(list(&plan["warnings"]).iter().any(|w| w.as_str().unwrap().starts_with("holes:12: the file marks an internal thread")), "{}", plan["warnings"]);
}

fn jobs_note(plan: &Value, library: &Value) -> (Vec<step2cam::job::Job>, String) {
    let (jobs, log) = step2cam::job::jobs(&finished(&plan, library), library).unwrap();
    let notes = jobs.iter().flat_map(|j| j.notes.clone()).chain(log).collect::<Vec<_>>().join("\n");
    (jobs, notes)
}

#[test]
fn a_thread_is_milled_after_its_hole() {
    let plan = plan_of(&plate(json!({"hole.function:holes:12": "tapped:M5"})));
    let (ops, _, notes) = posted(&plan, &example_tools());
    let thread = ops.last().unwrap();
    assert_eq!(thread["kind"], "thread_mill");
    assert_eq!(thread["tool"], "tm3");
    assert_same(
        json!([thread["internal"], thread["diameter_mm"], thread["start_diameter_mm"], thread["pitch_mm"], thread["top_z_mm"], thread["bottom_z_mm"], thread["radial_passes"], thread["spring_passes"]]),
        json!([true, 5, 4.2, 0.8, -1, -11, 2, 1]),
    );
    assert!(any_note(&notes, "thread milled"), "{notes:?}");
}

#[test]
fn a_thread_asks_for_a_stock_grid_fine_enough_for_the_neck() {
    let plan = plan_of(&plate(json!({"hole.function:holes:12": "tapped:M5"})));
    let (jobs, notes) = jobs_note(&plan, &fixture("../../examples/tools.example.json"));
    assert_eq!(jobs[0].manifest["stock"]["cell_size_mm"], 0.056);
    assert!(notes.contains("thread milled to the ⌀5.0 major diameter"), "{notes}");
    // tm3 in the M5's 4.2 mm hole: its path circle is 2.5 - 1.5 = 1 mm off the axis, so its 2 mm
    // neck passes 2.1 - 1 - 1 = 0.1 mm from the wall. camlib proves that clearance on its stock
    // grid, which must be finer than the clearance over a cell's diagonal; the job rounds that
    // down to a whole micrometre.
    // Without a thread nothing is asked: the grid is the part's own, 1/400 of its longest side.
    let (jobs, notes) = jobs_note(&plan_of(&fixture("plate.analysis.json")), &example_tools());
    assert_eq!(jobs[0].manifest["stock"]["cell_size_mm"], 0.26);
    assert!(!notes.contains("stock grid"), "{notes}");
}

#[test]
fn a_thread_no_mill_fits_says_why() {
    let plan = plan_of(&plate(json!({"hole.function:holes:12": "tapped:M5x0.25"})));
    let (jobs, notes) = jobs_note(&plan, &fixture("../../examples/tools.example.json"));
    assert!(notes.contains("no thread mill can cut it (tm3: pitch 0.25 outside its range)"), "{notes}");
    assert!(jobs[0].manifest["operations"].as_array().unwrap().iter().all(|o| o["kind"] != "thread_mill"));
}

/// A ⌀2.5 hole tapped M3, on the library as it is and with a drill for it added.
fn a_thread_in_a_small_hole(library: &Value) -> (Vec<Value>, Vec<String>) {
    let mut a = holes_only(&[down(0., 0., 2.5, 10., "through")]);
    a["features"][0]["faces"] = json!([1]);
    let plan = capp::plan::plan(&tapped(json!({"hole.function:holes:0": "tapped:M3"}), a), None, &top_stock(0.)).unwrap();
    let (ops, _, notes) = posted(&plan, library);
    (ops, notes)
}
fn with_a_small_drill() -> Value {
    let mut data = example_tools();
    let mut drill = data["tools"][0].clone();
    for (key, value) in [("id", json!("drill2.5")), ("type", json!("drill")), ("number", json!(9)), ("diameter", json!(2.5)), ("flute_length", json!(20)), ("point_angle", json!(118))] {
        drill[key] = value;
    }
    data["tools"].as_array_mut().unwrap().push(drill);
    data
}

/// Python's test_a_thread_no_mill_fits_says_why, as far as it passes: the rest is the ignored test
/// below.
#[test]
fn a_thread_in_a_hole_that_is_not_cut_is_not_threaded() {
    let (ops, notes) = a_thread_in_a_small_hole(&example_tools());
    assert!(ops.is_empty(), "{ops:?}");
    assert!(any_note(&notes, "is not cut, so it is not threaded"), "{notes:?}");
    let (ops, notes) = a_thread_in_a_small_hole(&with_a_small_drill());
    let kinds: Vec<&str> = ops.iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["drilling", "drilling"]); // spot, then the tap drill
    assert_eq!(ops[0]["tool"], "spot8");
    assert_eq!(ops[1]["tool"], "drill2.5");
    assert!(any_note(&notes, "no thread mill can cut it (tm3: "), "{notes:?}");
}

#[test]
fn a_thread_no_mill_fits_says_why_with_the_tools_own_diameter() {
    let (_, notes) = a_thread_in_a_small_hole(&with_a_small_drill());
    assert!(any_note(&notes, "no thread mill can cut it (tm3: ⌀3 does not fit the ⌀2.5 hole)"), "{notes:?}");
}

#[test]
fn a_thread_whose_neck_needs_too_fine_a_grid_is_not_cut() {
    let plan = plan_of(&plate(json!({"hole.function:holes:12": "tapped:M5"})));
    let mut library = fixture("../../examples/tools.example.json");
    for tool in library["tools"].as_array_mut().unwrap() {
        if tool["id"] == "tm3" {
            tool["neck_diameter"] = json!(2.17); // 0.015 mm clear of the wall
        }
    }
    let (_, notes) = jobs_note(&plan, &library);
    assert!(notes.contains("too fine for this part"), "{notes}");

    // The same hole in a 2 m part: the grid would need over a billion cells.
    let big = ([-1000., -1000., -10.], [1000., 1000., 0.]);
    let mut a = synthetic(&[down(0., 0., 4.2, 10., "through")], big, &[], &[]);
    a["features"][0]["faces"] = json!([1]);
    let plan = capp::plan::plan(&tapped(json!({"hole.function:holes:0": "tapped:M5"}), a), None, &top_stock(0.)).unwrap();
    // Without a machine described: a real mill's travels would refuse a 2 m part first.
    let mut no_machine = example_tools();
    no_machine.as_object_mut().unwrap().remove("machines");
    let (ops, _, notes) = posted(&plan, &no_machine);
    assert!(ops.iter().all(|o| o["kind"] != "thread_mill"), "{ops:?}");
    assert!(any_note(&notes, "too fine for this part"), "{notes:?}");
}

#[test]
fn a_thread_below_the_drills_full_diameter_is_not_cut() {
    let mut analysis = plate(json!({"hole.function:holes:12": "tapped:M5@10"}));
    blind(&mut analysis, "holes:12");
    let plan = plan_of(&analysis);
    // A flat-bottomed hole drilled to its depth is at full diameter only down to the drill's point.
    let (_, notes) = jobs_note(&plan, &fixture("../../examples/tools.example.json"));
    assert!(notes.contains("below where the hole reaches full diameter"), "{notes}");

    // A flat-bottomed hole: the drill's tip stops at the bottom, so full diameter ends a tip
    // length short; a thread asked all the way down cannot be milled there.
    let mut a = holes_only(&[down(0., 0., 4.2, 8., "flat")]);
    a["features"][0]["faces"] = json!([1]);
    let plan = capp::plan::plan(&tapped(json!({"hole.function:holes:0": "tapped:M5@8"}), a), None, &top_stock(0.)).unwrap();
    let (ops, _, notes) = posted(&plan, &example_tools());
    assert!(ops.iter().all(|o| o["kind"] != "thread_mill"), "{ops:?}");
    assert!(any_note(&notes, "below where the hole reaches full diameter"), "{notes:?}");
}

#[test]
fn a_thread_mill_needs_its_tooth_and_neck() {
    let plan = plan_of(&fixture("plate.analysis.json"));
    let mut library = fixture("../../examples/tools.example.json");
    let tm = library["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == "tm3").unwrap();
    tm.as_object_mut().unwrap().remove("neck_diameter");
    let Err(error) = capp::finish(&mut plan.clone(), &library, &[]) else { panic!("accepted") };
    assert!(error.contains("missing neck_diameter"), "{error}");
    // A neck as wide as the tooth is no neck.
    let mut library = fixture("../../examples/tools.example.json");
    let tm = library["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == "tm3").unwrap();
    tm["neck_diameter"] = tm["diameter"].clone();
    assert!(capp::finish(&mut plan.clone(), &library, &[]).is_err());
}
