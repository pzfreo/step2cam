//! Reference data: values from standards and recognised practice, kept as data in
//! `rulebase/reference/`, each table saying where its values came from and whether they have been
//! checked against the standard itself.
use serde_json::Value;
use std::sync::OnceLock;

/// The tables, as shipped with the crate.
pub const TABLES: [(&str, &str); 4] = [
    ("iso261-coarse-pitch.json", include_str!("../rulebase/reference/iso261-coarse-pitch.json")),
    ("full-thread.json", include_str!("../rulebase/reference/full-thread.json")),
    ("iso14649-names.json", include_str!("../rulebase/reference/iso14649-names.json")),
    ("iso513-groups.json", include_str!("../rulebase/reference/iso513-groups.json")),
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

/// The ISO 14649 names for an operation once its tool is known: the feature it machines, the
/// operation, a hole's bottom condition, and whether the names fit it exactly
/// (`iso14649-names.json`). None for a kind the table does not have.
pub fn iso14649(op: &Value, tool_type: Option<&str>) -> Option<Value> {
    static NAMES: OnceLock<Value> = OnceLock::new();
    let names = NAMES.get_or_init(|| table("iso14649-names.json"));
    let entry = names["kinds"].get(op["kind"].as_str()?)?;
    let feature = match entry.get("feature_by_open") {
        Some(by) => by[if op["open"] == true { "true" } else { "false" }].clone(),
        None => entry["feature"].clone(),
    };
    let operation = match entry.get("operation_by_tool") {
        Some(by) => tool_type.and_then(|t| by.get(t)).cloned().unwrap_or(Value::Null),
        None => entry["operation"].clone(),
    };
    let mut out = serde_json::json!({"feature": feature, "operation": operation, "exact": entry["exact"]});
    if let Some(bottom) = entry.get("bottom_condition").and_then(|b| b.get(op["bottom"].as_str()?)) {
        out["bottom_condition"] = bottom.clone();
    }
    if let Some(note) = entry.get("note") {
        out["note"] = note.clone();
    }
    Some(out)
}

/// The ISO 513 group a material named in Specify falls in, and the words that placed it: the
/// first entry of `iso513-groups.json`'s `by_words` with a word in the name, ignoring case. None
/// for a name it cannot place, or one it never places (a printed material).
pub fn iso513_group(material: &str) -> Option<(String, String)> {
    static TABLE: OnceLock<Value> = OnceLock::new();
    let table = TABLE.get_or_init(|| table("iso513-groups.json"));
    let name = material.to_lowercase();
    let never = table["placing"]["never"].as_array()?;
    if never.iter().filter_map(Value::as_str).any(|w| name.contains(w)) {
        return None;
    }
    for entry in table["placing"]["by_words"].as_array()? {
        if let Some(word) = entry[0].as_array()?.iter().filter_map(Value::as_str).find(|w| name.contains(*w)) {
            return Some((entry[1].as_str()?.to_string(), word.to_string()));
        }
    }
    None
}
