use serde_json::{Value, json};
fn read(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
}
#[test]
fn complex_front_threads_run_with_real_entry_clearance() {
    let analysis = read("../tests/fixtures/synthetic/stepped-plate.analysis.json");
    let mesh = read("../tests/fixtures/synthetic/stepped-plate.mesh.json");
    for path in ["../demo/tools.json", "../libraries/haas-vf2.json", "../libraries/linuxcnc-vmc.json"] {
        let out = step2cam::process(&analysis, &mesh, &read(path)).unwrap();
        assert!(out["error"].is_null(), "{path}: {}", out["error"]);
        let front = out["plan"]["setups"].as_array().unwrap().iter().find(|s| s["id"] == "front").unwrap();
        let thread = front["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "thread").unwrap_or_else(|| panic!("{path}: {}", out["log"]));
        assert_eq!(thread["features"], json!(["holes:48.57", "holes:49.58"]));
        for setup in out["plan"]["setups"].as_array().unwrap().iter().skip(1) {
            assert!(setup["clamping"]["contact_area_mm2"].as_array().unwrap().iter().all(|a| a.as_f64().unwrap()>0.), "{path}: {setup}");
        }
    }
}
