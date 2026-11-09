//! What the plan, tools and threads tests share: synthetic edits of the analysis, as the Python
//! tests made them. Each test binary uses only some of this.
#![allow(dead_code)]
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use capp::plan::Options;

pub type Bounds = ([f64; 3], [f64; 3]);
pub const BOX: Bounds = ([-20., -20., -10.], [20., 20., 0.]);

pub fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures").join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
pub fn example_tools() -> Value {
    fixture("../../examples/tools.example.json")
}
pub fn binding() -> Value {
    json!({"source_sha256": "x", "face_count": 1, "loader_version": 2, "part": 0})
}

/// An analysis with these hole records (and other features and faces).
pub fn synthetic(holes: &[Value], bounds: Bounds, features: &[Value], faces: &[Value]) -> Value {
    let mut all: Vec<Value> = holes
        .iter()
        .enumerate()
        .map(|(n, r)| json!({"id": format!("holes:{n}"), "family": "holes", "record": r, "faces": []}))
        .collect();
    all.extend_from_slice(features);
    json!({
        "binding": binding(), "faces": faces, "features": all,
        "dimensioning": {"bounds": [bounds.0, bounds.1]},
    })
}
pub fn holes_only(holes: &[Value]) -> Value {
    synthetic(holes, BOX, &[], &[])
}

pub fn hole(axis: [f64; 3], location: [f64; 3], diameter: f64, depth: f64, bottom: &str) -> Value {
    json!({"axis": axis, "location": location, "diameter": diameter, "depth": depth, "bottom": bottom,
           "cbore": null, "spotface": null, "csink": null})
}
/// A hole opening at this point of the top face, its axis straight down.
pub fn down(x: f64, y: f64, diameter: f64, depth: f64, bottom: &str) -> Value {
    hole([0., 0., -1.], [x, y, 0.], diameter, depth, bottom)
}
pub fn with(mut record: Value, key: &str, value: Value) -> Value {
    record[key] = value;
    record
}
/// The analysis with these Specify answers stored in it.
pub fn tapped(answers: Value, mut analysis: Value) -> Value {
    analysis["resume"] = json!({"answers": answers});
    analysis
}

pub type Triangle = [[f64; 3]; 3];
/// A mesh of just these triangles, for the synthetic analysis.
pub fn mesh_of(triangles: &[Triangle], bounds: Bounds) -> Value {
    let positions: Vec<f64> = triangles.iter().flatten().flatten().copied().collect();
    json!({"format": "pmi-assist-mesh", "version": 2, "binding": binding(),
           "bbox": [bounds.0, bounds.1], "positions": positions,
           "indices": (0..3 * triangles.len()).collect::<Vec<_>>(), "faces": [], "missing": [], "edges": []})
}
/// Two triangles: a level square at `z`.
pub fn square(z: f64, half: f64, cx: f64, cy: f64) -> Vec<Triangle> {
    let (a, b) = ([cx - half, cy - half, z], [cx + half, cy - half, z]);
    let (c, d) = ([cx + half, cy + half, z], [cx - half, cy + half, z]);
    vec![[a, b, c], [a, c, d]]
}

pub fn top_stock(top_stock: f64) -> Options {
    Options { top_stock, ..Default::default() }
}

/// Equal as Python's == sees it: numbers by value, so 1 and 1.0 are the same.
pub fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, p)| y.get(k).is_some_and(|q| same(p, q))),
        _ => a == b,
    }
}
#[track_caller]
pub fn assert_same(actual: Value, expected: Value) {
    assert!(same(&actual, &expected), "\n  actual: {actual}\nexpected: {expected}");
}
/// As Python's assertAlmostEqual: the same to 7 places.
#[track_caller]
pub fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 5e-8, "{actual} != {expected}");
}
pub fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{v} is not a number"))
}
pub fn list(v: &Value) -> &Vec<Value> {
    v.as_array().unwrap_or_else(|| panic!("{v} is not a list"))
}
/// The texts of a plan's warnings.
pub fn warnings(plan: &Value) -> Vec<&str> {
    list(&plan["warnings"]).iter().map(|w| w.as_str().unwrap()).collect()
}

/// What a plan's first setup posts to: its camlib operations and tools, and every note.
pub fn posted(plan: &Value, library: &Value) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let (jobs, log) = step2cam::job::jobs(&finished(&plan, library), library).unwrap();
    let job = jobs.iter().find(|j| j.setup == plan["setups"][0]["id"].as_str().unwrap());
    let mut notes = log;
    let (mut operations, mut tools) = (vec![], vec![]);
    if let Some(job) = job {
        // camlib reads it: a wrong field name or value fails here.
        let manifest: camlib::stream::StreamManifest = serde_json::from_value(job.manifest.clone()).unwrap_or_else(|e| panic!("camlib refuses the job: {e}"));
        camlib::stream::StreamProducer::new(manifest).unwrap_or_else(|e| panic!("camlib refuses the job: {e}"));
        operations.clone_from(list(&job.manifest["operations"]));
        tools.clone_from(list(&job.manifest["tools"]));
        notes.extend(job.notes.iter().cloned());
    }
    (operations, tools, notes)
}
pub fn any_note(notes: &[String], text: &str) -> bool {
    notes.iter().any(|n| n.contains(text))
}

/// The plan as it is executed: settled, and each operation's tool named.
pub fn finished(plan: &Value, library: &Value) -> Value {
    let mut plan = plan.clone();
    capp::finish(&mut plan, library, &[]).unwrap();
    plan
}

pub fn example_tap(major: f64, pitch: f64, chamfer: f64) -> Value {
    json!({"id":"tap", "type":"tap", "number":20, "diameter":major, "pitch":pitch,
        "tap_type":"cut", "chamfer_form":"bottoming", "chamfer_pitches":chamfer,
        "thread_length":20, "shank_diameter":6, "overall_length":70, "stickout":25,
        "holder_length":30, "holder_diameter":20, "spindle_rpm":700, "coolant":"flood",
        "flute_style":"spiral_flute"})
}
