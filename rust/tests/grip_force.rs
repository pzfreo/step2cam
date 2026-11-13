//! M3: force decisions use resolved cutting data, and alter only facing stepdown.
use serde_json::{Value, json};
use std::f64::consts::PI;
fn read(path: &str) -> Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    serde_json::from_str(&std::fs::read_to_string(root.join(path)).unwrap()).unwrap()
}
fn run(library: &Value) -> Value { run_named("synthetic/stepped-plate", library) }
fn run_named(name: &str, library: &Value) -> Value {
    let result = step2cam::process(&read(&format!("tests/fixtures/{name}.analysis.json")),
        &read(&format!("tests/fixtures/{name}.mesh.json")), library).unwrap();
    assert!(result["error"].is_null(), "{}", result["error"]);
    result
}
fn near(a: f64, b: f64) { assert!((a-b).abs()<1e-8, "{a} != {b}"); }
fn num(v: &Value) -> f64 { v.as_f64().unwrap() }
fn decisions(out: &Value) -> Vec<&Value> {
    out["plan"]["decisions"].as_array().unwrap().iter().filter(|d| d["rule"]=="machine.grip_vs_cut@1").collect()
}
#[test]
fn thin_flip_facing_is_split_by_the_force_formula() {
    // Deliberately weak shop setting: preserve the pass-limit regression without
    // pretending the production DX6 is tightened at its weakest published row.
    let mut lib = read("libraries/haas-vf2.json");
    lib["machines"][0]["workholding"][0]["clamp_force_n"] = json!(2000.);
    let out = run(&lib);
    let setups = out["plan"]["setups"].as_array().unwrap();
    let bottom = setups.iter().find(|s| s["flip_seating"].is_object()).unwrap();
    near(num(&bottom["clamping"]["grip_height_mm"]), 3.175);
    let face = bottom["operations"].as_array().unwrap().iter().find(|o| o["kind"]=="facing").unwrap();
    let t = lib["tools"].as_array().unwrap().iter().find(|t| t["id"]==face["tool"]).unwrap();
    let d = num(&t["diameter"]);
    let feed = num(&face["technology"]["feed_mm_min"]);
    let rpm = num(&face["technology"]["spindle_rpm"]);
    let ae = num(&face["technology"]["stepover_mm"]).min(d);
    let phi = (1.-2.*ae/d).acos();
    let h = feed/(rpm*num(&t["flutes"]))*(1.-phi.cos())/phi;
    let kc = 800.*h.powf(-0.25);
    let holding = 0.095 * num(&lib["machines"][0]["workholding"][0]["clamp_force_n"]) * 2.;
    let slope = kc*ae*feed/(PI*d*rpm);
    let limit = (holding/(2.*slope)/0.05).floor()*0.05;
    let depth = num(&bottom["stock"]["max"][2])-num(&face["floor_z"]);
    near(depth, 7.25);
    near(limit, 1.65);
    let passes = (depth/limit).ceil() as u64;
    assert_eq!(passes, 5);
    near(num(&face["technology"]["stepdown_mm"]), limit);
    let ds = decisions(&out);
    let decision = ds.iter().find(|d| d["subject"]=="bottom-1-facing").unwrap();
    assert_eq!(decision["outcome"], "stepped_down");
    assert_eq!(decision["passes"], passes);
    near(num(&decision["cutting_force_n"]), slope*depth);
    near(num(&decision["per_pass_force_n"]), slope*limit);
    assert!(slope*limit*2. <= holding);
    assert!(slope*(limit+0.05)*2. > holding);
    // Every milling operation has numeric evidence; axial cycles explicitly decline the sliding check.
    for setup in setups.iter().filter(|s| s["clamping"].is_object()) {
        assert!(ds.iter().any(|d| d["subject"]==setup["id"] && d["holding_force_n"].is_number()));
        for op in setup["operations"].as_array().unwrap() {
            let own = ds.iter().find(|d| op["because"].as_array().unwrap().contains(&d["id"])).unwrap();
            let spec = lib["tools"].as_array().unwrap().iter().find(|t| t["id"]==op["tool"]).unwrap();
            if matches!(spec["type"].as_str(), Some("drill" | "spot_drill" | "tap")) {
                assert_eq!(own["outcome"], "none");
                assert_eq!(own["force_model"], "thrust into the supports; not a sliding load on the grip");
                assert!(own.get("cutting_force_n").is_none());
            } else { assert!(own["cutting_force_n"].is_number()); }
        }
    }
    let schema = jsonschema::validator_for(&read("schemas/plan-1.schema.json")).unwrap();
    let errors: Vec<_> = schema.iter_errors(&out["plan"]).map(|e| format!("{}: {e}", e.instance_path())).collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    // The adapter must execute the limit, not merely report it in the trace.
    let programs = out["programs"].as_array().unwrap();
    let job_face = programs.iter().flat_map(|p| p["job"]["operations"].as_array().unwrap()).find(|o| o["id"]=="bottom-1-facing").unwrap();
    near(num(&job_face["stepdown_mm"]), limit);
    assert!(ds.iter().any(|d| d["outcome"]=="warning" && d["cutting_force_n"].is_number() && d["says"].as_str().unwrap().contains("operation unchanged")));
}
#[test]
fn no_force_warns_once_per_setup_and_keeps_facing_data() {
    for path in ["libraries/haas-vf2.json", "demo/tools.json"] {
        let mut lib = read(path);
        let vise = lib["machines"][0]["workholding"][0].as_object_mut().unwrap();
        vise.remove("clamp_force_n"); vise.remove("clamp_torque_nm");
        let out = run(&lib);
        let ds = decisions(&out);
        for setup in out["plan"]["setups"].as_array().unwrap().iter().filter(|s| s["clamping"].is_object()) {
            let warnings: Vec<_> = ds.iter().filter(|d| d["subject"]==setup["id"]).collect();
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0]["says"].as_str().unwrap().starts_with("grip not checked against the cut: the library gives no clamping force for "));
            for face in setup["operations"].as_array().unwrap().iter().filter(|o| o["kind"]=="facing") {
                let tool = lib["tools"].as_array().unwrap().iter().find(|t| t["id"]==face["tool"]).unwrap();
                near(num(&face["technology"]["stepdown_mm"]), num(&tool["stepdown_mm"]));
            }
        }
        assert!(ds.iter().all(|d| d["outcome"]=="warning"));
    }
}
#[test]
fn ample_force_keeps_the_original_facing_stepdown() {
    let mut lib = read("libraries/haas-vf2.json");
    lib["machines"][0]["workholding"][0]["clamp_force_n"] = json!(1e9);
    let out = run(&lib);
    let ds = decisions(&out);
    assert!(!ds.is_empty());
    for decision in ds {
        if decision["force_model"]=="thrust into the supports; not a sliding load on the grip" {
            assert_eq!(decision["outcome"], "none");
            assert!(decision.get("cutting_force_n").is_none());
        } else { assert_eq!(decision["outcome"], "checked"); }
    }
    for setup in out["plan"]["setups"].as_array().unwrap() {
        for face in setup["operations"].as_array().unwrap().iter().filter(|o| o["kind"]=="facing") {
            near(num(&face["technology"]["stepdown_mm"]), 10.);
        }
    }
}
#[test]
fn clamp_force_and_its_citation_are_validated() {
    let schema = jsonschema::validator_for(&read("schemas/resources-1.schema.json")).unwrap();
    assert!(schema.is_valid(&read("libraries/haas-vf2.json")));
    for bad in [json!(0), json!(-1), json!("500"), Value::Null] {
        let mut lib = read("libraries/haas-vf2.json");
        lib["machines"][0]["workholding"][0]["clamp_force_n"] = bad;
        assert!(capp::resources::normalize(&lib).unwrap_err().contains("clamp_force_n"));
        assert!(!schema.is_valid(&lib));
    }
    let mut lib = read("libraries/haas-vf2.json");
    lib["machines"][0]["workholding"][0]["_sources"] = json!({});
    assert!(capp::resources::normalize(&lib).unwrap_err().contains("_sources.clamp_force"));
    assert!(!schema.is_valid(&lib));
}


#[test]
fn moderate_tightening_preserves_facing_and_records_torque_with_force() {
    let lib = read("libraries/haas-vf2.json");
    let out = run(&lib);
    let ds = decisions(&out);
    assert!(!ds.iter().any(|d| d["outcome"]=="stepped_down"));
    for setup in out["plan"]["setups"].as_array().unwrap().iter().filter(|s| s["clamping"].is_object()) {
        let force = &setup["clamping"]["force_check"];
        near(num(&force["clamp_force_n"]), 3440.*4.4482216152605);
        near(num(&force["clamp_torque_nm"]), 40.*1.3558179483314);
        near(num(&force["friction_coefficient"]), 0.095);
        near(num(&force["holding_force_n"]), 2907.3576477342626);
        near(num(&force["allowable_cutting_force_n"]), 1453.6788238671313);
        for face in setup["operations"].as_array().unwrap().iter().filter(|o| o["kind"]=="facing") {
            near(num(&face["technology"]["stepdown_mm"]), 10.);
            let own = ds.iter().find(|d| face["because"].as_array().unwrap().contains(&d["id"])).unwrap();
            assert_eq!(own["outcome"], "checked");
            assert!(num(&own["cutting_force_n"]) <= num(&force["allowable_cutting_force_n"]));
        }
    }
    for candidate in out["plan"]["workholding"]["candidates"].as_array().unwrap().iter().filter(|c| c["available"]==true) {
        for grip in candidate["grips"].as_array().unwrap() {
            near(num(&grip["force_check"]["clamp_torque_nm"]), 40.*1.3558179483314);
            near(num(&grip["force_check"]["clamp_force_n"]), 3440.*4.4482216152605);
        }
    }
    let schema = jsonschema::validator_for(&read("schemas/plan-1.schema.json")).unwrap();
    let errors: Vec<_> = schema.iter_errors(&out["plan"]).map(|e| format!("{}: {e}", e.instance_path())).collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

fn settled(library: &Value) -> Value {
    let mut plan = capp::plan::plan(&read("tests/fixtures/synthetic/stepped-plate.analysis.json"),
        Some(&read("tests/fixtures/synthetic/stepped-plate.mesh.json")), &Default::default()).unwrap();
    capp::finish(&mut plan, library, &[]).unwrap();
    json!({"plan":plan})
}
fn force(ap: f64, ae: f64, diameter: f64, feed: f64, rpm: f64, flutes: f64) -> f64 {
    let phi = (1.-2.*ae/diameter).acos();
    let h = feed/(rpm*flutes)*(1.-phi.cos())/phi;
    800.*h.powf(-0.25)*ap*ae*feed/(PI*diameter*rpm)
}
#[test]
fn engagement_above_half_diameter_and_full_width_roughing_are_checked() {
    let mut lib = read("libraries/haas-vf2.json");
    lib["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"]=="em20").unwrap()["stepover_mm"] = json!(15.);
    let out = settled(&lib);
    let ds = decisions(&out);
    let face = ds.iter().find(|d| d["subject"]=="top-1-facing").unwrap();
    near(num(&face["radial_width_mm"]), 15.);
    near(num(&face["cutting_force_n"]), force(1.,15.,20.,4804.87,7882.,4.));
    assert!(num(&face["cutting_force_n"]) > force(1.,10.,20.,4804.87,7882.,4.));
    let rough = ds.iter().find(|d| d["subject"]=="top-2-roughing").unwrap();
    near(num(&rough["radial_width_mm"]), 20.);
    near(num(&rough["cutting_force_n"]), force(9.,20.,20.,4804.87,7882.,4.));
    assert_eq!(rough["outcome"], "warning");
    near(num(&rough["axial_depth_mm"]), 9.);
    near(num(&rough["suggested_stepdown_mm"]), 5.2);
    let text = rough["says"].as_str().unwrap();
    for premise in ["entry unknown", "40 ft·lbf", "ap9.00 × ae20.00", "reduce stepdown to ≤5.20"] {
        assert!(text.contains(premise), "{text}");
    }
    // ae remains bounded by D even when the library requests a larger stepover.
    lib["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"]=="em20").unwrap()["stepover_mm"] = json!(25.);
    let wide = settled(&lib);
    let ds = decisions(&wide);
    near(num(&ds.iter().find(|d| d["subject"]=="top-1-facing").unwrap()["radial_width_mm"]), 20.);
}
#[test]
fn an_unholdable_minimum_face_warns_without_failing_or_altering_the_cut() {
    let mut lib = read("libraries/haas-vf2.json");
    lib["machines"][0]["workholding"][0]["clamp_force_n"] = json!(1.);
    let out = run(&lib);
    let ds = decisions(&out);
    for setup in out["plan"]["setups"].as_array().unwrap() {
        for op in setup["operations"].as_array().unwrap().iter().filter(|o| o["kind"]=="facing") {
            near(num(&op["technology"]["stepdown_mm"]), 10.);
            let own = ds.iter().find(|d| op["because"].as_array().unwrap().contains(&d["id"])).unwrap();
            assert_eq!(own["outcome"], "warning");
            assert!(own["says"].as_str().unwrap().contains("cannot be held at any depth"));
            assert!(num(&own["per_pass_force_n"])*2. > num(&own["holding_force_n"]));
            assert!(num(&own["cutting_force_n"]) > num(&own["holding_force_n"]));
        }
    }
}
#[test]
fn helix_milled_holes_stay_sideways_while_spots_and_taps_are_axial() {
    let mut lib = read("libraries/haas-vf2.json");
    lib["tools"].as_array_mut().unwrap().retain(|t| t["type"]!="drill");
    let out = settled(&lib);
    let ds = decisions(&out);
    let holes: Vec<_> = out["plan"]["setups"].as_array().unwrap().iter()
        .flat_map(|s| s["operations"].as_array().unwrap()).filter(|o| o["kind"]=="holes").collect();
    assert!(!holes.is_empty());
    for op in holes {
        let tool = lib["tools"].as_array().unwrap().iter().find(|t| t["id"]==op["tool"]).unwrap();
        assert_eq!(tool["type"], "end_mill");
        let own = ds.iter().find(|d| op["because"].as_array().unwrap().contains(&d["id"])).unwrap();
        assert!(own["cutting_force_n"].is_number());
        assert_ne!(own["outcome"], "none");
    }
    // Supply a tapped M5 hole on the plate using the existing recognised hole.
    let lib = read("libraries/haas-vf2.json");
    let mut analysis = read("tests/fixtures/plate.analysis.json");
    analysis["resume"] = json!({"answers":{"part.material":"Aluminium 6082-T6","hole.function:holes:12":"tapped:M5"}});
    let mut plan = capp::plan::plan(&analysis, Some(&read("tests/fixtures/plate.mesh.json")), &Default::default()).unwrap();
    capp::finish(&mut plan, &lib, &[]).unwrap();
    let out = json!({"plan":plan});
    let ds = decisions(&out);
    let tap = out["plan"]["setups"].as_array().unwrap().iter().flat_map(|s| s["operations"].as_array().unwrap())
        .find(|o| o["kind"]=="thread" && o["tool"].as_str().is_some_and(|id| id.starts_with("tap"))).unwrap();
    let own = ds.iter().find(|d| tap["because"].as_array().unwrap().contains(&d["id"])).unwrap();
    assert_eq!(own["outcome"], "none");
    assert_eq!(own["force_model"], "thrust into the supports; not a sliding load on the grip");
    assert!(own.get("cutting_force_n").is_none());
}


#[test]
fn classified_stock_edge_roughing_uses_stepover_and_closed_entry_uses_a_slot() {
    let lib = read("libraries/haas-vf2.json");
    for (access, width, outcome) in [("stock_edge", 8., "checked"), ("closed",20.,"warning"), ("unknown",20.,"warning")] {
        let mut plan = capp::plan::plan(&read("tests/fixtures/synthetic/stepped-plate.analysis.json"),
            Some(&read("tests/fixtures/synthetic/stepped-plate.mesh.json")), &Default::default()).unwrap();
        plan["setups"][0]["operations"].as_array_mut().unwrap().iter_mut().find(|o| o["kind"]=="roughing").unwrap()["entry_access"] = json!(access);
        capp::finish(&mut plan, &lib, &[]).unwrap();
        let out = json!({"plan":plan});
        let ds = decisions(&out);
        let rough = ds.iter().find(|d| d["subject"]=="top-2-roughing").unwrap();
        near(num(&rough["axial_depth_mm"]),9.);
        near(num(&rough["radial_width_mm"]),width);
        near(num(&rough["cutting_force_n"]),force(9.,width,20.,4804.87,7882.,4.));
        assert_eq!(rough["outcome"],outcome);
        if outcome=="warning" {
            let limit=num(&rough["suggested_stepdown_mm"]);
            assert!(force(limit,width,20.,4804.87,7882.,4.)<=num(&rough["allowable_cutting_force_n"]));
            assert!(force(limit+0.05,width,20.,4804.87,7882.,4.)>num(&rough["allowable_cutting_force_n"]));
        }
        let schema=jsonschema::validator_for(&read("schemas/plan-1.schema.json")).unwrap();
        assert!(schema.is_valid(&out["plan"]));
    }
}
#[test]
fn missing_material_reference_warns_once_per_setup_not_per_operation() {
    let mut lib=read("libraries/haas-vf2.json");
    lib["overrides"]["material"]=json!("Steel");
    let out=settled(&lib);
    let ds=decisions(&out);
    let warnings:Vec<_>=ds.iter().filter(|d| d["outcome"]=="warning").collect();
    let setups:Vec<_>=out["plan"]["setups"].as_array().unwrap().iter().filter(|s| s["clamping"].is_object()).collect();
    assert_eq!(warnings.len(),setups.len());
    for setup in setups {
        assert_eq!(warnings.iter().filter(|d| d["subject"]==setup["id"]).count(),1);
    }
}
#[test]
fn unspecified_material_is_named_in_the_grouped_warning() {
    let lib=read("libraries/haas-vf2.json");
    let mut plan=capp::plan::plan(&read("tests/fixtures/synthetic/square-slots.analysis.json"),
        Some(&read("tests/fixtures/synthetic/square-slots.mesh.json")),&Default::default()).unwrap();
    capp::finish(&mut plan,&lib,&[]).unwrap();
    let out=json!({"plan":plan});
    let ds=decisions(&out);
    let warnings:Vec<_>=ds.iter().filter(|d| d["outcome"]=="warning").collect();
    let setups=out["plan"]["setups"].as_array().unwrap().iter().filter(|s| s["clamping"].is_object()).count();
    assert_eq!(warnings.len(),setups);
    assert!(warnings.iter().all(|d| d["says"].as_str().unwrap().ends_with("for unspecified material")));
}
#[test]
fn mounting_bore_uses_the_executed_helix_pitch_not_an_imaginary_three_degree_pitch() {
    let lib=read("libraries/haas-vf2.json");
    let out=run_named("mounting-plate",&lib);
    let program=&out["programs"][0];
    let job=program["job"]["operations"].as_array().unwrap().iter().find(|o| o["id"]=="top-5-holes").unwrap();
    near(num(&job["stepdown_mm"]),10.);
    let operation=program["operations"].as_array().unwrap().iter().find(|o| o["id"]=="top-5-holes").unwrap();
    let moves=&program["moves"].as_array().unwrap()[operation["start"].as_u64().unwrap() as usize..operation["end"].as_u64().unwrap() as usize];
    let helix=moves.iter().find(|m| m["kind"]=="lead").unwrap();
    let points=helix["points"].as_array().unwrap();
    let center=&job["centers_mm"][0];
    let turns=points.windows(2).map(|p| {
        let a=[num(&p[0][0])-num(&center[0]),num(&p[0][1])-num(&center[1])];
        let b=[num(&p[1][0])-num(&center[0]),num(&p[1][1])-num(&center[1])];
        (a[0]*b[1]-a[1]*b[0]).atan2(a[0]*b[0]+a[1]*b[1])
    }).sum::<f64>().abs()/(2.*PI);
    near(turns,1.);
    let pitch=(num(&points.first().unwrap()[2])-num(&points.last().unwrap()[2]))/turns;
    near(pitch,9.5);
    let ds=decisions(&out);
    let bore=ds.iter().find(|d| d["subject"]=="top-5-holes").unwrap();
    // Eight mm of actual part material, below the faced top, is less than
    // the executed pitch. A hypothetical 1.3 mm pitch must not lower this load.
    near(num(&bore["axial_depth_mm"]),8.);
    near(num(&bore["cutting_force_n"]),force(8.,20.,20.,4804.87,7882.,4.));
    assert_eq!(bore["outcome"],"warning");
    near(num(&bore["suggested_stepdown_mm"]),5.2);
}
