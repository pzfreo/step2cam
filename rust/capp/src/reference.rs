//! Reference data: values from standards and recognised practice, kept as data in
//! `rulebase/reference/`, each table saying where its values came from and whether they have been
//! checked against the standard itself.
use crate::model::{Cut, Operation};
use serde_json::Value;
use std::sync::OnceLock;

/// The tables, as shipped with the crate.
pub const TABLES: [(&str, &str); 7] = [
    ("tap-drills.json", include_str!("../rulebase/reference/tap-drills.json")),
    ("drilling.json", include_str!("../rulebase/reference/drilling.json")),
    ("iso13399-fields.json", include_str!("../rulebase/reference/iso13399-fields.json")),
    ("iso261-coarse-pitch.json", include_str!("../rulebase/reference/iso261-coarse-pitch.json")),
    ("full-thread.json", include_str!("../rulebase/reference/full-thread.json")),
    ("iso14649-names.json", include_str!("../rulebase/reference/iso14649-names.json")),
    ("iso513-groups.json", include_str!("../rulebase/reference/iso513-groups.json")),
];

fn table(name: &str) -> Value {
    let (_, text) = TABLES.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no reference table {name}"));
    serde_json::from_str(text).unwrap_or_else(|e| panic!("rulebase/reference/{name}: {e}"))
}

/// Exact, type-specific tool property aliases and the fields deliberately left unmapped.
pub fn iso13399_fields() -> &'static Value {
    static FIELDS: OnceLock<Value> = OnceLock::new();
    FIELDS.get_or_init(|| table("iso13399-fields.json"))
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
    let (less, step, least) = full_thread_parameters();
    let down = ((depth - less * pitch) / step).floor() * step;
    if down > least * pitch { down } else { least * pitch }
}

/// The default full thread a tap can cut when only `room` mm of the drilled full diameter lies
/// below the thread's top after its lead and bottom clearance: `room` rounded down as
/// [`full_thread`] rounds, or None when that is less than its least full thread.
pub fn full_thread_within(room: f64, pitch: f64) -> Option<f64> {
    let (_, step, least) = full_thread_parameters();
    let down = ((room + 1e-9) / step).floor() * step;
    (down >= least * pitch - 1e-9).then_some(down)
}

fn full_thread_parameters() -> (f64, f64, f64) {
    static PARAMETERS: OnceLock<(f64, f64, f64)> = OnceLock::new();
    *PARAMETERS.get_or_init(|| {
        let p = &table("full-thread.json")["parameters"];
        let get = |key: &str| p[key].as_f64().unwrap_or_else(|| panic!("full-thread.json has no {key}"));
        (get("less_pitches"), get("round_down_mm"), get("at_least_pitches"))
    })
}

/// The ISO 14649 names for an operation once its tool is known: the feature it machines, the
/// operation, a hole's bottom condition, and whether the names fit it exactly
/// (`iso14649-names.json`). None for a kind the table does not have.
pub(crate) fn iso14649(op: &Operation, tool_type: Option<&str>) -> Option<Value> {
    static NAMES: OnceLock<Value> = OnceLock::new();
    let names = NAMES.get_or_init(|| table("iso14649-names.json"));
    let entry = names["kinds"].get(op.cut.kind().name())?;
    let (open, bottom) = match &op.cut {
        Cut::Notch(notch) => (notch.open, Some(notch.bottom.as_str())),
        Cut::Holes(holes) => (false, Some(holes.bottom.as_str())),
        Cut::Slot(slot) => (false, Some(slot.bottom.as_str())),
        _ => (false, None),
    };
    let feature = match entry.get("feature_by_open") {
        Some(by) => by[if open { "true" } else { "false" }].clone(),
        None => entry["feature"].clone(),
    };
    let operation = match entry.get("operation_by_tool") {
        Some(by) => tool_type.and_then(|t| by.get(t)).cloned().unwrap_or(Value::Null),
        None => entry["operation"].clone(),
    };
    let exact = match entry.get("exact_by_tool") {
        Some(by) => tool_type.and_then(|t| by.get(t)).cloned().unwrap_or(Value::Bool(false)),
        None => entry["exact"].clone(),
    };
    let mut out = serde_json::json!({"feature": feature, "operation": operation, "exact": exact});
    if let Some(bottom) = entry.get("bottom_condition").and_then(|b| b.get(bottom?)) {
        out["bottom_condition"] = bottom.clone();
    }
    if let Some(note) = entry.get("note") {
        out["note"] = note.clone();
    }
    Some(out)
}

/// The ISO 513 group a material named in Specify falls in, and the word that placed it
/// (`iso513-groups.json`): the name's lower-case words are matched whole, a phrase as consecutive
/// words. None for a name with a word in `never` (a printing process), or one nothing matches.
pub fn iso513_group(material: &str) -> Option<(String, String)> {
    static TABLE: OnceLock<Value> = OnceLock::new();
    let table = TABLE.get_or_init(|| table("iso513-groups.json"));
    let words: Vec<String> = material.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect();
    let spaced = format!(" {} ", words.join(" "));
    let has = |phrase: &str| spaced.contains(&format!(" {phrase} "));
    if table["placing"]["never"].as_array()?.iter().filter_map(Value::as_str).any(has) {
        return None;
    }
    for entry in table["placing"]["by_words"].as_array()? {
        if let Some(word) = entry[0].as_array()?.iter().filter_map(Value::as_str).find(|w| has(w)) {
            return Some((entry[1].as_str()?.to_string(), word.to_string()));
        }
    }
    None
}

fn drilling_parameters() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    &DATA.get_or_init(|| table("drilling.json"))["parameters"]
}

/// Depth/D only for holes beyond the reference's deep-hole warning threshold.
/// Use the same depth below the entry surface and drill diameter as the peck decision.
pub(crate) fn deep_drilling_ratio(depth: f64, diameter: f64) -> Option<f64> {
    (depth > drilling_parameters()["deep_hole_above_d"].as_f64().unwrap() * diameter)
        .then_some(depth / diameter)
}

/// Default drilling cycle for conventional drills. Depth runs from where the drill enters (any
/// stock left above the mouth) and includes requested breakthrough; point height is a reach
/// allowance, not the hole's depth.
/// A positive library peck is a cap only when the rule calls for pecking; zero/absent is no cap.
pub(crate) fn drilling(depth: f64, diameter: f64, tool: &Value) -> Value {
    let p = drilling_parameters();
    let peck = if depth <= p["no_peck_through_d"].as_f64().unwrap() * diameter {
        None
    } else {
        let increment = p["peck_d"].as_f64().unwrap() * diameter;
        Some(tool["peck_mm"].as_f64().filter(|v| v.is_finite() && *v > 0.)
            .map_or(increment, |cap| cap.min(increment)))
    };
    serde_json::json!({"peck_mm": peck, "peck_mode": p["peck_mode"]})
}

/// Tap drill reference and shop process policy; no hole geometry is changed.
pub(crate) fn tap_drills() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| table("tap-drills.json"))
}

pub fn tap_drill(major: f64, pitch: f64, form: bool) -> Option<f64> {
    let data = tap_drills();
    if form {
        return Some(((major - 0.0068 * data["form"]["percent_thread"].as_f64()? * pitch) * 100.).round() / 100.);
    }
    data["cut_rows"].as_array()?.iter().find(|r| r[0].as_f64() == Some(major) && r[1].as_f64() == Some(pitch))?[2].as_f64()
}
