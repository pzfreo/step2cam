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
            assert_eq!(ordinary["reason"], "bottom 4.125 mm of outline left attached");
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
            assert_close(number(&ordinary["outline_left_mm"]), 4.125);
            assert_close(number(&flip["extra_stock_mm"]), 10.);
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

#[test]
fn tall_linuxcnc_flip_uses_the_deepest_prepared_seat() {
    let mut t = fixture("../../libraries/linuxcnc-vmc.json");
    t["overrides"]["workholding_option"] = json!("vise_flip_face");
    let a = fixture("synthetic/tall-wall-pocket.analysis.json");
    let m = fixture("synthetic/tall-wall-pocket.mesh.json");
    let mut trial = capp::flip::build(&a, &m, &t).unwrap();
    let mut rejected = vec![];
    let expected = loop {
        let mut finished = trial.clone();
        capp::finish_plan(&mut finished, &t, &[]).unwrap();
        let bottom = finished.setups.iter().find(|s| s.flip_seating.is_some()).unwrap();
        let id = &bottom.clamping.as_ref().unwrap()["because"][0];
        let evidence = &finished.decisions.as_ref().unwrap().iter().find(|d| d["id"] == *id).unwrap()["chosen"]["prior_setups"];
        if evidence["prepared_contacts"] == true && evidence["established_seat"] == true && evidence["status"] == "no_overlap" {
            break bottom.flip_seating.as_ref().unwrap()["grip_height_mm"].clone();
        }
        rejected.push(bottom.flip_seating.clone().unwrap());
        assert!(capp::flip::retry_seating(&mut trial, &m, &t, &rejected).unwrap(), "a prepared seat must exist");
    };
    let out = run("synthetic/tall-wall-pocket", &t);
    assert_eq!(option(&out, "vise_flip_face")["finished"], true);
    assert_eq!(out["plan"]["workholding"]["chosen"], "vise_flip_face");
    let bottom = list(&out["plan"]["setups"]).iter().find(|s| s["id"] == "bottom").unwrap();
    assert_eq!(bottom["clamping"]["grip_height_mm"], expected);
    assert!(list(&bottom["operations"]).iter().any(|o| o["kind"] == "facing" && o["tool"].is_string()));
}

#[test]
fn preparation_failure_retries_shallower_seats_without_restoring_operations() {
    let mut t = fixture("../../libraries/linuxcnc-vmc.json");
    t["overrides"]["workholding_option"] = json!("vise_flip_face");
    let a = fixture("synthetic/tall-wall-pocket.analysis.json");
    let m = fixture("synthetic/tall-wall-pocket.mesh.json");
    let mut p = capp::flip::build(&a, &m, &t).unwrap();
    let top = p.setups.iter_mut().find(|s| s.id == "top").unwrap();
    // A retained finish ends between the minimum and deepest contact bands.
    let prepared_from_model_z = 5.6;
    let floor = prepared_from_model_z-top.origin_model.unwrap()[2];
    for op in &mut top.operations {
        if let capp::model::Cut::WallFinish(cut) = &mut op.cut { cut.floor_z = cut.floor_z.max(floor); }
    }
    let original = top.operations.clone();
    let mut rejected = vec![];
    let grip = loop {
        let mut finished = p.clone();
        capp::finish_plan(&mut finished, &t, &[]).unwrap();
        let bottom = finished.setups.iter().find(|s| s.flip_seating.is_some()).unwrap();
        let seat = bottom.flip_seating.as_ref().unwrap();
        if !bottom.operations.is_empty() { break number(&seat["grip_height_mm"]); }
        rejected.push(seat.clone());
        assert!(capp::flip::retry_seating(&mut p, &m, &t, &rejected).unwrap());
        assert_eq!(p.setups.iter().find(|s| s.id == "top").unwrap().operations, original);
    };
    assert!(!rejected.is_empty(), "the deepest candidate must fail preparation");
    let rules = capp::rules::workholding();
    let minimum = number(&rules["min_grip_mm"]);
    let step = number(&rules["flip_support_search_step_mm"]);
    let maximum = 10.*number(&rules["max_grip_fraction"]);
    let expected = (0..=((maximum-minimum)/step).floor() as usize)
        .map(|n| minimum+n as f64*step).filter(|g| *g <= 10.-prepared_from_model_z).max_by(f64::total_cmp).unwrap();
    assert_close(grip, expected);
}

#[test]
fn process_retries_when_only_the_upper_side_band_is_prepared() {
    let mut a = fixture("synthetic/tall-wall-pocket.analysis.json");
    let mut m = fixture("synthetic/tall-wall-pocket.mesh.json");
    let mut t = fixture("../../libraries/linuxcnc-vmc.json");
    t["overrides"]["workholding_option"] = json!("vise_flip_face");
    // Synthetic near-vertical lower band: its analytic normal is just outside
    // the upright-contour tolerance, while its mesh remains within mesh error.
    // The upper band is upright and can be prepared for the receiving jaws.
    let split = 5.6;
    let positions: Vec<[f64; 3]> = list(&m["positions"]).chunks(3).map(|p| [number(&p[0]),number(&p[1]),number(&p[2])]).collect();
    let indices = list(&m["indices"]).clone();
    let faces = list(&a["faces"]).clone();
    let mut triangles: Vec<[[f64; 3]; 3]> = vec![];
    let mut upper_ranges = vec![];
    let mut lower_ranges = vec![];
    for range in list(&m["faces"]) {
        for upper in [true, false] {
            let start = triangles.len();
            for i in range[0].as_u64().unwrap() as usize..(range[0].as_u64().unwrap()+range[1].as_u64().unwrap()) as usize {
                let poly: Vec<_> = indices[3*i..3*i+3].iter().map(|n| positions[n.as_u64().unwrap() as usize]).collect();
                let mut clipped = vec![];
                for j in 0..poly.len() {
                    let (p,q) = (poly[j],poly[(j+1)%poly.len()]);
                    let inside = |v: [f64; 3]| if upper { v[2] >= split } else { v[2] <= split };
                    if inside(p) { clipped.push(p); }
                    if inside(p) != inside(q) {
                        let f = (split-p[2])/(q[2]-p[2]);
                        clipped.push([p[0]+f*(q[0]-p[0]),p[1]+f*(q[1]-p[1]),split]);
                    }
                }
                for j in 1..clipped.len().saturating_sub(1) { triangles.push([clipped[0],clipped[j],clipped[j+1]]); }
            }
            let r = json!([start,triangles.len()-start]);
            if upper { upper_ranges.push(r); } else { lower_ranges.push(r); }
        }
    }
    upper_ranges.extend(lower_ranges);
    let mut new_faces = faces.clone();
    for face in &faces {
        let mut lower = face.clone();
        lower["id"] = json!(new_faces.len());
        if number(&lower["direction"][2]).abs() < 1e-6 {
            lower["direction"][2] = json!(2e-6);
        }
        new_faces.push(lower);
    }
    // A recognised level divides the bands; its empty range adds no support area.
    new_faces.push(json!({"id":new_faces.len(),"kind":"plane","direction":[0.,0.,1.],"centroid":[0.,0.,split],"area":0.}));
    upper_ranges.push(json!([triangles.len(),0]));
    a["faces"] = json!(new_faces);
    m["faces"] = json!(upper_ranges);
    m["positions"] = json!(triangles.iter().flatten().flatten().copied().collect::<Vec<_>>());
    m["indices"] = json!((0..triangles.len()*3).collect::<Vec<_>>());
    m["edges"] = json!([]);
    let initial = capp::flip::plan(&a, &m, &t).unwrap();
    let deepest = list(&initial["setups"]).iter().find(|s| s.get("flip_seating").is_some())
        .unwrap_or_else(|| panic!("{}", initial["decisions"]))["flip_seating"]["grip_height_mm"].clone();
    let mut first = initial.clone();
    capp::finish(&mut first, &t, &[]).unwrap();
    let first_bottom = list(&first["setups"]).iter().find(|s| s.get("flip_seating").is_some()).unwrap();
    let evidence = &list(&first["decisions"]).iter().find(|d| d["id"] == first_bottom["clamping"]["because"][0]).unwrap()["chosen"]["prior_setups"];
    assert_eq!(evidence["prepared_contacts"], false);
    assert_eq!(evidence["established_seat"], true);
    let out = step2cam::process(&a, &m, &t).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let bottom = list(&out["plan"]["setups"]).iter().find(|s| s.get("flip_seating").is_some()).expect("retry retains the flip");
    let evidence = &list(&out["plan"]["decisions"]).iter().find(|d| d["id"] == bottom["clamping"]["because"][0]).unwrap()["chosen"]["prior_setups"];
    assert_eq!(evidence["prepared_contacts"], true);
    let grip = number(&bottom["clamping"]["grip_height_mm"]);
    assert!(grip < number(&deepest));
    let seat = number(&bottom["flip_seating"]["seat_z_model"]);
    let rule = capp::rules::workholding();
    let min = number(&rule["min_grip_mm"]);
    let step = number(&rule["flip_support_search_step_mm"]);
    let max = seat*number(&rule["max_grip_fraction"]);
    let expected = (0..=((max-min)/step).floor() as usize).map(|n| min+n as f64*step)
        .filter(|g| seat-g >= split).max_by(f64::total_cmp).unwrap();
    assert_close(grip, expected);
    assert!(list(&out["plan"]["decisions"]).iter().any(|d| d["chosen"]["seating_attempt"].as_u64().is_some_and(|n| n > 1)));
    assert!(list(&bottom["operations"]).iter().any(|o| o["kind"] == "facing" && o["tool"].is_string()));
}
