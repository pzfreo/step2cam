//! The reference tables in `capp/rulebase/reference/`: each says where its values came from and
//! that they have not been checked against the standard itself, and agrees with spot values read
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
        assert_eq!(source["checked_against_standard"], false, "{name}: only public sources are used");
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
