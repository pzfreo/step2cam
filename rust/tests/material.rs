//! camlib compares the stock left by the last move with the part. A height field cannot decide cells
//! beside a wall on its own; camlib also reports a separate per-motion gouge proof.
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use step2cam::simulate::simulate;

/// The plan as it is executed: settled, and each operation's tool named.
fn finished(plan: &Value, library: &Value) -> Value {
    let mut plan = plan.clone();
    capp::finish(&mut plan, library, &[]).unwrap();
    plan
}

fn fixture(path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures").join(path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
// The demo's tools on no particular machine: these tests are about planning, not the Carvera.
fn tools() -> Value {
    let mut tools = fixture("../../demo/tools.json");
    tools.as_object_mut().unwrap().remove("machines");
    tools
}
fn process(part: &str) -> Value {
    let out = step2cam::process(&fixture(&format!("{part}.analysis.json")), &fixture(&format!("{part}.mesh.json")), &tools()).unwrap();
    assert!(out["error"].is_null(), "{part}: {}", out["error"]);
    out
}
fn left_on_part(out: &Value) -> f64 {
    out["programs"][0]["material"]["residual"]["volume_mm3"][0].as_f64().unwrap()
}
/// A part's first job with the last of its operations of this kind changed, simulated.
fn simulate_with(part: &str, kind: &str, change: &dyn Fn(&mut Value)) -> Result<step2cam::simulate::Run, step2cam::simulate::Failure> {
    let plan = capp::plan::plan(&fixture(&format!("{part}.analysis.json")), Some(&fixture(&format!("{part}.mesh.json"))), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&finished(&plan, &tools()), &tools()).unwrap();
    let mut manifest = jobs[0].manifest.clone();
    change(manifest["operations"].as_array_mut().unwrap().iter_mut().rev().find(|o| o["kind"] == kind).unwrap());
    simulate(&manifest.to_string())
}
fn refused(part: &str, kind: &str, change: &dyn Fn(&mut Value)) -> bool {
    match simulate_with(part, kind, change) {
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
fn the_thickness_around_a_boss_is_cut_away() {
    // Without the flats operation about 3 mm stays on the stepped plate: 16,000 mm³. What is left
    // now includes the web around the part, about 500 mm³, which no operation removes.
    let out = process("synthetic/stepped-plate");
    assert!(left_on_part(&out) < 2_000., "{}", left_on_part(&out));
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
fn a_boss_wall_followed_down_into_the_plate_is_refused() {
    let part = "synthetic/stepped-plate";
    assert!(!refused(part, "contour", &|_| {}));
    assert!(refused(part, "contour", &|o| o["floor_z_mm"] = (o["floor_z_mm"].as_f64().unwrap() - 1.).into()));
}

#[test]
fn the_log_says_what_was_found_and_what_was_not_compared() {
    let out = process("synthetic/stepped-plate");
    let log = out["log"].to_string();
    assert!(log.contains("setup top: finished stock against the part: no definite gouge"), "{log}");
    assert!(log.contains("setup front: the finished stock is not compared"), "{log}");
}


fn threaded(plate: &str) -> Value {
    let mut analysis = fixture(&format!("{plate}.analysis.json"));
    analysis["resume"] = json!({"answers": {"hole.function:holes:12": "tapped:M5"}});
    analysis
}
fn threaded_job() -> Value {
    let plan = capp::plan::plan(&threaded("plate"), Some(&fixture("plate.mesh.json")), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&finished(&plan, &tools()), &tools()).unwrap();
    jobs[0].manifest.clone()
}

/// A thread goes past the bore the mesh shows: compared after it, every correct thread is a gouge.
/// And the fine grid it needs used to exhaust the check, which then said nothing.
#[test]
fn a_threaded_part_is_compared_right_through_before_its_threads() {
    let out = step2cam::process(&threaded("plate"), &fixture("plate.mesh.json"), &tools()).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    assert_eq!(out["programs"][0]["material"]["complete"], true);
    assert!(out["log"].to_string().contains("before thread milling"), "{}", out["log"]);
    let top = out["plan"]["setups"][0]["operations"].as_array().unwrap();
    assert_eq!(top.last().unwrap()["kind"], "thread");
}

#[test]
fn a_misplaced_hole_is_found_on_a_threaded_part_wherever_it_is() {
    let manifest = threaded_job();
    let holes = manifest["operations"].as_array().unwrap().iter().position(|o| o["kind"] == "drilling" && o["centers_mm"].as_array().is_some_and(|c| c.len() > 1)).expect("a pattern of holes");
    for hole in 0..manifest["operations"][holes]["centers_mm"].as_array().unwrap().len() {
        let mut wrong = manifest.clone();
        let x = wrong["operations"][holes]["centers_mm"][hole][0].as_f64().unwrap();
        wrong["operations"][holes]["centers_mm"][hole][0] = (x + 3.).into();
        let Err(failure) = simulate(&wrong.to_string()) else { panic!("hole {hole} moved 3 mm passed") };
        assert!(failure.message.contains("cuts into the part"), "{}", failure.message);
    }
}

#[test]
fn a_comparison_that_runs_out_of_budget_gives_no_program() {
    let plan = capp::plan::plan(&fixture("plate.analysis.json"), Some(&fixture("plate.mesh.json")), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&finished(&plan, &tools()), &tools()).unwrap();
    let mut manifest = jobs[0].manifest.clone();
    manifest["verification"]["material"][0]["options"]["max_boundary_tests"] = 1000.into();
    let Err(failure) = simulate(&manifest.to_string()) else { panic!("a comparison that did not finish passed") };
    assert!(failure.message.contains("ran out of budget"), "{}", failure.message);
    assert!(failure.operation.is_none());
}

/// camlib refusing roughing does not make the part's mesh any less known.
#[test]
fn the_stock_is_still_compared_when_roughing_is_refused() {
    let mut library = tools();
    for tool in library["tools"].as_array_mut().unwrap() {
        if tool["type"] == "end_mill" {
            tool["flute_length"] = 1.into();
            tool["stickout"] = 1.5.into();
        }
    }
    let out = step2cam::process(&fixture("plate.analysis.json"), &fixture("plate.mesh.json"), &library).unwrap();
    assert!(out["plan"]["warnings"].to_string().contains("roughing left out"), "{}", out["plan"]["warnings"]);
    assert!(out["programs"][0]["material"].is_object(), "{}", out["log"]);
    assert!(out.to_string().find("part_surface").is_none());
}

#[test]
fn stock_below_the_part_is_no_gouge() {
    let options = capp::plan::Options { bottom_stock: 2., ..Default::default() };
    for part in ["plate", "mounting-plate"] {
        let plan = capp::plan::plan(&fixture(&format!("{part}.analysis.json")), Some(&fixture(&format!("{part}.mesh.json"))), &options).unwrap();
        let (jobs, _) = step2cam::job::jobs(&finished(&plan, &tools()), &tools()).unwrap();
        simulate(&jobs[0].manifest.to_string()).map_err(|f| format!("{part}: {}", f.message)).unwrap();
    }
}

#[test]
fn the_log_says_how_small_an_error_the_check_can_miss() {
    let log = process("mounting-plate")["log"].to_string();
    assert!(log.contains("cell comparison can miss an error under about 0.04 mm in height or 0.26 mm sideways"), "{log}");
}

#[test]
fn the_per_motion_proof_reports_when_mesh_error_spends_its_tolerance() {
    let out = process("mounting-plate");
    let program = &out["programs"][0];
    let proof = &program["material"]["sweep_proof"];
    assert_eq!(proof["history_complete"], true);
    assert_eq!(proof["proven_moves"], 0);
    assert_eq!(proof["unproven_moves"].as_u64().unwrap(), program["moves"].as_array().unwrap().len() as u64);
    assert_eq!(proof["decided_cells"], 0);
    assert!(proof["unavailable"].as_str().unwrap().contains("does not exceed the target mesh's surface error"));
    assert!(out["log"].to_string().contains("per-motion gouge proof"));
}

#[test]
fn the_per_motion_proof_decides_the_channels_wall_cells() {
    let out = step2cam::process(
        &fixture("synthetic/walls/channel.analysis.json"),
        &fixture("synthetic/walls/channel.mesh.json"),
        &fixture("../../demo/tools.json"),
    ).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let program = &out["programs"][0];
    let material = &program["material"];
    let proof = &material["sweep_proof"];
    assert_eq!(material["complete"], true);
    assert_eq!(material["gouge"]["definite_cells"], 0);
    assert!(material["gouge"]["possible_only_cells"].as_u64().unwrap() > 0);
    assert_eq!(proof["decided_cells"], material["gouge"]["possible_only_cells"]);
    assert_eq!(proof["history_complete"], true);
    assert_eq!(proof["unproven_moves"], 0);
    assert_eq!(proof["proven_moves"].as_u64().unwrap(), program["moves"].as_array().unwrap().len() as u64);
    assert!(proof["unavailable"].is_null());
}
