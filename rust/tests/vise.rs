//! Clamping decisions and the actual camlib fixture checks use the same setup frame.
mod common;
use common::*;
use serde_json::{Value, json};

fn library() -> Value { fixture("../../demo/tools.json") }
fn planned(name: &str, tools: &Value) -> Value {
    let mut p = capp::plan::plan(&fixture(&format!("{name}.analysis.json")), Some(&fixture(&format!("{name}.mesh.json"))), &Default::default()).unwrap();
    capp::finish(&mut p, tools, &[]).unwrap();
    p
}

#[test]
fn plate_jobs_have_clamping_and_camlib_clears_the_fixtures() {
    let tools = library();
    for name in ["plate", "mounting-plate", "synthetic/stepped-plate"] {
        let plan = planned(name, &tools);
        let validator = jsonschema::validator_for(&fixture("../../schemas/plan-1.schema.json")).unwrap();
        assert!(validator.is_valid(&plan), "{name}: {:?}", validator.iter_errors(&plan).map(|e| e.to_string()).collect::<Vec<_>>());
        for setup in list(&plan["setups"]) {
            assert_eq!(list(&setup["clamping"]["jaws"]).len(), 2, "{name}: {}", setup["id"]);
            let c = &setup["clamping"];
            let cap = number(&c["jaw_top_z"]) + number(&c["clearance_mm"]);
            for op in list(&setup["operations"]).iter().filter(|o| o["kind"] == "roughing" || o["kind"] == "wall_finish") {
                assert!(number(&op["floor_z"]) >= cap);
            }
        }
        if name == "synthetic/stepped-plate" {
            let ops = list(&plan["setups"][0]["operations"]);
            let slot = ops.iter().find(|o| o["kind"] == "slot").unwrap();
            assert_eq!(slot["floor_z"], -19., "the enclosed slot does not reach a jaw");
            for notch in ops.iter().filter(|o| o["kind"] == "notch" && o["open"] == true) {
                assert_close(number(&notch["floor_z"]), -14.2);
                assert_eq!(notch["bottom"], "blind", "no breakthrough below the cap");
            }
        }
        let (jobs, _) = step2cam::job::jobs(&plan, &tools).unwrap();
        assert!(!jobs.is_empty());
        for job in jobs {
            assert_eq!(list(&job.manifest["verification"]["scene"]["scene"]["bodies"]).len(), 2);
            step2cam::simulate::simulate(&job.manifest.to_string()).unwrap_or_else(|e| panic!("{name}/{}: {}", job.setup, e.message));
        }
        assert!(plan["warnings"].to_string().contains("later setup"));
        assert!(!plan["tools_needed"].to_string().contains("jaw"), "workholding is not fixed by a longer tool");
    }
}

#[test]
fn a_move_through_a_jaw_is_refused_and_names_the_fixture() {
    use camlib::{motion::{Curve, Motion, MoveKind}, stream::{StreamManifest, StreamProducer, DEFAULT_FRAME_LIMIT}, stream_verification::{Binding, Check}, verification::{Status, FindingKind}};
    let tools = library();
    let plan = planned("synthetic/stepped-plate", &tools);
    let (jobs, _) = step2cam::job::jobs(&plan, &tools).unwrap();
    let mut raw = jobs[0].manifest.clone();
    let jaw = &plan["setups"][0]["clamping"]["jaws"][0];
    let center: [f64; 3] = std::array::from_fn(|i| (number(&jaw["min"][i]) + number(&jaw["max"][i])) / 2.);
    let start = [center[0], center[1], number(&jaw["max"][2]) + 10.];
    raw["initial_position_mm"] = json!(start);
    raw["verification"]["material"] = json!([]);
    raw["verification"]["stock_contact"] = Value::Null;
    raw["tool_change"] = Value::Null;
    let manifest: StreamManifest = serde_json::from_value(raw).unwrap();
    let binding = Binding::new(&manifest).unwrap();
    let mut producer = StreamProducer::new(manifest.clone()).unwrap();
    let motion = Motion { id: 1, operation: manifest.operations[0].id().into(), tool: manifest.operations[0].tool().into(), kind: MoveKind::Plunge, curve: Curve::Line { start, end: center }, feed_mm_min: Some(100.) };
    let (_, bytes) = producer.push_checked_motion(motion.clone(), DEFAULT_FRAME_LIMIT, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    let checked = binding.decode(&bytes.unwrap(), producer.completed(), Some(&motion)).unwrap();
    assert_eq!(checked.check.motion_status(), Some(Status::Violation));
    let Check::Motion { report: Some(report), .. } = checked.check else { panic!("no scene report") };
    assert!(report.findings.iter().any(|f| f.kind == FindingKind::FixtureContact && f.object_id == jaw["id"].as_str().unwrap() && f.status == Status::Violation));
}

#[test]
fn complete_vise_rejects_wide_or_too_shallow_stock_but_missing_jaw_data_skips() {
    let mut tools = library();
    tools["machines"][0]["workholding"][0]["max_jaw_opening_mm"] = json!(1.);
    let plan = planned("synthetic/stepped-plate", &tools);
    assert!(list(&plan["setups"]).iter().all(|s| list(&s["operations"]).is_empty()));
    assert!(plan["warnings"].to_string().contains("wider than the vise opens"));
    assert!(list(&plan["covered_by_roughing"]).is_empty());
    assert!(!list(&plan["skipped"]).is_empty());
    tools["machines"][0]["workholding"][0].as_object_mut().unwrap().remove("jaw_width_mm");
    let legacy = planned("synthetic/stepped-plate", &tools);
    assert!(list(&legacy["setups"]).iter().any(|s| !list(&s["operations"]).is_empty()));
    assert!(legacy["decisions"].to_string().contains("fixture scene skipped"));
    let (jobs, _) = step2cam::job::jobs(&legacy, &tools).unwrap();
    assert!(jobs.iter().all(|j| j.manifest["verification"]["scene"].is_null()));
    let mut trace = capp::trace::Trace::default();
    let mut thin = json!({"setups":[{"id":"top","stock":{"min":[0,0,0],"max":[30,20,4]},"operations":[{"kind":"facing","floor_z":3.}]}]});
    capp::machines::place(&mut thin, &library(), &mut trace);
    assert!(list(&thin["setups"][0]["operations"]).is_empty());
    assert!(trace.warnings.iter().any(|w| w.contains("too little stock/part above")));
}

#[test]
fn parallels_are_recorded_and_passed_as_fixtures() {
    let mut tools = library();
    tools["machines"][0]["workholding"][0]["parallels_height_mm"] = json!(15.);
    let plan = planned("synthetic/stepped-plate", &tools);
    assert_eq!(list(&plan["setups"][0]["clamping"]["parallels"]).len(), 2);
    let (jobs, _) = step2cam::job::jobs(&plan, &tools).unwrap();
    assert_eq!(list(&jobs[0].manifest["verification"]["scene"]["scene"]["bodies"]).len(), 4);
}

#[test]
fn jaws_choose_the_narrower_axis_and_x_on_a_tie() {
    for (size, faces) in [([20., 40., 20.], ["stock.x.min", "stock.x.max"]), ([40., 20., 20.], ["stock.y.min", "stock.y.max"]), ([20., 20., 20.], ["stock.x.min", "stock.x.max"])] {
        let mut plan = json!({"setups": [{"id": "top", "stock": {"min": [7., 9., -20.], "max": [7. + size[0], 9. + size[1], 0.]}, "operations": []}]});
        capp::machines::place(&mut plan, &library(), &mut capp::trace::Trace::default());
        assert_eq!(plan["setups"][0]["clamping"]["faces"], json!(faces));
        assert_eq!(plan["setups"][0]["clamping"]["grip_height_mm"], 4.);
    }
}
