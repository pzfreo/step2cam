//! Turning: the spindle part (turned steps ⌀24, ⌀15, ⌀10 on Z, a ⌀4.2 hole on the axis and a ⌀4
//! cross hole) on the demo's Sherline 4400 lathe and Makera Carvera mill.
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn read(path: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path)).unwrap()).unwrap()
}
fn finished(library: &Value) -> Value {
    let mut plan = capp::plan::plan(&read("tests/fixtures/spindle.analysis.json"), None, &Default::default()).unwrap();
    capp::finish(&mut plan, library, &[]).unwrap();
    plan
}
fn kinds(setup: &Value) -> Vec<&str> {
    setup["operations"].as_array().unwrap().iter().map(|o| o["kind"].as_str().unwrap()).collect()
}
fn rules(plan: &Value) -> Vec<&str> {
    plan["decisions"].as_array().unwrap().iter().map(|d| d["rule"].as_str().unwrap()).collect()
}

#[test]
fn the_spindle_is_turned_first_then_cross_drilled() {
    let plan = finished(&read("demo/tools.json"));
    let setups = plan["setups"].as_array().unwrap();
    let lathe = &setups[0];
    assert_eq!(lathe["kind"], "lathe");
    assert_eq!(kinds(lathe), ["lathe_face", "lathe_rough", "lathe_finish", "lathe_drill", "lathe_part_off"]);
    assert_eq!(lathe["bar"]["diameter"], 25.0, "⌀24 plus 1 mm, rounded up");
    // The ⌀10 end faces the tailstock: its axial hole is drilled from there.
    assert_eq!(lathe["free_end"], 52.0);
    let drill = &lathe["operations"][3];
    assert_eq!(drill["features"], json!(["holes:3"]));
    assert_eq!(drill["tool"], "drill4.2");
    assert_eq!(lathe["operations"][2]["tool"], "tt-finish", "the smaller nose radius finishes");
    assert_eq!(lathe["operations"][4]["tool"], "pt2");
    // The mill keeps only the cross hole: no facing from a block, and not the hole the lathe drilled.
    let mill: Vec<&Value> = setups.iter().skip(1).collect();
    assert_eq!(mill.len(), 1);
    assert_eq!(kinds(mill[0]), ["holes"]);
    assert_eq!(mill[0]["operations"][0]["features"], json!(["holes:7"]));
    // The shoulder and end faces lie on the profile: made by turning, not skipped.
    assert!(plan["skipped"].as_array().unwrap().is_empty(), "{}", plan["skipped"]);
    for rule in ["lathe.route@1", "lathe.orientation@1", "lathe.bar@1", "lathe.turned_surface@1", "lathe.drilled_on_lathe@1", "tool.lathe.parting@1"] {
        assert!(rules(&plan).contains(&rule), "{rule}");
    }
}

#[test]
fn the_lathe_setup_is_planned_not_posted() {
    let library = read("demo/tools.json");
    let plan = finished(&library);
    let (jobs, log) = step2cam::job::jobs(&plan, &library).unwrap();
    assert!(jobs.iter().all(|j| j.setup != "lathe"));
    assert!(log.iter().any(|l| l.contains("does not yet hand lathe setups to camlib")), "{log:?}");
}

#[test]
fn without_a_lathe_the_turned_steps_are_not_cut() {
    let mut library = read("demo/tools.json");
    library["machines"].as_array_mut().unwrap().retain(|m| m["kind"] != "lathe");
    let plan = finished(&library);
    assert!(plan["setups"].as_array().unwrap().iter().all(|s| s["kind"] != "lathe"));
    let skipped: Vec<&str> = plan["skipped"].as_array().unwrap().iter().map(|s| s["feature"].as_str().unwrap()).collect();
    assert!(skipped.contains(&"turned_steps:9"), "{skipped:?}");
    let route = plan["decisions"].as_array().unwrap().iter().find(|d| d["rule"] == "lathe.route@1").unwrap();
    assert_eq!(route["outcome"], "none");
}

#[test]
fn bar_larger_than_the_lathe_turns_is_not_turned() {
    let mut library = read("demo/tools.json");
    let lathe = library["machines"].as_array_mut().unwrap().iter_mut().find(|m| m["kind"] == "lathe").unwrap();
    lathe["swing_mm"]["over_carriage"] = json!(20);
    let plan = finished(&library);
    assert!(plan["setups"].as_array().unwrap().iter().all(|s| s["kind"] != "lathe"));
    assert!(plan["warnings"].to_string().contains("larger than"), "{}", plan["warnings"]);
}

#[test]
fn the_lathes_spindle_caps_the_drill() {
    let plan = finished(&read("demo/tools.json"));
    let drill = &plan["setups"][0]["operations"][3];
    assert!(drill["technology"]["spindle_rpm"].as_f64().unwrap() <= 2800.);
}
