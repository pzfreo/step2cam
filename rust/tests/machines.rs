//! What a machine's capabilities decide: the demo's Makera Carvera (demo/tools.json), and a mill
//! too small for the part.
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn read(path: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path)).unwrap()).unwrap()
}
fn finished(part: &str, library: &Value) -> Value {
    let analysis = read(&format!("tests/fixtures/{part}.analysis.json"));
    let mesh = read(&format!("tests/fixtures/{part}.mesh.json"));
    let mut plan = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    capp::finish(&mut plan, library, &[]).unwrap();
    plan
}
fn decisions<'a>(plan: &'a Value, rule: &str) -> Vec<&'a Value> {
    plan["decisions"].as_array().unwrap().iter().filter(|d| d["rule"].as_str().unwrap().starts_with(rule)).collect()
}
fn ops(plan: &Value) -> Vec<&Value> {
    plan["setups"].as_array().unwrap().iter().flat_map(|s| s["operations"].as_array().unwrap()).collect()
}

/// The Carvera holds shanks up to 1/4": the larger tools are never chosen, and the trace says
/// why each was left out.
#[test]
fn the_carvera_uses_only_tools_its_collet_holds() {
    let library = read("demo/tools.json");
    let plan = finished("synthetic/stepped-plate", &library);
    let shank = |id: &str| {
        let t = library["tools"].as_array().unwrap().iter().find(|t| t["id"] == id).unwrap();
        t.get("shank_diameter").and_then(Value::as_f64).or(t["diameter"].as_f64()).unwrap()
    };
    let used: Vec<&str> = ops(&plan).iter().filter_map(|o| o["tool"].as_str()).collect();
    assert!(!used.is_empty());
    assert!(used.iter().all(|id| shank(id) <= 6.35), "{used:?}");
    let left_out = decisions(&plan, "machine.spindle.tool_size@");
    assert!(left_out.iter().any(|d| d["subject"] == "em12"), "{left_out:?}");
    assert!(plan["setups"].as_array().unwrap().iter().all(|s| s["machine"] == "carvera"));
}

/// Its spindle tops out at 15,000 rpm: faster cutting data are capped, the feed scaled by the
/// same ratio.
#[test]
fn the_carvera_caps_the_spindle_speed() {
    let mut library = read("demo/tools.json");
    // em6 asks for more than the spindle gives.
    let em6 = library["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == "em6").unwrap();
    em6["spindle_rpm"] = json!(20000);
    let plan = finished("synthetic/stepped-plate", &library);
    for op in ops(&plan) {
        if let Some(rpm) = op["technology"]["spindle_rpm"].as_f64() {
            assert!(rpm <= 15000., "{}", op["kind"]);
        }
    }
    let all = ops(&plan);
    let capped: Vec<&&Value> = all.iter().filter(|o| o["technology"]["capped"] == true).collect();
    assert!(!capped.is_empty(), "em6 is capped");
    if let Some(op) = capped.first() {
        let tool = library["tools"].as_array().unwrap().iter().find(|t| t["id"] == op["tool"]).unwrap();
        let k = 15000. / tool["spindle_rpm"].as_f64().unwrap();
        let feed = op["technology"]["feed_mm_min"].as_f64().unwrap();
        assert!((feed - tool["feed_mm_min"].as_f64().unwrap() * k).abs() < 0.01, "feed scaled with the speed");
        assert!(!decisions(&plan, "machine.spindle.max_rpm@").is_empty());
    }
}

/// A setup that needs more tools than the changer's 6 pockets is flagged.
#[test]
fn more_tools_than_pockets_is_flagged() {
    let mut library = read("demo/tools.json");
    library["machines"][0]["tool_changer"]["pockets"] = json!(2);
    let plan = finished("synthetic/stepped-plate", &library);
    assert!(plan["warnings"].to_string().contains("the changer holds 2"), "{}", plan["warnings"]);
}

/// A mill too small for the stock does not cut that setup, and says why for every feature.
#[test]
fn stock_larger_than_the_travel_is_not_cut() {
    let mut library = read("demo/tools.json");
    library["machines"][0]["axes"]["travel_mm"] = json!({"x": 80, "y": 240, "z": 140});
    let plan = finished("synthetic/stepped-plate", &library);
    let top = plan["setups"].as_array().unwrap().iter().find(|s| s["id"] == "top").unwrap();
    assert!(top["operations"].as_array().unwrap().is_empty());
    assert!(plan["warnings"].to_string().contains("does not fit"), "{}", plan["warnings"]);
    assert!(plan["skipped"].as_array().unwrap().iter().any(|s| s["reason"].as_str().unwrap().contains("travel")));
}

/// With no machine described, nothing is checked, and the trace says so.
#[test]
fn without_a_machine_the_trace_says_nothing_was_checked() {
    let mut library = read("demo/tools.json");
    library.as_object_mut().unwrap().remove("machines");
    let plan = finished("plate", &library);
    let none = decisions(&plan, "machine.for_setup@");
    assert_eq!(none.len(), 1);
    assert_eq!(none[0]["outcome"], "none");
}

/// The whole program still posts on the Carvera, and the vise check stays quiet without a vise.
#[test]
fn the_stepped_plate_posts_on_the_carvera() {
    let analysis = read("tests/fixtures/synthetic/stepped-plate.analysis.json");
    let mesh = read("tests/fixtures/synthetic/stepped-plate.mesh.json");
    let out = step2cam::process(&analysis, &mesh, &read("demo/tools.json")).unwrap();
    assert!(out["post_error"].is_null(), "{}", out["post_error"]);
    assert!(!out["programs"].as_array().unwrap().is_empty());
}
