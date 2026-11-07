//! Reference data: values from standards and recognised practice, kept as data in
//! `rulebase/reference/`, each table saying where its values came from and whether they have been
//! checked against the standard itself.
use serde_json::Value;
use std::sync::OnceLock;

/// The tables, as shipped with the crate.
pub const TABLES: [(&str, &str); 2] = [
    ("iso261-coarse-pitch.json", include_str!("../rulebase/reference/iso261-coarse-pitch.json")),
    ("full-thread.json", include_str!("../rulebase/reference/full-thread.json")),
];

fn table(name: &str) -> Value {
    let (_, text) = TABLES.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no reference table {name}"));
    serde_json::from_str(text).unwrap_or_else(|e| panic!("rulebase/reference/{name}: {e}"))
}

/// ISO 261's coarse pitch for a nominal size (`M5`), among the sizes the table lists.
pub fn coarse_pitch(size: &str) -> Option<f64> {
    static ROWS: OnceLock<Vec<(String, f64)>> = OnceLock::new();
    let rows = ROWS.get_or_init(|| {
        table("iso261-coarse-pitch.json")["rows"]
            .as_array()
            .expect("iso261-coarse-pitch.json has rows")
            .iter()
            .map(|row| (row[0].as_str().expect("a size").to_string(), row[1].as_f64().expect("a pitch")))
            .collect()
    });
    rows.iter().find(|(name, _)| name == size).map(|(_, pitch)| *pitch)
}

/// The minimum full thread in a blind tapped hole of drill depth `depth`, when none is asked for:
/// the depth less some pitches, rounded down, at least some pitches (`full-thread.json`).
pub fn full_thread(depth: f64, pitch: f64) -> f64 {
    static PARAMETERS: OnceLock<(f64, f64, f64)> = OnceLock::new();
    let (less, step, least) = *PARAMETERS.get_or_init(|| {
        let p = &table("full-thread.json")["parameters"];
        let get = |key: &str| p[key].as_f64().unwrap_or_else(|| panic!("full-thread.json has no {key}"));
        (get("less_pitches"), get("round_down_mm"), get("at_least_pitches"))
    });
    let down = ((depth - less * pitch) / step).floor() * step;
    if down > least * pitch { down } else { least * pitch }
}
