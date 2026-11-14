//! Roughing is useful above the selected tool's reach even when the stock is deeper.
use serde_json::{Value, json};

fn read(path: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}
fn tools(flutes: f64, stickout: f64) -> Value {
    let mut library = read("tests/fixtures/placeholder-tools.json");
    library.as_object_mut().unwrap().remove("machines");
    library["roughing_tool"] = json!("em12");
    let mill = library["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == "em12").unwrap();
    mill["flute_length"] = json!(flutes);
    mill["stickout"] = json!(stickout);
    library
}
fn planned(rough_from: &str) -> Value {
    capp::plan::plan(&read("tests/fixtures/synthetic/stepped-plate.analysis.json"),
        Some(&read("tests/fixtures/synthetic/stepped-plate.mesh.json")),
        &capp::plan::Options { rough_from: rough_from.into(), ..Default::default() }).unwrap()
}
fn ops(plan: &Value) -> &[Value] { plan["setups"][0]["operations"].as_array().unwrap() }

#[test]
fn short_flutes_keep_shallow_coverage_and_limit_crossing_wall_bands() {
    let mut plan = planned("top");
    let original = plan.clone();
    let library = tools(12., 80.);
    capp::finish(&mut plan, &library, &[]).unwrap();
    let rough = ops(&plan).iter().find(|o| o["kind"] == "roughing").unwrap();
    assert_eq!(rough["floor_z"], -11.);
    assert_eq!(rough["web_mm"], 9.);
    let depths = original["roughing_feature_depths"].as_object().unwrap();
    let mut kept = 0;
    let mut dropped = 0;
    for id in original["covered_by_roughing"].as_array().unwrap() {
        let depth = &depths[id.as_str().unwrap()];
        if depth["floor_z"].as_f64().unwrap() >= -11. {
            kept += 1;
            assert!(plan["covered_by_roughing"].as_array().unwrap().contains(id));
        } else {
            dropped += 1;
            assert!(!plan["covered_by_roughing"].as_array().unwrap().contains(id));
            let skip = plan["skipped"].as_array().unwrap().iter().find(|s| s["feature"] == *id).unwrap();
            let reason = skip["reason"].as_str().unwrap();
            assert!(reason.contains("roughing reaches only Z-11.0 (11.0 mm below the stock top, em12's 12.0 mm flutes"), "{reason}");
            assert!(reason.contains(": needs an end mill with flutes of at least "), "{reason}");
            assert!(plan["tools_needed"].as_array().unwrap().iter().any(|n| n["features"].as_array().unwrap().contains(id)));
            assert!(plan["decisions"].as_array().unwrap().iter().any(|d| d["subject"] == *id && d["outcome"] == "covered" && d["superseded_by"] == skip["decision"]));
        }
    }
    assert!(kept > 0 && dropped > 0, "kept {kept}, dropped {dropped}");
    let walls: Vec<_> = ops(&plan).iter().filter(|o| o["kind"] == "wall_finish").collect();
    assert_eq!(walls.len(), 2);
    assert_eq!(walls[0]["floor_z"], -11.);
    assert_eq!(walls[1]["floor_z"], -8.);
    assert_eq!(ops(&plan).iter().find(|o| o["kind"] == "flats").unwrap()["levels"], json!([-8.]));
    let settled = plan.clone();
    capp::settle(&mut plan, &library).unwrap();
    assert_eq!(plan, settled, "settlement is idempotent");
}

#[test]
fn stickout_limits_reach_and_flats_below_it_are_removed_before_dependants() {
    let mut plan = planned("top");
    capp::finish(&mut plan, &tools(75., 6.), &[]).unwrap();
    assert_eq!(ops(&plan).iter().find(|o| o["kind"] == "roughing").unwrap()["floor_z"], -5.);
    assert!(!ops(&plan).iter().any(|o| o["kind"] == "flats" || (o["kind"] == "v_channel" && o["on_flat"] == true)));
    let walls: Vec<_> = ops(&plan).iter().filter(|o| o["kind"] == "wall_finish").collect();
    assert_eq!(walls.len(), 1, "only the upper band has any wall above the cap");
    assert_eq!(walls[0]["floor_z"], -5.);
    assert_eq!(walls[0]["on_flat"], false, "the clipped wall no longer needs the unreachable flat");
    assert!(plan["warnings"].to_string().contains("6.0 mm stickout"));
}

#[test]
fn capped_roughing_runs_through_camlib_without_gouging() {
    let out = step2cam::process(&read("tests/fixtures/synthetic/stepped-plate.analysis.json"),
        &read("tests/fixtures/synthetic/stepped-plate.mesh.json"), &tools(12., 80.)).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    assert!(ops(&out["plan"]).iter().any(|o| o["kind"] == "roughing" && o["floor_z"] == -11.));
    assert!(!out["programs"][0]["material"].is_null(), "roughing must retain its material check");
    for program in out["programs"].as_array().unwrap().iter().filter(|p| !p["material"].is_null()) {
        assert_eq!(program["material"]["gouge"]["definite_cells"], 0);
    }
    let validator = jsonschema::validator_for(&read("schemas/plan-1.schema.json")).unwrap();
    assert!(validator.is_valid(&out["plan"]));
}

#[test]
fn without_jaw_data_normal_library_does_not_change_fixture_floors_or_coverage() {
    for name in ["plate", "mounting-plate", "synthetic/stepped-plate"] {
        let mut plan = capp::plan::plan(&read(&format!("tests/fixtures/{name}.analysis.json")),
            Some(&read(&format!("tests/fixtures/{name}.mesh.json"))), &Default::default()).unwrap();
        let before = plan.clone();
        // Isolate the reach rule: clamping intentionally caps these floors when jaw data exists.
        let mut library = read("tests/fixtures/placeholder-tools.json");
        library["machines"][0].as_object_mut().unwrap().remove("workholding");
        capp::finish(&mut plan, &library, &[]).unwrap();
        assert_eq!(plan["covered_by_roughing"], before["covered_by_roughing"]);
        assert!(!plan["decisions"].as_array().unwrap().iter().any(|d| d["rule"] == "process.roughing.reach@1"));
        for setup in plan["setups"].as_array().unwrap() {
            for op in setup["operations"].as_array().unwrap().iter().filter(|o| o["kind"] == "roughing") {
                let prior = before["setups"].as_array().unwrap().iter().find(|s| s["id"] == setup["id"]).unwrap()["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "roughing").unwrap();
                assert_eq!(op["floor_z"], prior["floor_z"]);
                assert_eq!(op["web_mm"], prior["web_mm"]);
            }
        }
    }
}

#[test]
fn boundary_and_partial_features_and_dependent_chamfers_use_the_capped_frame() {
    let mut plan = json!({"setups":[{"id":"side", "stock":{"min":[0,0,-30],"max":[20,20,10]}, "operations":[
        {"kind":"roughing", "top_z":10., "floor_z":-30., "web_mm":2., "surface_error_mm":0.},
        {"kind":"flats", "floor_z":-30., "levels":[-5.,-1.,2.]},
        {"kind":"holes", "diameter":3., "top_z":10., "bore_top_z":10., "bottom_z":-25., "bottom":"blind", "centers":[], "features":["holes:0"]},
        {"kind":"v_channel", "angle":90., "width":2., "depth":1., "length":20., "a":[0.,10.], "b":[20.,10.], "on_flat":true, "top_z":-5., "root_z":-6., "features":["v_channels:0"]},
        {"kind":"wall_finish", "wall":"wall-1", "outer":[], "floor_z":-2., "on_flat":false},
        {"kind":"edge_chamfer", "needs_wall":"wall-1", "boundaries":"outer", "radius":0.5, "width":0.5, "top_z":10., "outer":[], "holes":[],
         "features":["fillets:0"], "wall_features":["fillets:0"]}
    ]}], "covered_by_roughing":["at-cap","partial","below"],
        "roughing_feature_depths":{"at-cap":{"floor_z":-1.,"family":"pockets"},
            "partial":{"floor_z":-2.,"family":"step_levels"},"below":{"floor_z":-20.,"family":"pockets"}}});
    let mut trace = capp::trace::Trace::default();
    for id in ["at-cap", "partial", "below"] {
        trace.decide("process.covered_by_roughing", id, "covered", "roughing reaches it", Value::Null);
    }
    trace.store(&mut plan);
    capp::finish(&mut plan, &tools(12., 80.), &[]).unwrap();
    assert_eq!(plan["covered_by_roughing"], json!(["at-cap"]));
    assert_eq!(ops(&plan).iter().find(|o| o["kind"] == "roughing").unwrap()["floor_z"], -1.);
    assert_eq!(ops(&plan).iter().find(|o| o["kind"] == "flats").unwrap()["levels"], json!([-1.,2.]));
    assert_eq!(ops(&plan).iter().find(|o| o["kind"] == "holes").unwrap()["bottom_z"], -25.);
    assert!(!ops(&plan).iter().any(|o| o["kind"] == "wall_finish" || o["kind"] == "edge_chamfer" || o["kind"] == "v_channel"));
    assert!(plan["skipped"].as_array().unwrap().iter().any(|s| s["feature"] == "fillets:0" && s["reason"].as_str().unwrap().contains("its wall")));
}

#[test]
fn no_positive_reach_keeps_the_surface_for_checks_and_removes_roughing() {
    let mut plan = planned("top");
    capp::finish(&mut plan, &tools(0.5, 80.), &[]).unwrap();
    assert!(!ops(&plan).iter().any(|o| o["kind"] == "roughing" || o["kind"] == "flats" || o["kind"] == "wall_finish"));
    assert!(plan["covered_by_roughing"].as_array().unwrap().is_empty());
    assert!(plan["setups"][0]["part_surface"]["positions"].is_array());
}

#[test]
fn schema_accepts_mesh_free_settled_roughing_but_rejects_missing_or_partial_input_meshes() {
    let validator = jsonschema::validator_for(&read("schemas/plan-1.schema.json")).unwrap();
    let mut plan = planned("top");
    assert!(validator.is_valid(&plan));
    let operations = plan["setups"][0]["operations"].as_array_mut().unwrap();
    let rough = operations.iter_mut().find(|o| o["kind"] == "roughing").unwrap();
    let positions = rough.as_object_mut().unwrap().remove("positions").unwrap();
    assert!(!validator.is_valid(&plan));
    let rough = plan["setups"][0]["operations"].as_array_mut().unwrap().iter_mut().find(|o| o["kind"] == "roughing").unwrap();
    rough.as_object_mut().unwrap().remove("indices");
    assert!(!validator.is_valid(&plan), "unsettled roughing must have a mesh");
    capp::finish(&mut plan, &tools(12., 80.), &[]).unwrap();
    assert!(validator.is_valid(&plan), "settled roughing may omit both mesh arrays");
    let rough = plan["setups"][0]["operations"].as_array_mut().unwrap().iter_mut().find(|o| o["kind"] == "roughing").unwrap();
    rough["positions"] = positions;
    assert!(!validator.is_valid(&plan), "a half-present mesh is always invalid");
}

#[test]
fn a_single_band_wall_is_finished_above_the_cap_and_reports_what_is_left_below() {
    let analysis = read("tests/fixtures/plate.analysis.json");
    let mesh = read("tests/fixtures/plate.mesh.json");
    let mut plan = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    assert_eq!(ops(&plan).iter().filter(|o| o["kind"] == "wall_finish").count(), 1);
    assert!(!ops(&plan).iter().any(|o| o["kind"] == "flats"));
    let old_floor = ops(&plan).iter().find(|o| o["kind"] == "wall_finish").unwrap()["floor_z"].as_f64().unwrap();
    assert!(old_floor < -5.);
    let library = tools(6., 80.);
    capp::finish(&mut plan, &library, &[]).unwrap();
    let wall = ops(&plan).iter().find(|o| o["kind"] == "wall_finish").expect("the reachable wall is kept");
    assert_eq!(wall["floor_z"], -5.);
    assert_eq!(wall["tool"], "em12");
    let warning = plan["decisions"].as_array().unwrap().iter().find(|d|
        d["rule"] == "process.roughing.reach@1" && d["subject"] == format!("top:{}", wall["wall"].as_str().unwrap())).unwrap();
    let text = warning["says"].as_str().unwrap();
    assert!(text.contains("outside wall limited to Z-5.0"), "{text}");
    assert!(text.contains("finishing allowance below this level is left uncut: needs an end mill"), "{text}");
    assert!(text.contains("stickout of at least"), "{text}");
    assert!(wall["because"].as_array().unwrap().contains(&warning["id"]));
    assert!(plan["tools_needed"].as_array().unwrap().iter().any(|n| n["operations"].as_array().unwrap().contains(&warning["subject"])));
    let settled = plan.clone();
    capp::settle(&mut plan, &library).unwrap();
    assert_eq!(plan, settled);

    let out = step2cam::process(&analysis, &mesh, &library).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    assert!(ops(&out["plan"]).iter().any(|o| o["kind"] == "wall_finish" && o["floor_z"] == -5.), "{}", out["plan"]["warnings"]);
    assert_eq!(out["programs"][0]["material"]["gouge"]["definite_cells"], 0);
}

#[test]
fn the_recommended_flutes_and_stickout_actually_reach_a_feature_inside_the_margin() {
    let original = json!({"setups":[{"id":"top", "stock":{"min":[0,0,-100],"max":[20,20,0]}, "operations":[
        {"kind":"roughing", "top_z":0., "floor_z":-99.5, "web_mm":0.5, "surface_error_mm":0.}
    ]}], "covered_by_roughing":["pockets:0"], "skipped":[], "warnings":[],
        "roughing_feature_depths":{"pockets:0":{"floor_z":-74.5,"family":"pockets"}}});
    let mut plan = original.clone();
    capp::finish(&mut plan, &tools(75., 75.), &[]).unwrap();
    assert_eq!(plan["covered_by_roughing"], json!([]));
    let need = plan["tools_needed"][0]["description"].as_str().unwrap();
    assert_eq!(need, "an end mill with flutes of at least 76 mm, and stickout of at least 76 mm");
    let length: f64 = need.split("flutes of at least ").nth(1).unwrap().split_whitespace().next().unwrap().parse().unwrap();
    let mut reached = original.clone();
    capp::finish(&mut reached, &tools(length, length), &[]).unwrap();
    assert_eq!(reached["covered_by_roughing"], json!(["pockets:0"]));
    assert!(ops(&reached)[0]["floor_z"].as_f64().unwrap() <= -74.5);
    assert_eq!(reached["tools_needed"], json!([]));
    // Increasing only the flutes still leaves the holder too low.
    let mut short_holder = original;
    capp::finish(&mut short_holder, &tools(length, 75.), &[]).unwrap();
    assert_eq!(short_holder["covered_by_roughing"], json!([]));
}
