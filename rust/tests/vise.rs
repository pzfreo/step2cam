//! Clamping decisions and the actual camlib fixture checks use the same setup frame.
mod common;
use common::*;
use serde_json::{Value, json};

fn library() -> Value { fixture("../../tests/fixtures/placeholder-tools.json") }
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
            assert_eq!(plan["setups"][0]["clamping"]["grip_height_mm"], 3.8);
            assert_eq!(plan["setups"][0]["clamping"]["jaw_top_z"], -15.2);
            let decision = list(&plan["decisions"]).iter().find(|d| d["id"] == plan["setups"][0]["clamping"]["because"][0]).unwrap();
            let says = decision["says"].as_str().unwrap();
            assert!(says.contains("raise stock on parallels or similar so jaws grip only 3.8 mm"), "{says}");
            assert!(says.contains("through cuts are not checked against it"), "{says}");
            let ops = list(&plan["setups"][0]["operations"]);
            let rough = ops.iter().find(|o| o["kind"] == "roughing").unwrap();
            assert_eq!(rough["web_mm"], 4.8);
            assert_eq!(rough["floor_z"], -14.2);
            // Top setup caps carry the limit; the front setup's grip-history audit is not a cap.
            let jaw_warnings: Vec<_> = list(&plan["decisions"]).iter().filter(|d| d["rule"] == "machine.clamping@1" && d["outcome"] == "warning"
                && d["subject"].as_str().is_some_and(|s| s == "top" || s.starts_with("top:"))).collect();
            assert!(jaw_warnings.iter().any(|d| d["subject"] == "top:roughing"));
            assert!(jaw_warnings.iter().any(|d| d["subject"] == "top"));
            for warning in jaw_warnings {
                assert_eq!(warning["jaw_cap_z"], -14.2, "{}", warning["says"]);
            }
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
            if name == "synthetic/stepped-plate" && job.setup == "top" {
                assert!(job.notes.iter().any(|note| note.contains("leaves a 4.8 mm band of uncut walls")), "{:?}", job.notes);
            }
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
    // Also exercise step2cam's refusal boundary, not just the passive camlib report.
    // A raised jaw covering the facing region must stop this job and retain its name.
    let mut blocked = jobs[0].manifest.clone();
    let body = &mut blocked["verification"]["scene"]["scene"]["bodies"][0];
    body["footprint"]["components"][0]["outer"]["vertices"] = json!([[-100., -100., 0.], [200., -100., 0.], [200., 200., 0.], [-100., 200., 0.]]);
    body["z_max_mm"] = json!(1.);
    let failure = step2cam::simulate::simulate(&blocked.to_string()).err().expect("raised jaw must refuse the job");
    assert!(failure.message.contains("fixture/scene check"), "{}", failure.message);
    assert!(failure.message.contains(jaw["id"].as_str().unwrap()), "{}", failure.message);

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
    assert!(trace.warnings.iter().any(|w| w == "setup top is not cut: grip 3.175 mm exceeds maximum grip 2.0 mm (50.0% of 4.0 mm stock height)"));
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

#[test]
fn an_outside_chamfer_tip_must_stay_above_the_jaws() {
    let mut plan = json!({"setups": [{"id": "top", "stock": {"min": [0., 0., -10.], "max": [30., 20., 0.]},
        "operations": [{"kind": "edge_chamfer", "top_z": -5., "width": 1., "radius": 1., "boundaries": "outer", "features": ["fillets:0"]}]}]});
    capp::finish(&mut plan, &library(), &[]).unwrap();
    assert!(list(&plan["setups"][0]["operations"]).is_empty());
    assert!(plan["skipped"].to_string().contains("jaw clearance cap"));
}

fn refusal(plan: &Value) -> &str {
    warnings(plan).into_iter().find(|w| w.starts_with("setup top is not cut: ")).unwrap().strip_prefix("setup top is not cut: ").unwrap()
}

#[test]
fn each_grip_refusal_names_the_failed_limit_and_the_effect_of_parallels() {
    let mut tools = library();
    let vise = &mut tools["machines"][0]["workholding"][0];
    vise["parallels_height_mm"] = json!(10.);
    assert_eq!(refusal(&planned("plate", &tools)), "grip 10.0 mm set by 10.0 mm parallels exceeds maximum grip 5.5 mm (50.0% of 11.0 mm stock height); use taller parallels");
    tools["machines"][0]["workholding"][0]["parallels_height_mm"] = json!(15.);
    assert_eq!(refusal(&planned("mounting-plate", &tools)), "grip 5.0 mm set by 15.0 mm parallels exceeds maximum grip 4.5 mm (50.0% of 9.0 mm stock height); use taller parallels");
    tools["machines"][0]["workholding"][0]["parallels_height_mm"] = json!(18.);
    assert_eq!(refusal(&planned("plate", &tools)), "grip 2.0 mm set by 18.0 mm parallels is below minimum grip 3.175 mm; use shorter parallels");
    for (bottom, top, message) in [
        (-140., -1., "grip 28.0 mm exceeds jaw height 20.0 mm"),
        (-10., -6., "part top Z-6.0 does not clear jaw top Z-6.825 plus 1.0 mm side-cut clearance: must be above Z-5.825"),
    ] {
        let mut plan = json!({"setups":[{"id":"top", "stock":{"min":[0,0,bottom],"max":[30,20,0]}, "operations":[{"kind":"facing","floor_z":top}]}]});
        let mut trace = capp::trace::Trace::default();
        capp::machines::place(&mut plan, &library(), &mut trace);
        trace.store(&mut plan);
        assert_eq!(refusal(&plan), message);
        assert!(list(&plan["setups"][0]["operations"]).is_empty());
    }
}

#[test]
fn supplied_clamping_is_recorded_as_given() {
    let mut tools = library();
    tools["decisions"] = clamping("top", json!({"axis":"x", "grip_height_mm":3.5, "parallels_height_mm":16.5}));
    let validator = jsonschema::validator_for(&fixture("../../schemas/resources-1.schema.json")).unwrap();
    assert!(validator.is_valid(&tools));
    let plan = planned("plate", &tools);
    let c = &plan["setups"][0]["clamping"];
    assert_eq!(c["faces"], json!(["stock.x.min", "stock.x.max"]));
    assert_eq!(c["grip_height_mm"], 3.5);
    assert_eq!(c["jaw_top_z"], -7.5);
    assert_eq!(list(&c["parallels"]).len(), 2);
    let given = list(&plan["decisions"]).iter().find(|d| d["id"] == c["because"][0]).unwrap();
    assert_eq!(given["outcome"], "given");
    assert_eq!(given["source"], "user");
    assert_eq!(given["reason"], "shop clamping");
    let mut replay = capp::plan::plan(&fixture("plate.analysis.json"), Some(&fixture("plate.mesh.json")), &Default::default()).unwrap();
    replay["supplied_decisions"] = plan["supplied_decisions"].clone();
    capp::finish(&mut replay, &library(), &[]).unwrap();
    assert_eq!(replay["setups"][0]["clamping"], plan["setups"][0]["clamping"]);
    assert_eq!(replay["supplied_decisions"][0]["accepted"], true);
    assert!(given["says"].as_str().unwrap().contains("given clamping override, validated"));
    assert!(jsonschema::validator_for(&fixture("../../schemas/plan-1.schema.json")).unwrap().is_valid(&plan));
    let (jobs, _) = step2cam::job::jobs(&plan, &tools).unwrap();
    assert_eq!(list(&jobs[0].manifest["verification"]["scene"]["scene"]["bodies"]).len(), 4);
    step2cam::simulate::simulate(&jobs[0].manifest.to_string()).unwrap_or_else(|e| panic!("{}", e.message));

    // Inherit the vise's parallels unless the caller explicitly disables them with null.
    tools["machines"][0]["workholding"][0]["parallels_height_mm"] = json!(16.5);
    tools["decisions"][0]["choice"].as_object_mut().unwrap().remove("parallels_height_mm");
    assert_eq!(list(&planned("plate", &tools)["setups"][0]["clamping"]["parallels"]).len(), 2);
    tools["decisions"][0]["choice"]["parallels_height_mm"] = Value::Null;
    assert!(list(&planned("plate", &tools)["setups"][0]["clamping"]["parallels"]).is_empty());
}

#[test]
fn clamping_proposals_are_checked_rather_than_silently_replaced() {
    for (proposal, reason) in [
        (json!({"axis":"z","grip_height_mm":4.}), "axis must be x or y"),
        (json!({"axis":"x","grip_height_mm":2.}), "below minimum grip 3.175 mm"),
        (json!({"axis":"x","grip_height_mm":25.}), "exceeds jaw height 20.0 mm"),
        (json!({"axis":"x","grip_height_mm":6.}), "exceeds maximum grip 5.5 mm"),
        (json!({"axis":"x","grip_height_mm":4.,"parallels_height_mm":15.}), "disagrees with 5.0 mm grip set by 15.0 mm parallels"),
        (json!({"axis":"x","grip_height_mm":4.,"parallels_height_mm":-1.}), "parallels height must be positive and finite"),
        (json!({"axis":"x","grip_height_mm":4.,"grpi":4.}), "unknown field grpi"),
    ] {
        let mut tools = library();
        tools["decisions"] = clamping("top", proposal);
        let plan = planned("plate", &tools);
        assert!(!list(&plan["setups"][0]["operations"]).is_empty());
        for key in ["faces", "grip_height_mm", "jaw_top_z", "clearance_mm", "jaws", "parallels"] {
            assert_eq!(plan["setups"][0]["clamping"][key], planned("plate", &library())["setups"][0]["clamping"][key]);
        }
        assert!(plan["supplied_decisions"][0]["refusal_reason"].as_str().unwrap().contains(reason));
        assert!(warnings(&plan).iter().any(|w| w.contains(reason) && w.contains("rule's own choice is used")));
    }
    let mut tools = library();
    tools["decisions"] = clamping("top", json!({"axis":"x","grip_height_mm":4.}));
    tools["machines"][0]["workholding"][0]["max_jaw_opening_mm"] = json!(70.);
    assert_eq!(planned("plate", &tools)["supplied_decisions"][0]["refusal_reason"], "stock is 104.0 mm across the gripped X axis, wider than the vise opens (70.0 mm)");
    tools["machines"][0]["workholding"][0].as_object_mut().unwrap().remove("jaw_height_mm");
    assert!(planned("plate", &tools)["supplied_decisions"][0]["refusal_reason"].as_str().unwrap().contains("cannot be validated without vise jaw width, height and opening"));
    tools["decisions"] = clamping("typo", json!({"axis":"x","grip_height_mm":4.}));
    let mut plan = json!({"setups":[]});
    capp::finish(&mut plan, &tools, &[]).unwrap();
    assert_eq!(plan["supplied_decisions"][0]["refusal_reason"], "subject or operation matches no decision");
}

fn history_plan(operations: Value, rotation: Value, origin: Value) -> Value {
    json!({"setups":[
        {"id":"top","rotation":[[1,0,0],[0,1,0],[0,0,1]],"origin_model":[0,0,0],"stock":{"min":[0,0,-20],"max":[40,30,0]},"operations":operations},
        {"id":"front","rotation":rotation,"origin_model":origin,"stock":{"min":[0,0,-30],"max":[40,20,0]},"operations":[{"kind":"holes","diameter":3.,"top_z":0.,"bottom_z":-5.,"bottom":"drill_point","centers":[[20,10]],"features":[]}]}
    ]})
}
fn audit(plan: &Value) -> &Value {
    let primary = &plan["setups"][1]["clamping"]["because"][0];
    &list(&plan["decisions"]).iter().find(|d| d["id"] == *primary).unwrap()["chosen"]["prior_setups"]
}

#[test]
fn later_grip_bands_are_transformed_and_warn_about_earlier_removed_material() {
    let front = json!([[1,0,0],[0,0,1],[0,-1,0]]);
    let mut plan = history_plan(json!([{"kind":"facing","floor_z":-2.}]), front.clone(), json!([0,0,-20]));
    capp::finish(&mut plan, &library(), &[]).unwrap();
    let check = audit(&plan);
    assert_eq!(check["status"], "possible_overlap");
    assert_eq!(check["overlaps"], json!([{"operation":"top-1-facing","faces":["stock.y.max"]}]));
    assert!(check["scope"].as_str().unwrap().contains("not a stock simulation or proof of face retention"));
    assert!(warnings(&plan).iter().any(|w| w.contains("earlier cuts may have removed material in the later grip band")));
    assert!(list(&plan["setups"][1]["operations"]).iter().any(|o| o["kind"] == "holes"), "the conservative audit warns, rather than removing later cuts");

    let mut moved = history_plan(json!([{"kind":"facing","floor_z":-2.}]), front.clone(), json!([0,50,-20]));
    capp::finish(&mut moved, &library(), &[]).unwrap();
    assert_eq!(audit(&moved)["status"], "no_overlap", "origin_model must affect the comparison");
    let mut excluded = history_plan(json!([{"kind":"facing","floor_z":-2.}]), front, json!([0,0,-20]));
    capp::finish(&mut excluded, &library(), &[json!({"operation":"top-1-facing","message":"test exclusion"})]).unwrap();
    assert_eq!(audit(&excluded)["status"], "no_overlap", "refused earlier operations must not invent removed material");
}

#[test]
fn later_grip_audit_uses_capped_depths_and_admits_missing_information() {
    let mut plan = history_plan(json!([{"kind":"roughing","top_z":0.,"floor_z":-19.,"web_mm":0.5}]), json!([[1,0,0],[0,1,0],[0,0,1]]), json!([0,0,0]));
    plan["setups"][1]["stock"] = plan["setups"][0]["stock"].clone();
    capp::finish(&mut plan, &library(), &[]).unwrap();
    assert_eq!(plan["setups"][0]["operations"][0]["floor_z"], -15.);
    assert_eq!(audit(&plan)["status"], "no_overlap", "the later grip band ends at -16, below the earlier capped floor -15");
    let mut tools = library();
    tools["decisions"] = clamping("front", json!({"axis":"y","grip_height_mm":8.}));
    let mut deeper = plan.clone();
    capp::finish(&mut deeper, &tools, &[]).unwrap();
    assert_eq!(audit(&deeper)["status"], "possible_overlap", "a deeper grip overlaps earlier roughing");
    let mut missing = plan.clone();
    missing["setups"][0].as_object_mut().unwrap().remove("rotation");
    capp::finish(&mut missing, &library(), &[]).unwrap();
    assert_eq!(audit(&missing)["status"], "unchecked");
    assert!(warnings(&missing).iter().any(|w| w.contains("could not be checked completely")));
}

fn clamping(subject: &str, choice: Value) -> Value {
    json!([{"rule":"machine.clamping", "subject":subject, "choice":choice, "source":"user", "reason":"shop clamping"}])
}
#[test]
fn deprecated_clamping_still_works() {
    let mut tools = library();
    tools["overrides"] = json!({"clamping":{"top":{"axis":"x","grip_height_mm":3.5}}});
    let plan = planned("plate", &tools);
    assert_eq!(plan["supplied_decisions"][0]["accepted"], true);
    assert_eq!(plan["setups"][0]["clamping"]["grip_height_mm"], 3.5);
}
