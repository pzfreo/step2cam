//! Policy, adapter and raster retry contracts, independent of absolute output hashes.
use capp::{roughing, trace::Trace};
use serde_json::{Value, json};

fn library() -> Value {
    serde_json::from_str(include_str!("../../libraries/haas-vf2.json")).unwrap()
}
fn check_plan(plan: &Value) {
    let schema: Value = serde_json::from_str(include_str!("../../schemas/plan-1.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<_> = validator.iter_errors(plan).map(|e| format!("{e} at {}", e.instance_path())).collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
fn material(group: Option<&str>) -> roughing::Material<'_> {
    roughing::Material {
        group,
        name: Some(if group == Some("N") {
            "aluminium"
        } else {
            "steel"
        }),
    }
}
fn data() -> serde_json::Map<String, Value> {
    let v = json!({"feed_mm_min":1000.,"spindle_rpm":5000.,"stepdown_mm":1.,"stepover_mm":2.,"plunge_mm_min":100.,"source":"tool"});
    v.as_object().unwrap().clone()
}
fn op() -> capp::model::Operation {
    serde_json::from_value(json!({"kind":"roughing","top_z":1.,"floor_z":-10.,"web_mm":0.5,"surface_error_mm":0.01})).unwrap()
}
fn tool() -> Value {
    json!({"diameter":10.,"flute_length":30.,"stickout":35.})
}

#[test]
fn sourced_groups_use_tool_dimensions_and_chip_thinning_with_feed_cap() {
    let mut library = library();
    library["machines"][0]["controller_settings"]["max_cutting_feed_mm_min"] = json!(1500.);
    for (group, width, factor) in [("N", 2.5, 1. / 0.75_f64.sqrt()), ("P", 1., 5. / 3.)] {
        let (mut op, mut technology, mut trace) = (op(), data(), Trace::default());
        roughing::choose(
            &mut op,
            &mut technology,
            &tool(),
            material(Some(group)),
            &library,
            "top:roughing",
            &mut trace,
        )
        .unwrap();
        assert_eq!(technology["clearing_strategy"], "bounded_engagement");
        assert_eq!(technology["radial_width_mm"], width);
        assert_eq!(technology["stepdown_mm"], 20.); // full flute capped by 2D
        assert!(
            (technology["feed_mm_min"].as_f64().unwrap() - (1000. * factor).min(1500.)).abs()
                < 1e-9
        );
        assert_eq!(technology["plunge_mm_min"], 100.);
        assert_eq!(op.raster_technology, Some(data()));
        assert!(trace.decisions[0]["policy"]["_sources"].is_object());
    }
    library["machines"][0]["controller_settings"]
        .as_object_mut()
        .unwrap()
        .remove("max_cutting_feed_mm_min");
    library["rapid_mm_min"] = json!(1100.);
    let mut technology = data();
    roughing::choose(
        &mut op(),
        &mut technology,
        &tool(),
        material(Some("P")),
        &library,
        "top:roughing",
        &mut Trace::default(),
    )
    .unwrap();
    assert_eq!(technology["feed_mm_min"], 1100.);
}

#[test]
fn missing_material_policy_stays_raster_and_says_why() {
    for group in [None, Some("M"), Some("K"), Some("S"), Some("H")] {
        let (mut op, mut technology, mut trace) = (op(), data(), Trace::default());
        roughing::choose(
            &mut op,
            &mut technology,
            &tool(),
            material(group),
            &library(),
            "top:roughing",
            &mut trace,
        )
        .unwrap();
        assert_eq!(technology["clearing_strategy"], "raster");
        assert_eq!(technology["stepdown_mm"], 1.);
        assert!(
            trace.decisions[0]["says"]
                .as_str()
                .unwrap()
                .contains("no sourced")
        );
    }
}

#[test]
fn refusal_restores_slotting_data_records_reason_and_cannot_retry_adaptive_again() {
    let (mut op, mut technology, mut trace) =
        (op(), data(), Trace::default());
    let decision = roughing::choose(
        &mut op,
        &mut technology,
        &tool(),
        material(Some("N")),
        &library(),
        "top:roughing",
        &mut trace,
    )
    .unwrap();
    op.technology = Some(technology);
    op.because = Some(vec![decision]);
    let a = serde_json::from_str(include_str!("../../tests/fixtures/plate.analysis.json")).unwrap();
    let m = serde_json::from_str(include_str!("../../tests/fixtures/plate.mesh.json")).unwrap();
    let mut plan = capp::plan::build(&a, Some(&m), &Default::default()).unwrap();
    plan.setups[0].operations = vec![op];
    plan.decisions = Some(trace.decisions);
    assert_eq!(
        roughing::retry_setup(&plan, &"top-1-roughing".parse().unwrap(), "test"),
        Some("top".into())
    );
    assert_eq!(roughing::retry_setup(&plan, &"top-2-flats".parse().unwrap(), "test"), None);
    roughing::fallbacks(
        &mut plan,
        &[("top".into(), "deep layer shank clearance refused".into())].into(),
    );
    assert_eq!(roughing::retry_setup(&plan, &"top-1-roughing".parse().unwrap(), "test"), None);
    let plan = serde_json::to_value(plan).unwrap();
    let op = &plan["setups"][0]["operations"][0];
    assert_eq!(op["technology"]["stepdown_mm"], 1.);
    assert_eq!(op["technology"]["feed_mm_min"], 1000.);
    assert_eq!(
        plan["decisions"][1]["refusal"],
        "deep layer shank clearance refused"
    );
    assert_eq!(
        plan["decisions"][0]["superseded_by"],
        plan["decisions"][1]["id"]
    );
    assert!(
        op["because"]
            .as_array()
            .unwrap()
            .contains(&plan["decisions"][1]["id"])
    );
}

#[test]
fn adapter_writes_explicit_stock_driven_strategy_and_no_orbit_options() {
    let a: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/plate.analysis.json")).unwrap();
    let m: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/plate.mesh.json")).unwrap();
    let l = library();
    let mut p = capp::plan::plan(&a, Some(&m), &Default::default()).unwrap();
    capp::finish(&mut p, &l, &[]).unwrap();
    check_plan(&p);
    let (jobs, _) = step2cam::job::jobs(&p, &l).unwrap();
    let rough = jobs
        .iter()
        .flat_map(|j| j.manifest["operations"].as_array().unwrap())
        .find(|o| o["kind"] == "roughing")
        .unwrap();
    serde_json::from_value::<camlib::stream::StreamManifest>(jobs[0].manifest.clone())
        .unwrap()
        .validate()
        .unwrap();
    assert_eq!(rough["clearing"]["kind"], "adaptive");
    assert_eq!(rough["clearing"]["path_strategy"], "engagement_front");
    assert!(rough.get("omit_empty_cuts").is_none());
    assert_eq!(rough["stepup_mm"], 10.); // original slotting increment for shallow-floor cleanup
    assert_eq!(rough["stepdown_mm"], 37.); // full flute, below 2D
}

#[test]
fn nonmetallic_iso_n_has_no_nonferrous_metal_source() {
    let mut technology = data();
    roughing::choose(
        &mut op(),
        &mut technology,
        &tool(),
        roughing::Material {
            group: Some("N"),
            name: Some("acetal plastic"),
        },
        &library(),
        "top:roughing",
        &mut Trace::default(),
    )
    .unwrap();
    assert_eq!(technology["clearing_strategy"], "raster");
}

#[test]
fn camlib_refusal_really_replans_raster_and_retains_why() {
    let a: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/plate.analysis.json")).unwrap();
    let m: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/plate.mesh.json")).unwrap();
    let mut l = library();
    // Valid positive policy width, below camlib's native minimum contact angle:
    // camlib must refuse the attempted adaptive operation, rather than a test stub.
    l["roughing_by_group"]["N"]["radial_width_diameter_fraction"] = json!(1e-12);
    let out = step2cam::process(&a, &m, &l).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    check_plan(&out["plan"]);
    let decision = out["plan"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| {
            d["outcome"] == "fallback" && d["rule"] == "process.roughing.bounded_engagement@4"
        })
        .unwrap();
    assert!(
        decision["refusal"]
            .as_str()
            .unwrap()
            .contains("native 1..170 degree")
    );
    let planned = out["plan"]["setups"][0]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["kind"] == "roughing")
        .unwrap();
    assert!(
        planned["because"]
            .as_array()
            .unwrap()
            .contains(&decision["id"])
    );
    assert_eq!(planned["technology"]["clearing_strategy"], "raster");
    let selection = out["plan"]["decisions"].as_array().unwrap().iter()
        .find(|d| d["rule"].as_str().is_some_and(|r| r.starts_with("tool.clearing@"))
            && d["subject"] == "top:roughing" && d["chosen"] == planned["tool"] && d.get("superseded_by").is_none()).unwrap();
    assert!(planned["because"].as_array().unwrap().contains(&selection["id"]));
    assert!(out["log"].as_array().unwrap().iter().any(|n| n.as_str().is_some_and(|s| s.contains("retry raster"))));
    let admitted = out["programs"][0]["job"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["kind"] == "roughing")
        .unwrap();
    assert_eq!(admitted["clearing"]["kind"], "raster");
    for key in ["stepdown_mm", "feed_mm_min", "plunge_mm_min"] {
        assert_eq!(admitted[key], planned["raster_technology"][key]);
    }
    assert_eq!(admitted["omit_empty_cuts"], true);
    assert!(admitted.get("stepup_mm").is_none());
}

#[test]
fn flat_residual_refusal_retries_preparation_without_losing_features() {
    let a: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/synthetic/stepped-plate.analysis.json"
    ))
    .unwrap();
    let m: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/synthetic/stepped-plate.mesh.json"
    ))
    .unwrap();
    let l: Value = serde_json::from_str(include_str!("../../demo/tools.json")).unwrap();
    let out = step2cam::process(&a, &m, &l).unwrap();
    assert!(out["error"].is_null());
    assert_eq!(out["plan"]["workholding"]["chosen"], "vise_flip_face");
    let top = &out["plan"]["setups"][0];
    assert!(
        top["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["kind"] == "flats")
    );
    assert!(
        top["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["kind"] == "v_channel" && o["tool"].is_string())
    );
    assert!(
        out["plan"]["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["outcome"] == "fallback"
                && d["refusal"]
                    .as_str()
                    .is_some_and(|s| s.contains("where roughing did not reach")))
    );
}

#[test]
fn raster_retry_keeps_the_machine_feed_ceiling_below_nominal_feed() {
    let mut l = library();
    l["machines"][0]["controller_settings"]["max_cutting_feed_mm_min"] = json!(750.);
    let (mut op, mut technology, mut trace) = (
        op(),
        data(),
        Trace::default(),
    );
    roughing::choose(
        &mut op,
        &mut technology,
        &tool(),
        material(Some("N")),
        &l,
        "top:roughing",
        &mut trace,
    )
    .unwrap();
    assert_eq!(technology["feed_mm_min"], 750.);
    op.technology = Some(technology);
    let a = serde_json::from_str(include_str!("../../tests/fixtures/plate.analysis.json")).unwrap();
    let m = serde_json::from_str(include_str!("../../tests/fixtures/plate.mesh.json")).unwrap();
    let mut plan = capp::plan::build(&a, Some(&m), &Default::default()).unwrap();
    plan.setups[0].operations = vec![op];
    plan.decisions = Some(trace.decisions);
    roughing::fallbacks(
        &mut plan,
        &[("top".into(), "test adaptive refusal".into())].into(),
    );
    let plan = serde_json::to_value(plan).unwrap();
    assert_eq!(
        plan["setups"][0]["operations"][0]["technology"]["feed_mm_min"],
        750.
    );
    assert_eq!(
        plan["setups"][0]["operations"][0]["technology"]["stepdown_mm"],
        1.
    );
}

#[test]
fn adaptive_stepdown_keeps_the_existing_tool_reach_margin() {
    let mut technology = data();
    let mut short = tool();
    short["stickout"] = json!(12.);
    roughing::choose(&mut op(), &mut technology, &short, material(Some("N")), &library(), "top:roughing", &mut Trace::default()).unwrap();
    assert_eq!(technology["stepdown_mm"], 11.);
}

#[test]
fn empty_adaptive_preparation_uses_checked_raster_instead_of_claiming_zero_work() {
    let a = serde_json::from_str(include_str!("../../tests/fixtures/plate.analysis.json")).unwrap();
    let m = serde_json::from_str(include_str!("../../tests/fixtures/plate.mesh.json")).unwrap();
    let out = step2cam::process(&a, &m, &library()).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    assert!(out["plan"]["decisions"].as_array().unwrap().iter().any(|d| d["outcome"] == "fallback"
        && d["refusal"].as_str().is_some_and(|s| s.contains("no clearing moves"))));
    assert!(out["programs"].as_array().unwrap().iter().flat_map(|p| p["moves"].as_array().unwrap())
        .any(|m| m["op"] == "top-2-roughing" && m["feed"].as_f64().is_some_and(|f| f > 0.)));
    check_plan(&out["plan"]);
}
