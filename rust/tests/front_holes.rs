use serde_json::{Value, json};
fn read(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
}
/// The stepped plate's front M3 holes, as `process` plans them with `library`.
fn front_thread(analysis: &Value, library: &str) -> (Value, Option<Value>) {
    let mesh = read("../tests/fixtures/synthetic/stepped-plate.mesh.json");
    let mut shop = read(library);
    let standard = step2cam::process(analysis, &mesh, &shop).unwrap();
    if !library.contains("haas") {
        assert!(standard["plan"]["warnings"].to_string().contains("no standard parallels height"));
        // Explicit synthetic support heights keep the tool-entry regression executable.
        shop["machines"][0]["workholding"][0]["parallels_heights_mm"] = if library.contains("demo") { json!([8.,12.7,15.875]) } else { json!([120.,138.]) };
    }
    let out = step2cam::process(analysis, &mesh, &shop).unwrap();
    assert!(out["error"].is_null(), "{library}: {}", out["error"]);
    let front = out["plan"]["setups"].as_array().unwrap().iter().find(|s| s["id"] == "front").unwrap();
    let thread = front["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "thread").cloned();
    (out, thread)
}
fn depth(thread: &Value) -> f64 {
    thread["top_z"].as_f64().unwrap() - thread["bottom_z"].as_f64().unwrap()
}

#[test]
fn complex_front_threads_run_with_real_entry_clearance() {
    // The drawing asks for at least 8.5 mm of full thread (hole.full_thread), which no tap in
    // these libraries can reach in the 10 mm hole: only the Carvera's long thread mill cuts it.
    let analysis = read("../tests/fixtures/synthetic/stepped-plate.analysis.json");
    assert_eq!(analysis["resume"]["answers"]["hole.full_thread:hole_patterns.set:48.49.57.58"], "8.5");
    for path in ["../demo/tools.json", "../libraries/haas-vf2.json", "../libraries/linuxcnc-vmc.json"] {
        let (out, thread) = front_thread(&analysis, path);
        if path != "../demo/tools.json" {
            // These inventories cannot finish this blind M3 to the asked depth. Preserve the
            // concrete resource limit rather than inventing a longer tool: the thread is not
            // an operation, and its features say why.
            assert!(thread.is_none(), "{path}: {thread:?}");
            let skipped: Vec<_> = out["plan"]["skipped"].as_array().unwrap().iter().filter(|s| s["reason"].as_str().unwrap().starts_with("thread left out: ")).collect();
            assert_eq!(skipped.iter().map(|s| s["feature"].clone()).collect::<Vec<_>>(), [json!("holes:48.57"), json!("holes:49.58")]);
            let why = if path.contains("haas") { "tmM3: reaches 7.33" } else { "⌀3 does not fit the ⌀2.5 hole" };
            assert!(skipped.iter().all(|s| s["reason"].as_str().unwrap().contains(why)), "{path}: {skipped:?}");
            if path.contains("haas") {
                // tapM3 is refused at the asked depth rather than cutting 8 mm of it.
                let text = out["plan"]["decisions"].to_string();
                assert!(text.contains("full thread 8.50 mm + chamfer 1.50 mm + bottom clearance 0.50 mm requires 10.50 mm; drilled full diameter is 10.00 mm"), "{text}");
            }
            continue;
        }
        let thread = thread.unwrap_or_else(|| panic!("{path}: {}", out["log"]));
        assert_eq!(thread["features"], json!(["holes:48.57", "holes:49.58"]));
        assert!(thread.get("default_depth").is_none(), "{thread}");
        assert!(depth(&thread) >= 8.5 - 1e-9, "{thread}");
        let program = out["programs"].as_array().unwrap().iter().find(|p| p["setup"]=="front").unwrap();
        let operations = program["job"]["operations"].as_array().unwrap();
        let milled = operations.iter().find(|o| o["kind"]=="thread_mill").expect("Carvera front thread must execute, not just remain untooled in the plan");
        assert_eq!(milled["centers_mm"].as_array().unwrap().len(),2);
        assert!(program["job"]["stock"]["cell_size_mm"].as_f64().unwrap()<0.03);
        for setup in out["plan"]["setups"].as_array().unwrap().iter().skip(1) {
            assert!(setup["clamping"]["contact_area_mm2"].as_array().unwrap().iter().all(|a| a.as_f64().unwrap()>0.), "{path}: {setup}");
        }
    }
}

#[test]
fn without_a_full_thread_answer_the_haas_tap_cuts_the_default_shorter() {
    // Review M7: the default 8.5 mm full thread left no room for the form C tap's lead; tapM3
    // cuts 10 - (3 + 1) × 0.5 = 8 mm of it, and the job taps both holes.
    let mut analysis = read("../tests/fixtures/synthetic/stepped-plate.analysis.json");
    analysis["resume"]["answers"].as_object_mut().unwrap().remove("hole.full_thread:hole_patterns.set:48.49.57.58");
    let (out, thread) = front_thread(&analysis, "../libraries/haas-vf2.json");
    let thread = thread.unwrap_or_else(|| panic!("{}", out["log"]));
    assert_eq!(thread["tool"], "tapM3", "{}", thread["tool_note"]);
    assert!((depth(&thread) - 8.).abs() < 1e-9, "{thread}");
    let program = out["programs"].as_array().unwrap().iter().find(|p| p["setup"]=="front").unwrap();
    let tapped = program["job"]["operations"].as_array().unwrap().iter().find(|o| o["kind"]=="tapping").expect("the front M3 is tapped");
    assert_eq!(tapped["centers_mm"].as_array().unwrap().len(),2);
}
