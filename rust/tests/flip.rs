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
    fixture("../../demo/tools.json")
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
    assert_eq!(
        bottom["stock"],
        json!({"min":[0.,0.,-18.],"max":[100.,56.,7.25]})
    );
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
    p["setups"][0]["operations"].as_array_mut().unwrap().push(json!({"kind":"holes","diameter":4.4,"top_z":-7.,"bottom_z":-19.,"bottom":"through","centers":[[2.,30.]],"features":["test:crossing"],"countersinks":[null]}));
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
