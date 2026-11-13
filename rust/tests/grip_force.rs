//! M3: force decisions use resolved cutting data, and alter only facing stepdown.
use serde_json::{Value, json};
use std::f64::consts::PI;
fn read(path: &str) -> Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    serde_json::from_str(&std::fs::read_to_string(root.join(path)).unwrap()).unwrap()
}
fn run(library: &Value) -> Value {
    let result = step2cam::process(&read("tests/fixtures/synthetic/stepped-plate.analysis.json"),
        &read("tests/fixtures/synthetic/stepped-plate.mesh.json"), library).unwrap();
    assert!(result["error"].is_null(), "{}", result["error"]);
    result
}
fn near(a: f64, b: f64) { assert!((a-b).abs()<1e-8, "{a} != {b}"); }
fn num(v: &Value) -> f64 { v.as_f64().unwrap() }
fn decisions(out: &Value) -> Vec<&Value> {
    out["plan"]["decisions"].as_array().unwrap().iter().filter(|d| d["rule"]=="workholding.grip_vs_cut@1").collect()
}
#[test]
fn thin_flip_facing_is_split_by_the_force_formula() {
    let lib = read("libraries/haas-vf2.json");
    let out = run(&lib);
    let setups = out["plan"]["setups"].as_array().unwrap();
    let bottom = setups.iter().find(|s| s["flip_seating"].is_object()).unwrap();
    near(num(&bottom["clamping"]["grip_height_mm"]), 3.175);
    let face = bottom["operations"].as_array().unwrap().iter().find(|o| o["kind"]=="facing").unwrap();
    let t = lib["tools"].as_array().unwrap().iter().find(|t| t["id"]==face["tool"]).unwrap();
    let d = num(&t["diameter"]);
    let feed = num(&face["technology"]["feed_mm_min"]);
    let rpm = num(&face["technology"]["spindle_rpm"]);
    let ae = num(&face["technology"]["stepover_mm"]).min(d/2.);
    let phi = (1.-2.*ae/d).acos();
    let h = feed/(rpm*num(&t["flutes"]))*(1.-phi.cos())/phi;
    let kc = 800.*h.powf(-0.25);
    let holding = 0.0665 * num(&lib["machines"][0]["workholding"][0]["clamp_force_n"]) * 2.;
    let slope = kc*ae*feed/(PI*d*rpm);
    let limit = (holding/(2.*slope)/0.05).floor()*0.05;
    let depth = num(&bottom["stock"]["max"][2])-num(&face["floor_z"]);
    near(depth, 7.25);
    near(limit, 1.3);
    let passes = (depth/limit).ceil() as u64;
    assert_eq!(passes, 6);
    near(num(&face["technology"]["stepdown_mm"]), limit);
    let ds = decisions(&out);
    let decision = ds.iter().find(|d| d["subject"]=="bottom-1-facing").unwrap();
    assert_eq!(decision["outcome"], "stepped_down");
    assert_eq!(decision["passes"], passes);
    near(num(&decision["cutting_force_n"]), slope*depth);
    near(num(&decision["per_pass_force_n"]), slope*limit);
    assert!(slope*limit*2. <= holding);
    assert!(slope*(limit+0.05)*2. > holding);
    // Every retained operation in every gripped setup has numeric evidence, linked to it.
    for setup in setups.iter().filter(|s| s["clamping"].is_object()) {
        assert!(ds.iter().any(|d| d["subject"]==setup["id"] && d["holding_force_n"].is_number()));
        for op in setup["operations"].as_array().unwrap() {
            assert!(ds.iter().any(|d| d["cutting_force_n"].is_number() && op["because"].as_array().unwrap().contains(&d["id"])));
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
    assert!(ds.iter().all(|d| d["outcome"]=="checked"));
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
