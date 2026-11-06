//! Threads from Specify's answers, on the plate (its ⌀4.2 hole is holes:12, through, 10 deep).
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures").join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
fn plate(answers: Value) -> Value {
    let mut analysis = fixture("plate.analysis.json");
    analysis["resume"] = json!({"answers": answers});
    analysis
}
fn plan_of(analysis: &Value) -> Value {
    step2cam::plan::plan(analysis, None, &Default::default()).unwrap()
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
}

#[test]
fn a_pattern_answer_threads_every_member() {
    let analysis = fixture("plate.analysis.json");
    let pattern = analysis["features"].as_array().unwrap().iter().find(|f| f["family"] == "hole_patterns").unwrap()["id"].as_str().unwrap().to_string();
    let plan = plan_of(&plate(json!({format!("hole.function:{pattern}"): "tapped:M8"})));
    let ops = threads(&plan);
    assert_eq!(ops.len(), 1);
    assert!(ops[0]["features"].as_array().unwrap().len() > 1, "{ops:?}");
}

#[test]
fn a_fine_pitch_is_read() {
    let ops = threads(&plan_of(&plate(json!({"hole.function:holes:12": "tapped:M5x0.5"}))));
    assert_eq!(ops[0]["spec"], "M5x0.5");
    assert_eq!(ops[0]["pitch"], 0.5);
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
}

#[test]
fn a_thread_another_tool_wrote_is_a_warning() {
    let mut analysis = fixture("plate.analysis.json");
    let faces = analysis["features"].as_array().unwrap().iter().find(|f| f["id"] == "holes:12").unwrap()["faces"].clone();
    analysis["existing"] = json!([{"type": "internal thread", "faces": faces}]);
    let plan = plan_of(&analysis);
    assert!(threads(&plan).is_empty());
    assert!(plan["warnings"].to_string().contains("states no size"), "{}", plan["warnings"]);
}

fn jobs_note(plan: &Value, library: &Value) -> (Vec<step2cam::job::Job>, String) {
    let (jobs, log) = step2cam::job::jobs(plan, library).unwrap();
    let notes = jobs.iter().flat_map(|j| j.notes.clone()).chain(log).collect::<Vec<_>>().join("\n");
    (jobs, notes)
}

#[test]
fn a_thread_asks_for_a_stock_grid_fine_enough_for_the_neck() {
    let plan = plan_of(&plate(json!({"hole.function:holes:12": "tapped:M5"})));
    let (jobs, notes) = jobs_note(&plan, &fixture("../../examples/tools.example.json"));
    assert_eq!(jobs[0].manifest["stock"]["cell_size_mm"], 0.056);
    assert!(notes.contains("thread milled to the ⌀5.0 major diameter"), "{notes}");
}

#[test]
fn a_thread_no_mill_fits_says_why() {
    let plan = plan_of(&plate(json!({"hole.function:holes:12": "tapped:M5x0.25"})));
    let (jobs, notes) = jobs_note(&plan, &fixture("../../examples/tools.example.json"));
    assert!(notes.contains("no thread mill can cut it (tm3: pitch 0.25 outside its range)"), "{notes}");
    assert!(jobs[0].manifest["operations"].as_array().unwrap().iter().all(|o| o["kind"] != "thread_mill"));
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
}

#[test]
fn a_thread_below_the_drills_full_diameter_is_not_cut() {
    let mut analysis = plate(json!({"hole.function:holes:12": "tapped:M5@10"}));
    blind(&mut analysis, "holes:12");
    let plan = plan_of(&analysis);
    // A flat-bottomed hole drilled to its depth is at full diameter only down to the drill's point.
    let (_, notes) = jobs_note(&plan, &fixture("../../examples/tools.example.json"));
    assert!(notes.contains("below where the hole reaches full diameter"), "{notes}");
}

#[test]
fn a_thread_mill_needs_its_tooth_and_neck() {
    let plan = plan_of(&fixture("plate.analysis.json"));
    let mut library = fixture("../../examples/tools.example.json");
    let tm = library["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == "tm3").unwrap();
    tm.as_object_mut().unwrap().remove("neck_diameter");
    let Err(error) = step2cam::job::jobs(&plan, &library) else { panic!("accepted") };
    assert!(error.contains("missing neck_diameter"), "{error}");
}
