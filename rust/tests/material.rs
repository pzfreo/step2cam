//! camlib compares the stock left by the last move with the part. A height field cannot decide cells
//! beside a wall, so what it can say is: no definite gouge, and at least so much left.
use serde_json::Value;
use std::{fs, path::PathBuf};
use step2cam::post::{Machine, post};

fn fixture(path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures").join(path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
fn tools() -> Value {
    fixture("../../demo/tools.json")
}
fn process(part: &str) -> Value {
    let out = step2cam::process(&fixture(&format!("{part}.analysis.json")), &fixture(&format!("{part}.mesh.json")), &tools()).unwrap();
    assert!(out["post_error"].is_null(), "{part}: {}", out["post_error"]);
    out
}
fn left_on_part(out: &Value) -> f64 {
    out["programs"][0]["material"]["residual"]["volume_mm3"][0].as_f64().unwrap()
}
/// A part's first job with one of its operations changed, posted.
fn post_with(part: &str, kind: &str, change: &dyn Fn(&mut Value)) -> Result<step2cam::post::Program, step2cam::post::Failure> {
    let plan = step2cam::plan::plan(&fixture(&format!("{part}.analysis.json")), Some(&fixture(&format!("{part}.mesh.json"))), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&plan, &tools()).unwrap();
    let mut manifest = jobs[0].manifest.clone();
    change(manifest["operations"].as_array_mut().unwrap().iter_mut().find(|o| o["kind"] == kind).unwrap());
    let machine: Machine = serde_json::from_value(tools()["machine"].clone()).unwrap();
    post(&manifest.to_string(), machine)
}
fn refused(part: &str, kind: &str, change: &dyn Fn(&mut Value)) -> bool {
    match post_with(part, kind, change) {
        Ok(_) => false,
        Err(failure) => {
            assert!(failure.message.contains("cuts into the part"), "{}", failure.message);
            true
        }
    }
}
fn lower_facing(by: f64) -> impl Fn(&mut Value) {
    move |o| o["floor_z_mm"] = (o["floor_z_mm"].as_f64().unwrap() - by).into()
}

#[test]
fn correct_parts_finish_without_a_definite_gouge() {
    for part in ["plate", "mounting-plate", "synthetic/stepped-plate", "synthetic/stepped-plate-chamfered"] {
        let material = &process(part)["programs"][0]["material"];
        assert_eq!(material["complete"], true, "{part}");
        assert_eq!(material["gouge"]["definite_cells"], 0, "{part}");
    }
}

#[test]
fn it_finds_the_thickness_left_around_a_boss() {
    // The stepped plate's flats are not on a roughing layer: about 3 mm is left over most of it.
    assert!(left_on_part(&process("synthetic/stepped-plate")) > 10_000.);
    assert!(left_on_part(&process("mounting-plate")) < 1_000.);
}

#[test]
fn facing_into_the_part_is_refused() {
    assert!(!refused("plate", "facing", &lower_facing(0.)));
    assert!(refused("plate", "facing", &lower_facing(1.)));
    assert!(refused("plate", "facing", &lower_facing(0.1)));
}

#[test]
fn a_hole_drilled_in_the_wrong_place_is_refused() {
    assert!(refused("plate", "drilling", &|o| {
        for center in o["centers_mm"].as_array_mut().unwrap() {
            center[0] = (center[0].as_f64().unwrap() + 3.).into();
        }
    }));
}

#[test]
fn a_wall_cut_too_far_in_is_refused() {
    // The outside wall's path, moved towards the part's middle by about 0.5 mm.
    let shrink = |o: &mut Value| {
        let outer = &mut o["region"]["components"][0]["outer"]["vertices"];
        let points = outer.as_array_mut().unwrap();
        let middle = |k: usize, points: &[Value]| {
            let v = points.iter().map(|p| p[k].as_f64().unwrap());
            (v.clone().fold(f64::INFINITY, f64::min) + v.fold(f64::NEG_INFINITY, f64::max)) / 2.
        };
        let centre = [middle(0, points), middle(1, points)];
        let half = points.iter().map(|p| (p[0].as_f64().unwrap() - centre[0]).abs()).fold(0., f64::max);
        let scale = 1. - 0.5 / half;
        for p in points.iter_mut() {
            for k in 0..2 {
                p[k] = (centre[k] + (p[k].as_f64().unwrap() - centre[k]) * scale).into();
            }
        }
    };
    assert!(refused("mounting-plate", "contour", &shrink));
}

#[test]
fn the_log_says_what_was_found_and_what_was_not_compared() {
    let out = process("synthetic/stepped-plate");
    let log = out["log"].to_string();
    assert!(log.contains("setup top: finished stock against the part: no definite gouge"), "{log}");
    assert!(log.contains("setup front: the finished stock is not compared"), "{log}");
}

