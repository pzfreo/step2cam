//! #86's resource-aware option, separate from the machine-independent golden plans.
use std::{fs, path::PathBuf};
fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
fn list(value: &Value) -> &Vec<Value> {
    value.as_array().unwrap()
}
use serde_json::{Value, json};

fn tools() -> Value {
    let mut t = fixture("../../tests/fixtures/placeholder-tools.json");
    t["overrides"]["workholding_option"] = json!("vise_flip_face");
    t
}
fn input() -> (Value, Value) {
    (
        fixture("synthetic/stepped-plate.analysis.json"),
        fixture("synthetic/stepped-plate.mesh.json"),
    )
}
fn planned() -> Value {
    let (a, m) = input();
    capp::flip::plan(&a, &m, &tools()).unwrap()
}
fn setup<'a>(p: &'a Value, id: &str) -> &'a Value {
    list(&p["setups"]).iter().find(|s| s["id"] == id).unwrap()
}
fn audit<'a>(p: &'a Value, s: &Value) -> &'a Value {
    let d = list(&p["decisions"])
        .iter()
        .find(|d| d["id"] == s["clamping"]["because"][0])
        .unwrap();
    &d["chosen"]["prior_setups"]
}

#[test]
fn complex_plate_flips_faces_and_checks_real_grip_bands() {
    let (a, m) = input();
    let out = step2cam::process(&a, &m, &tools()).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let p = &out["plan"];
    assert_eq!(
        list(&p["setups"])
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["top", "bottom", "front"]
    );
    assert_eq!(list(&p["skipped"]).len(), 6, "{}", p["skipped"]);
    assert!(
        list(&p["skipped"])
            .iter()
            .all(|s| s["reason"].as_str().unwrap().contains("r0.8 edge round"))
    );
    let top = setup(p, "top");
    assert_eq!(top["stock"]["min"][2], -26.25);
    assert_eq!(top["clamping"]["grip_height_mm"], 5.25);
    assert_eq!(top["clamping"]["jaw_top_z"], -21.);
    let rough = list(&top["operations"])
        .iter()
        .find(|o| o["kind"] == "roughing")
        .unwrap();
    assert_eq!(rough["floor_z"], -19.);
    assert_eq!(rough["web_mm"], 0.);
    let bottom = setup(p, "bottom");
    // The receiving envelope includes Op 1's residual stock margins, with
    // camlib's conservative numerical enclosure, rather than a fresh part box.
    assert_eq!(bottom["stock"]["min"][0], -2.);
    assert_eq!(bottom["stock"]["min"][1], -2.);
    assert_eq!(bottom["stock"]["max"], json!([102.,58.,7.25]));
    let low = bottom["stock"]["min"][2].as_f64().unwrap();
    assert!(low <= -18. && low > -18.001);
    let job = list(&out["programs"]).iter().find(|j|j["setup"]=="bottom").unwrap();
    assert!(job["job"]["stock"]["mesh"].is_object());
    assert_eq!(bottom["stock"]["min"], job["job"]["stock"]["min_mm"]);
    assert_eq!(bottom["stock"]["max"], job["job"]["stock"]["max_mm"]);
    assert_eq!(
        list(&bottom["operations"]).len(),
        1,
        "the whole outline was finished in Op 1"
    );
    assert_eq!(bottom["operations"][0]["kind"], "facing");
    assert_eq!(bottom["operations"][0]["floor_z"], 0.);
    assert_eq!(bottom["flip_seating"]["seat_z_model"], 12.);
    assert_eq!(
        bottom["clamping"]["faces"],
        json!(["stock.x.min", "stock.x.max"])
    );
    assert_eq!(
        bottom["clamping"]["contact_min_z"], -12.,
        "seat under plate top, not boss top at -18"
    );
    assert_eq!(bottom["clamping"]["jaw_top_z"], -8.825);
    assert_eq!(bottom["clamping"]["parallels"][0]["max"][2], -12.);
    let bodies = list(&job["job"]["verification"]["scene"]["scene"]["bodies"]);
    assert_eq!(bodies.iter().filter(|b| b["id"].as_str().unwrap().contains("parallel") && b["role"]=="fixture").count(), 2);
    assert_eq!(
        audit(p, bottom)["status"],
        "no_overlap",
        "{}",
        p["warnings"]
    );
    assert!(p["decisions"].to_string().contains("bottom_stock_mm"));
    assert!(p["warnings"].to_string().contains("residual margin"));
    let notes = out["log"].to_string();
    for forbidden in [
        "web",
        "skin or burr",
        "support the part clear of the holes",
        "left out",
    ] {
        assert!(!notes.contains(forbidden), "{forbidden}: {notes}");
    }
    assert_eq!(list(&out["programs"]).len(), 3);
    for program in list(&out["programs"]) {
        if program["setup"] != "front" {
            assert_eq!(
                program["material"]["gouge"]["definite_cells"], 0,
                "{}",
                program["material"]
            );
            assert!(!list(&program["job"]["verification"]["material"]).is_empty());
        }
    }
    let validator =
        jsonschema::validator_for(&fixture("../../schemas/plan-1.schema.json")).unwrap();
    assert!(
        validator.is_valid(p),
        "{:?}",
        validator
            .iter_errors(p)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn grip_audit_still_reports_a_hole_crossing_a_prepared_band() {
    let mut p = planned();
    p["setups"][0]["operations"].as_array_mut().unwrap().push(json!({"kind":"holes","diameter":4.4,"top_z":-7.,"bore_top_z":-7.,"bottom_z":-19.,"bottom":"through","centers":[[2.,30.]],"features":["test:crossing"],"countersinks":[null]}));
    capp::finish(&mut p, &tools(), &[]).unwrap();
    let bottom = setup(&p, "bottom");
    assert_eq!(audit(&p, bottom)["status"], "possible_overlap");
    assert!(
        list(&bottom["operations"]).is_empty(),
        "a disputed grip must not be machined"
    );
    assert!(audit(&p, bottom)["overlaps"].to_string().contains("holes"));
}

#[test]
fn excluded_finishing_requires_replanning_or_explicit_sacrificial_stock_warning() {
    let mut p = planned();
    capp::finish(&mut p, &tools(), &[]).unwrap();
    let index = list(&setup(&p, "top")["operations"])
        .iter()
        .position(|o| o["kind"] == "wall_finish")
        .unwrap();
    let operation = format!("top-{}-wall_finish", index + 1);
    let mut p = planned();
    capp::finish(
        &mut p,
        &tools(),
        &[json!({"operation":operation,"message":"test refusal"})],
    )
    .unwrap();
    assert_eq!(audit(&p, setup(&p, "bottom"))["status"], "possible_overlap");
    assert!(list(&setup(&p, "bottom")["operations"]).is_empty());
    assert!(
        p["warnings"]
            .to_string()
            .contains("part remains on 7.25 mm of sacrificial bottom stock")
    );
    let mut fallback = capp::flip::fallback(
        &input().0,
        &input().1,
        "test refusal of finished grip preparation",
    )
    .unwrap();
    capp::finish(&mut fallback, &tools(), &[]).unwrap();
    assert_eq!(setup(&fallback, "top")["stock"]["min"][2], -19.);
    assert!(
        list(&fallback["setups"])
            .iter()
            .all(|s| s.get("flip_seating").is_none())
    );
}

#[test]
fn supplied_parallels_validate_both_setups_or_withhold_the_option() {
    let (a, m) = input();
    let mut t = tools();
    t["machines"][0]["workholding"][0]["parallels_height_mm"] = json!(14.);
    let mut p = capp::flip::plan(&a, &m, &t).unwrap();
    assert_eq!(setup(&p, "top")["stock"]["min"][2], -27.);
    capp::finish(&mut p, &t, &[]).unwrap();
    assert_eq!(setup(&p, "top")["clamping"]["grip_height_mm"], 6.);
    assert_eq!(setup(&p, "bottom")["clamping"]["grip_height_mm"], 6.);
    assert_eq!(
        setup(&p, "bottom")["clamping"]["parallels"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    t["machines"][0]["workholding"][0]["parallels_height_mm"] = json!(19.);
    let p = capp::flip::plan(&a, &m, &t).unwrap();
    assert!(setup(&p, "top").get("flip_role").is_none());
    assert!(
        list(&p["decisions"])
            .iter()
            .any(|d| d["outcome"] == "none" && d["says"].as_str().unwrap().contains("minimum"))
    );
}

#[test]
fn mounting_plate_edge_breaks_get_inboard_supports_and_full_outline_depth() {
    let a = fixture("mounting-plate.analysis.json");
    let m = fixture("mounting-plate.mesh.json");
    let out = step2cam::process(&a, &m, &tools()).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let p = &out["plan"];
    let bottom = setup(p, "bottom");
    assert!(
        bottom["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["kind"] == "facing")
    );
    assert!(
        bottom["flip_seating"]["supports"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["offset_mm"].as_f64().unwrap() > 0.)
    );
    assert!(bottom["flip_seating"]["contacts"].as_array().unwrap().iter().all(|c|c["fraction"].as_f64().unwrap()>=0.6 && c["fraction"].as_f64().unwrap()<1.));
    for id in ["fillets:1", "fillets:4", "fillets:15", "fillets:37"] {
        assert!(
            !list(&p["skipped"]).iter().any(|s| s["feature"] == id),
            "{id}: {}",
            p["skipped"]
        );
        assert!(list(&p["covered_by_roughing"]).contains(&json!(id)));
    }
    for program in list(&out["programs"]) {
        assert_eq!(program["material"]["gouge"]["definite_cells"], 0);
    }
    let text = p["warnings"].to_string();
    assert!(!text.contains("boss"));
    assert!(!text.contains("material below the seat"));
}

#[test]
fn modeled_side_pockets_reduce_contact_and_preserve_a_usable_grip() {
    let (a, m) = input();
    let mut t = tools();
    // The finished short ends are 100 mm apart, so this vise forces the
    // opposed long sides, which both have the modelled open side pockets.
    t["machines"][0]["workholding"][0]["max_jaw_opening_mm"] = json!(80.);
    let out = step2cam::process(&a, &m, &t).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let p = &out["plan"];
    let bottom = setup(p, "bottom");
    assert_eq!(bottom["flip_seating"]["axis"], 1);
    for c in list(&bottom["clamping"]["contacts"]) {
        let f = c["fraction"].as_f64().unwrap();
        assert!((0.6..1.).contains(&f), "{c}");
    }
    assert_eq!(
        audit(p, bottom)["status"],
        "no_overlap",
        "{}",
        p["warnings"]
    );
    assert!(
        list(&bottom["operations"])
            .iter()
            .any(|o| o["kind"] == "facing")
    );
    assert!(p["decisions"].to_string().contains("contact_measurements"));
    for program in list(&out["programs"])
        .iter()
        .filter(|p| p["setup"] != "front")
    {
        assert_eq!(program["material"]["gouge"]["definite_cells"], 0);
    }
}

#[test]
fn default_and_none_trace_availability_without_changing_the_machine_independent_plan() {
    let (a, m) = input();
    let original = capp::plan::plan(&a, Some(&m), &Default::default()).unwrap();
    for choice in [None, Some("none")] {
        let mut t = fixture("../../tests/fixtures/placeholder-tools.json");
        if let Some(choice) = choice {
            t["overrides"]["workholding_option"] = json!(choice);
        }
        let mut p = capp::flip::plan(&a, &m, &t).unwrap();
        let d = p["decisions"].as_array_mut().unwrap().pop().unwrap();
        assert_eq!(d["chosen"]["selected"], false);
        assert_eq!(d["chosen"]["available"], true);
        assert_eq!(d["chosen"]["requested"], json!(choice));
        assert!(
            d["says"]
                .as_str()
                .unwrap()
                .contains("available but was not selected")
        );
        assert_eq!(p, original, "only the availability decision may differ");
    }
    let p = capp::flip::plan(&a, &m, &tools()).unwrap();
    let d = list(&p["decisions"])
        .iter()
        .find(|d| {
            d["rule"].as_str().unwrap().starts_with("setup.flip_face") && d["subject"] == "part"
        })
        .unwrap();
    assert_eq!(d["chosen"]["selected"], true);
    assert_eq!(d["chosen"]["available"], true);
    assert_eq!(setup(&p, "bottom")["operations"][0]["kind"], "facing");
}

#[test]
fn unavailable_selected_options_and_invalid_choices_are_explicit() {
    let (a, m) = input();
    let mut no_vise = tools();
    no_vise.as_object_mut().unwrap().remove("machines");
    let mut clamping = tools();
    clamping["overrides"]["clamping"] = json!({"top":{"axis":"y","grip_height_mm":3.8}});
    let mut ineligible = a.clone();
    ineligible["features"] = json!([]);
    let mut missing = m.clone();
    missing["missing"] = json!([0]);
    for (a, m, t, reason) in [
        (&a, &m, &no_vise, "dimensioned vise"),
        (&a, &m, &clamping, "clamping proposals"),
        (
            &ineligible,
            &missing,
            &tools(),
            "no planned web or through features",
        ),
    ] {
        let p = capp::flip::plan(a, m, t).unwrap();
        let d = list(&p["decisions"]).last().unwrap();
        assert_eq!(d["chosen"]["selected"], true);
        assert_eq!(d["chosen"]["available"], false);
        assert!(d["says"].as_str().unwrap().contains(reason), "{d}");
        assert!(
            !list(&p["setups"])
                .iter()
                .any(|s| s.get("flip_seating").is_some())
        );
    }
    let validator =
        jsonschema::validator_for(&fixture("../../schemas/resources-1.schema.json")).unwrap();
    for choice in [
        json!("typo"),
        json!(""),
        json!(null),
        json!(false),
        json!(1),
    ] {
        let mut t = tools();
        t["overrides"]["workholding_option"] = choice;
        let err = capp::resources::normalize(&t).unwrap_err();
        assert!(err.contains("overrides.workholding_option"), "{err}");
        assert!(
            err.contains("none") && err.contains("vise_flip_face"),
            "{err}"
        );
        assert!(
            step2cam::process(&a, &m, &t)
                .unwrap_err()
                .contains("overrides.workholding_option")
        );
        assert!(!validator.is_valid(&t));
    }
    for choice in ["none", "vise_flip_face"] {
        let mut t = tools();
        t["overrides"]["workholding_option"] = json!(choice);
        assert!(capp::resources::normalize(&t).is_ok());
        assert!(validator.is_valid(&t));
    }
}

#[test]
fn flip_seating_parallels_must_be_achievable_from_inventory() {
    let (a, m) = input();
    let mut t = tools();
    t["machines"][0]["workholding"][0]["parallels_height_mm"] = json!(14.);
    t["machines"][0]["workholding"][0]["parallels_heights_mm"] = json!([14.,15.875]);
    let mut p = capp::flip::plan(&a, &m, &t).unwrap();
    capp::finish(&mut p, &t, &[]).unwrap();
    let bottom = setup(&p, "bottom");
    assert_eq!(list(&bottom["clamping"]["parallels"]).len(), 2);
    assert_eq!(bottom["clamping"]["parallels"][0]["max"][2], -12.);
    assert!(list(&bottom["operations"]).iter().any(|o| o["kind"]=="facing"));

    // The fixed height alone must not bypass an explicit inventory restriction.
    t["machines"][0]["workholding"][0]["parallels_heights_mm"] = json!([15.875]);
    let mut p = capp::flip::plan(&a, &m, &t).unwrap();
    capp::finish(&mut p, &t, &[]).unwrap();
    assert!(list(&setup(&p, "bottom")["operations"]).is_empty());
    assert!(p["warnings"].to_string().contains("mesh seating requires 14.0 mm parallels, absent from available heights"));
}
