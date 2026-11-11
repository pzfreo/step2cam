//! Policy, adapter and raster retry contracts, independent of absolute output hashes.
use capp::{roughing, trace::Trace};
use serde_json::{Value, json};

fn library() -> Value {
    serde_json::from_str(include_str!("../../libraries/haas-vf2.json")).unwrap()
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
fn data() -> Value {
    json!({"feed_mm_min":1000.,"spindle_rpm":5000.,"stepdown_mm":1.,"stepover_mm":2.,"plunge_mm_min":100.,"source":"tool"})
}
fn tool() -> Value {
    json!({"diameter":10.,"flute_length":30.})
}

#[test]
fn sourced_groups_use_tool_dimensions_and_chip_thinning_with_feed_cap() {
    let mut library = library();
    library["machines"][0]["controller_settings"]["max_cutting_feed_mm_min"] = json!(1500.);
    for (group, width, factor) in [("N", 2.5, 1. / 0.75_f64.sqrt()), ("P", 1., 5. / 3.)] {
        let (mut op, mut technology, mut trace) = (json!({}), data(), Trace::default());
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
        assert_eq!(op["raster_technology"], data());
        assert!(trace.decisions[0]["policy"]["_sources"].is_object());
    }
    library["machines"][0]["controller_settings"]
        .as_object_mut()
        .unwrap()
        .remove("max_cutting_feed_mm_min");
    library["rapid_mm_min"] = json!(1100.);
    let mut technology = data();
    roughing::choose(
        &mut json!({}),
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
        let (mut op, mut technology, mut trace) = (json!({}), data(), Trace::default());
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
        (json!({"kind":"roughing"}), data(), Trace::default());
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
    op["technology"] = technology;
    op["because"] = json!([decision]);
    let mut plan = json!({"setups":[{"id":"top","operations":[op]}]});
    trace.store(&mut plan);
    assert_eq!(
        roughing::adaptive_setup(&plan, "top-1-roughing"),
        Some("top".into())
    );
    assert_eq!(roughing::adaptive_setup(&plan, "top-2-flats"), None);
    roughing::fallbacks(
        &mut plan,
        &[("top".into(), "deep layer shank clearance refused".into())].into(),
    );
    assert_eq!(roughing::adaptive_setup(&plan, "top-1-roughing"), None);
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
    assert_eq!(rough["stepdown_mm"], 38.); // full flute, below 2D
}

#[test]
fn nonmetallic_iso_n_has_no_nonferrous_metal_source() {
    let mut technology = data();
    roughing::choose(
        &mut json!({}),
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
