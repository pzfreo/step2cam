//! The reference tables in `capp/rulebase/reference/`: each says where its values came from and
//! whether they have been checked against the standard itself, and agrees with spot values read
//! from its sources.
use capp::reference::{TABLES, coarse_pitch, full_thread};
use serde_json::Value;

#[test]
fn every_table_says_where_it_came_from() {
    for (name, text) in TABLES {
        let table: Value = serde_json::from_str(text).unwrap();
        assert_eq!(table["format"], "capp-reference", "{name}");
        let source = &table["source"];
        assert!(source["obtained_from"].as_array().is_some_and(|s| !s.is_empty()), "{name}: no sources");
        // The new cut-tap rows were checked directly in the published ISO 2306 preview.
        assert_eq!(source["checked_against_standard"], name == "tap-drills.json", "{name}");
        if name == "tap-drills.json" {
            assert!(source["obtained_from"].as_array().unwrap().iter().any(|s| s.as_str().unwrap().contains("iso-2306-1972.pdf")));
            assert_eq!(table["form"]["checked_against_standard"], false);
        }
    }
}

#[test]
fn a_rule_that_cites_a_table_cites_one_that_exists() {
    let rules: Value = serde_json::from_str(capp::trace::RULES).unwrap();
    for rule in rules["rules"].as_array().unwrap() {
        for source in rule["sources"].as_array().unwrap() {
            if let Some(via) = source["via"].as_str() {
                let file = via.strip_prefix("reference/").unwrap();
                assert!(TABLES.iter().any(|(name, _)| *name == file), "{}: {via}", rule["id"]);
            }
        }
    }
}

/// Spot values from both sources (Wikipedia's ISO metric screw thread page and Engineering
/// Toolbox's ISO 724 page, read 2026-11-07), which agree on every row.
#[test]
fn the_coarse_pitches_are_the_published_ones() {
    for (size, pitch) in [("M1.6", 0.35), ("M2.5", 0.45), ("M5", 0.8), ("M6", 1.0), ("M12", 1.75), ("M24", 3.0)] {
        assert_eq!(coarse_pitch(size), Some(pitch), "{size}");
    }
    assert_eq!(coarse_pitch("M7"), None, "not a size in the table");
}

/// specify-core's default: the drill depth less three pitches, rounded down to 0.5 mm, at least
/// one pitch.
#[test]
fn the_full_thread_is_specify_cores_default() {
    assert_eq!(full_thread(10., 0.8), 7.5); // 7.6 rounded down
    assert_eq!(full_thread(12., 1.0), 9.0);
    assert_eq!(full_thread(2., 1.0), 1.0); // at least one pitch
}

/// Every name the mapping uses is in the published lists it was read from, and every kind of
/// operation a plan holds has names.
#[test]
fn the_iso14649_names_are_published_names() {
    let (_, text) = TABLES.iter().find(|(n, _)| *n == "iso14649-names.json").unwrap();
    let table: Value = serde_json::from_str(text).unwrap();
    let listed = |key: &str| -> Vec<String> { table["names_in_source"][key].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect() };
    let (features, operations) = (listed("features"), listed("operations"));
    let names = |v: &Value| -> Vec<String> {
        match v {
            Value::String(s) => vec![s.clone()],
            Value::Object(o) => o.values().filter_map(Value::as_str).map(String::from).collect(),
            _ => vec![],
        }
    };
    let kinds = table["kinds"].as_object().unwrap();
    for kind in ["facing", "roughing", "flats", "spot_drill", "holes", "counterbore", "thread", "slot", "notch", "v_channel", "wall_finish", "edge_chamfer"] {
        let entry = kinds.get(kind).unwrap_or_else(|| panic!("no names for {kind}"));
        for f in names(&entry["feature"]).into_iter().chain(names(&entry["feature_by_open"])).chain(names(&entry["bottom_condition"])) {
            assert!(features.contains(&f), "{kind}: {f} is not a published feature name");
        }
        for o in names(&entry["operation"]).into_iter().chain(names(&entry["operation_by_tool"])) {
            assert!(operations.contains(&o), "{kind}: {o} is not a published operation name");
        }
        if entry["exact"] == false || entry.get("exact_by_tool").is_some() {
            assert!(entry["note"].is_string(), "{kind}: an inexact mapping says why");
        }
    }
}

#[test]
fn a_finished_plan_names_each_operation_in_iso14649() {
    let fixture = |p: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(p)).unwrap()).unwrap() };
    let mut plan = capp::plan::plan(&fixture("tests/fixtures/plate.analysis.json"), Some(&fixture("tests/fixtures/plate.mesh.json")), &Default::default()).unwrap();
    capp::finish(&mut plan, &fixture("examples/tools.example.json"), &[]).unwrap();
    for op in plan["setups"][0]["operations"].as_array().unwrap() {
        assert!(op["iso14649"]["feature"].is_string(), "{}", op["kind"]);
    }
    let holes: Vec<&Value> = plan["setups"][0]["operations"].as_array().unwrap().iter().filter(|o| o["kind"] == "holes").collect();
    assert!(holes.iter().any(|h| h["iso14649"]["operation"] == "drilling"));
    assert!(holes.iter().any(|h| h["iso14649"]["operation"] == "bottom_and_side_finish_milling"), "the bored hole");
    assert!(holes.iter().all(|h| h["iso14649"]["feature"] == "Round_hole" && h["iso14649"]["bottom_condition"].is_string()));
    for h in &holes {
        assert_eq!(h["iso14649"]["exact"], h["iso14649"]["operation"] == "drilling", "a bored hole's names are only the closest");
    }
}

#[test]
fn cut_tap_rows_match_iso2306_table1_and_form_is_separately_sourced() {
    for (major, pitch, drill) in [(3., 0.5, 2.5), (4., 0.7, 3.3), (5., 0.8, 4.2), (6., 1., 5.)] {
        assert_eq!(capp::reference::tap_drill(major, pitch, false), Some(drill));
        assert!((major - pitch - drill).abs() < 1e-9);
    }
    assert_eq!(capp::reference::tap_drill(3., 0.5, true), Some(2.78));
    assert_eq!(capp::reference::tap_drill(5., 0.8, true), Some(4.65));
    assert_eq!(capp::reference::tap_drill(5., 0.25, false), None);
}
