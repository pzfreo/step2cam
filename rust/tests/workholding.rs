//! #94: consequences come from each candidate's checked run, and supplied choices replay.
use serde_json::{Value, json};
fn fixture(name: &str) -> Value {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}
fn list(v: &Value) -> &Vec<Value> {
    v.as_array().unwrap()
}
fn number(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn assert_close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn tools() -> Value {
    // The planner's behaviour, pinned on the frozen placeholder library (#88); the real Carvera
    // library's default is checked in carvera.rs.
    fixture("placeholder-tools.json")
}
fn run(name: &str, t: &Value) -> Value {
    let out = step2cam::process(
        &fixture(&format!("{name}.analysis.json")),
        &fixture(&format!("{name}.mesh.json")),
        t,
    )
    .unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    out
}
fn option<'a>(out: &'a Value, id: &str) -> &'a Value {
    out["plan"]["workholding"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap()
}
fn supplied(choice: Value, source: &str) -> Value {
    json!([{"rule":"setup.workholding","subject":"part","choice":choice,"source":source,"reason":"test shop choice"}])
}
#[test]
fn demo_defaults_and_independent_consequences() {
    for name in ["plate", "mounting-plate", "synthetic/stepped-plate"] {
        let out = run(name, &tools());
        assert_eq!(
            out["plan"]["workholding"]["chosen"], "vise_flip_face",
            "{name}"
        );
        assert_eq!(out["plan"]["workholding"]["source"], "rule");
        let ordinary = option(&out, "vise_ordinary");
        let flip = option(&out, "vise_flip_face");
        assert_eq!(ordinary["available"], true);
        assert_eq!(flip["available"], true);
        assert_eq!(ordinary["finished"], false);
        assert_eq!(flip["finished"], true);
        assert_eq!(flip["reason"], "finished");
        assert!(number(&ordinary["outline_left_mm"]) > 4.);
        assert_eq!(flip["outline_left_mm"], 0.);
        assert_eq!(ordinary["extra_stock_mm"], 0.);
        assert!(number(&flip["extra_stock_mm"]) > 5.);
        assert_eq!(
            flip["setups"].as_u64().unwrap(),
            ordinary["setups"].as_u64().unwrap() + 1
        );
        assert_ne!(ordinary["estimated_seconds"], flip["estimated_seconds"]);
        assert!(!list(&ordinary["tools"]).is_empty());
        assert!(!list(&flip["grips"]).is_empty());
        assert_eq!(option(&out, "margins_tabs")["available"], false);
        assert!(
            option(&out, "margins_tabs")["reason"]
                .as_str()
                .unwrap()
                .contains("#87")
        );
        assert_eq!(option(&out, "fixture_plate")["available"], false);
        if name.ends_with("stepped-plate") {
            assert_eq!(ordinary["reason"], "bottom 4.8 mm of outline left attached");
            for candidate in [ordinary, flip] {
                let need = list(&candidate["tools_needed"])
                    .iter()
                    .find(|n| list(&n["operations"]).contains(&json!("front-3-thread")))
                    .expect(
                        "both holding routes must report the missing M3 tapping or milling tool",
                    );
                let description = need["description"].as_str().unwrap();
                assert!(description.starts_with("an M3×0.5 cut tap (⌀2.5 tap drill)"));
                assert!(
                    description.contains(", or a thread mill whose pitch range includes 0.5 mm")
                );
                assert_eq!(need["features"], json!(["holes:48.57", "holes:49.58"]));
                assert!(list(&need["operations"]).contains(&json!("front-3-thread")));
            }
            assert_close(number(&ordinary["outline_left_mm"]), 4.8);
            assert_close(number(&flip["extra_stock_mm"]), 7.25);
            assert_eq!(ordinary["setups"], 2);
            assert_eq!(flip["setups"], 3);
            assert!(
                list(&flip["features_left_out"]).len() < list(&ordinary["features_left_out"]).len()
            );
        }
    }
}
#[test]
fn each_available_choice_is_checked_traced_and_replayable() {
    for (id, source) in [("vise_ordinary", "user"), ("vise_flip_face", "agent:shop")] {
        let mut t = tools();
        t["decisions"] = supplied(json!(id), source);
        let out = run("synthetic/stepped-plate", &t);
        assert_eq!(out["plan"]["workholding"]["chosen"], id);
        assert_eq!(out["plan"]["workholding"]["source"], source);
        assert_eq!(out["plan"]["supplied_decisions"][0]["accepted"], true);
        assert!(
            out["log"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s.as_str().unwrap().contains("per-motion gouge proof"))
        );
        let selected = option(&out, id);
        assert_eq!(
            selected["setups"].as_u64().unwrap() as usize,
            list(&out["plan"]["setups"]).len()
        );
        assert_eq!(
            id == "vise_flip_face",
            out["plan"]["setups"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["flip_seating"].is_object())
        );
        let decision = list(&out["plan"]["decisions"])
            .iter()
            .find(|d| d["rule"] == "setup.workholding@1")
            .unwrap();
        assert_eq!(decision["source"], source);
        assert_eq!(decision["reason"], "test shop choice");
        assert_eq!(selected["tools_needed"], out["plan"]["tools_needed"]);
        let mut replay = tools();
        replay["decisions"] = out["plan"]["supplied_decisions"].clone();
        let again = run("synthetic/stepped-plate", &replay);
        assert_eq!(again["programs"], out["programs"]);
        assert_eq!(again["plan"]["workholding"], out["plan"]["workholding"]);
    }
}
#[test]
fn haas_flip_finishes_despite_missing_spot_tools() {
    // Without its 120° spot drill the Haas library cannot spot the 118° drills.
    let mut library = fixture("../../libraries/haas-vf2.json");
    library["tools"].as_array_mut().unwrap().retain(|t| t["id"] != "spot10-120");
    let out = run("synthetic/stepped-plate", &library);
    assert_eq!(out["plan"]["workholding"]["chosen"], "vise_flip_face");
    let flip = option(&out, "vise_flip_face");
    assert_eq!(flip["finished"], true);
    assert_eq!(flip["reason"], "finished");
    assert_eq!(flip["tools_needed"], out["plan"]["tools_needed"]);
    // The spot drilling no tool cuts is not an operation; its holes are still drilled, so they are
    // neither skipped nor counted as left out, and the setup warns that they are not spotted.
    assert!(list(&out["plan"]["setups"]).iter().all(|s| list(&s["operations"]).iter().all(|o| o["kind"] != "spot_drill")));
    let unspotted: Vec<_> = list(&out["plan"]["decisions"])
        .iter()
        .filter(|d| d["rule"] == "tool.none.unspotted@1" && d["outcome"] == "warning")
        .flat_map(|d| list(&d["features"]))
        .collect();
    assert!(!unspotted.is_empty(), "spot drilling must exercise an operation without a tool");
    for f in &unspotted {
        assert!(list(&out["plan"]["setups"]).iter().any(|setup| list(&setup["operations"])
            .iter()
            .any(|o| o["kind"] == "holes" && o["tool"].is_string() && list(&o["features"]).contains(*f))), "{f}");
        // The front M3 holes are skipped only for their thread, which no Haas tool cuts to the
        // drawing's 8.5 mm full thread.
        let unthreaded = |r: &Value| r.as_str().is_some_and(|r| r.starts_with("thread left out: "));
        assert!(list(&out["plan"]["skipped"]).iter().all(|s| &s["feature"] != *f || unthreaded(&s["reason"])), "{f}");
        assert!(list(&flip["features_left_out"]).iter().all(|l| &l["feature"] != *f || list(&l["reasons"]).iter().all(unthreaded)), "{f}");
    }
    assert!(list(&out["plan"]["warnings"]).iter().any(|w| w.as_str().unwrap().contains(" drilled without spotting: no usable spot drill")));
    assert!(list(&flip["tools_needed"]).iter().any(|n| {
        list(&n["operations"])
            .iter()
            .any(|op| op.as_str().unwrap().ends_with("-spot_drill"))
    }));
}
#[test]
fn unavailable_unknown_and_malformed_choices_are_refused() {
    for choice in [
        json!("margins_tabs"),
        json!("fixture_plate"),
        json!("unknown"),
        json!(23),
    ] {
        let mut t = tools();
        t["decisions"] = supplied(choice, "agent");
        let out = run("synthetic/stepped-plate", &t);
        assert_eq!(out["plan"]["workholding"]["chosen"], "vise_flip_face");
        assert_eq!(out["plan"]["workholding"]["source"], "rule");
        assert_eq!(out["plan"]["supplied_decisions"][0]["accepted"], false);
        assert!(
            list(&out["plan"]["decisions"])
                .iter()
                .any(|d| d["rule"] == "requirement.supplied_refused@1")
        );
    }
}
#[test]
fn vise_options_require_dimensions_and_no_other_mill_is_substituted() {
    for workholding in [json!([]), json!([{"kind":"vise","max_jaw_opening_mm":150}])] {
        let mut t = tools();
        let second = t["machines"][0].clone();
        t["machines"][0]["workholding"] = workholding;
        t["machines"].as_array_mut().unwrap().push(second);
        t["decisions"] = supplied(json!("vise_ordinary"), "user");
        let out = run("plate", &t);
        assert!(out["plan"]["workholding"]["chosen"].is_null());
        assert!(
            list(&out["plan"]["workholding"]["candidates"])
                .iter()
                .all(|c| !c["id"].as_str().unwrap().starts_with("vise_"))
        );
        assert_eq!(out["plan"]["supplied_decisions"][0]["accepted"], false);
        assert!(
            out["plan"]["supplied_decisions"][0]["refusal_reason"]
                .as_str()
                .unwrap()
                .contains("no dimensioned vise")
        );
    }
}
#[test]
fn legacy_override_is_a_user_choice_and_explicit_decisions_take_precedence() {
    for (legacy, id) in [
        ("none", "vise_ordinary"),
        ("vise_flip_face", "vise_flip_face"),
    ] {
        let mut t = tools();
        t["overrides"]["workholding_option"] = json!(legacy);
        let out = run("synthetic/stepped-plate", &t);
        assert_eq!(out["plan"]["workholding"]["chosen"], id);
        assert_eq!(out["plan"]["workholding"]["source"], "user");
        assert!(
            out["plan"]["supplied_decisions"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("override")
        );
        t["decisions"] = supplied(json!("vise_ordinary"), "agent");
        assert_eq!(
            run("synthetic/stepped-plate", &t)["plan"]["workholding"]["chosen"],
            "vise_ordinary"
        );
    }
}
#[test]
fn choosing_rule_is_lexicographic_in_the_approved_order() {
    let best = json!({"finished":true,"features_left_out":[],"setups":2,"extra_stock_mm":4.,"estimated_seconds":100.});
    for (field, value) in [
        ("finished", json!(false)),
        ("features_left_out", json!([{"feature":"hole"}])),
        ("setups", json!(3)),
        ("extra_stock_mm", json!(5.)),
        ("estimated_seconds", json!(101.)),
    ] {
        let mut worse = best.clone();
        worse[field] = value;
        assert!(capp::candidates::compare(&best, &worse).is_lt(), "{field}");
    }
    let mut slower = best.clone();
    slower["estimated_seconds"] = json!(1000.);
    let mut unfinished = best.clone();
    unfinished["finished"] = json!(false);
    unfinished["estimated_seconds"] = json!(1.);
    assert!(capp::candidates::compare(&slower, &unfinished).is_lt());
}

#[test]
fn tall_part_exposes_attached_outline_and_bare_flip_unavailability() {
    let out = run("synthetic/tall-wall-pocket", &tools());
    assert_eq!(out["plan"]["workholding"]["chosen"], "vise_ordinary");
    assert_eq!(option(&out, "vise_ordinary")["finished"], false);
    assert_close(
        number(&option(&out, "vise_ordinary")["outline_left_mm"]),
        32.,
    );
    let flip = option(&out, "vise_flip_face");
    assert_eq!(flip["available"], false);
    let reason = flip["reason"].as_str().unwrap();
    assert!(!reason.contains("selected"));
    assert!(!reason.contains("option is unavailable"));
    assert_eq!(
        option(&out, "fixture_plate")["short_reason"],
        "no suitable through holes"
    );
    assert!(
        option(&out, "fixture_plate")["reason"]
            .as_str()
            .unwrap()
            .starts_with("no suitable through holes")
    );
}
