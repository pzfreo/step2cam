//! Drawing requirements the plan does not act on are warned of, one each, with where they apply.
mod common;
use common::fixture;
use serde_json::{Value, json};

/// The plan's requirement.not_acted_on decisions.
fn unacted(analysis: &str) -> (Value, Vec<Value>) {
    let plan = capp::plan::plan(&fixture(analysis), None, &Default::default()).unwrap();
    let found = plan["decisions"].as_array().unwrap().iter().filter(|d| d["rule"] == "requirement.not_acted_on@1").cloned().collect();
    (plan, found)
}

fn by_key<'a>(found: &'a [Value], key: &str) -> &'a Value {
    let mut hits = found.iter().filter(|d| d["requirements"].as_array().unwrap().iter().any(|k| k == key));
    let hit = hits.next().unwrap_or_else(|| panic!("no warning for {key}"));
    assert!(hits.next().is_none(), "{key} warned of twice");
    hit
}

#[test]
fn stepped_plate_names_each_requirement_not_acted_on() {
    let (plan, found) = unacted("synthetic/stepped-plate.analysis.json");
    for d in &found {
        assert_eq!(d["outcome"], "warning");
        assert!(plan["warnings"].as_array().unwrap().contains(&d["says"]), "{d}");
    }
    let says = |key: &str| by_key(&found, key)["says"].as_str().unwrap().to_string();
    assert_eq!(says("part.general_tolerance"), "general tolerance ISO 2768-m: not checked");
    assert_eq!(says("part.surface_finish"), "surface finish Ra 3.2: not checked");
    assert!(says("part.edges").contains("Break sharp edges 0.2 max"));
    assert_eq!(says("datum.A.flatness"), "flatness 0.02 on datum A (face 4): not checked");
    assert_eq!(says("datum.B.control"), "perpendicularity 0.05 on datum B (face 0): not checked");
    assert_eq!(says("datum.C.control"), "perpendicularity 0.05 on datum C (face 5): not checked");
    let fit = by_key(&found, "hole.function:holes:36");
    assert_eq!(fit["features"], json!(["holes:36"]));
    assert!(fit["says"].as_str().unwrap().starts_with("fit H7 on the ⌀10 hole: not held"), "{fit}");
    // One warning for the position tolerances; the general pattern has none (Specify asks none).
    let position = by_key(&found, "hole.position:holes:36");
    let mut keys: Vec<&str> = position["requirements"].as_array().unwrap().iter().map(|k| k.as_str().unwrap()).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["hole.position:hole_patterns.rectangle:16.29.35.45", "hole.position:hole_patterns.set:48.49.57.58", "hole.position:holes:36"]);
    assert_eq!(position["says"], "position not checked: ⌀0.4 on the 4 ⌀4.4 holes (clearance M4); ⌀0.2 on the 2 ⌀2.5 holes (tapped M3); ⌀2 on the ⌀10 hole (fit H7)");
    // Acted on: the M3 thread and its depth, the M4 clearance size, the material; datums and
    // locating are choices.
    let keys: Vec<&str> = found.iter().flat_map(|d| d["requirements"].as_array().unwrap().iter().map(|k| k.as_str().unwrap())).collect();
    for key in ["hole.function:hole_patterns.set:48.49.57.58", "hole.full_thread:hole_patterns.set:48.49.57.58",
        "hole.function:hole_patterns.rectangle:16.29.35.45", "hole.position:hole_patterns.rectangle:17.22.30.31",
        "part.material", "part.coating", "part.heat_treatment", "part.locating", "datum.A"] {
        assert!(!keys.contains(&key), "{key}: {keys:?}");
    }
    assert_eq!(found.len(), 8, "{found:#?}");
}

#[test]
fn turned_fits_and_runouts_are_named() {
    let (_, found) = unacted("spindle.analysis.json");
    let fit = by_key(&found, "diameter.fit:turned_steps:6");
    assert_eq!(fit["features"], json!(["turned_steps:6"]));
    assert!(fit["says"].as_str().unwrap().starts_with("fit h6 on the ⌀"), "{fit}");
    let runout = by_key(&found, "diameter.runout:turned_steps:2");
    assert_eq!(by_key(&found, "diameter.runout:turned_steps:9")["id"], runout["id"]);
    assert!(runout["says"].as_str().unwrap().starts_with("runout not checked: 0.02 on"), "{runout}");
}

#[test]
fn a_part_with_no_requirements_has_no_such_warning() {
    let (_, found) = unacted("synthetic/square-slots.analysis.json");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_fit_on_a_pattern_names_its_holes() {
    let mut analysis = fixture("plate.analysis.json");
    analysis["resume"] = json!({"answers": {"hole.function:hole_patterns.grid:6.7.8.10.11.13": "fit:H7"}});
    let plan = capp::plan::plan(&analysis, None, &Default::default()).unwrap();
    let fits: Vec<&Value> = plan["decisions"].as_array().unwrap().iter().filter(|d| d["rule"] == "requirement.not_acted_on@1").collect();
    assert_eq!(fits.len(), 1, "{fits:?}");
    let mut holes: Vec<&str> = fits[0]["features"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    holes.sort_unstable();
    assert_eq!(holes, ["holes:10", "holes:11", "holes:13", "holes:6", "holes:7", "holes:8"]);
    assert!(fits[0]["says"].as_str().unwrap().starts_with("fit H7 on the 6 ⌀"), "{}", fits[0]["says"]);
}
