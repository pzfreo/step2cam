//! Missing-tool queries report resources without changing the plan's operations.
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures").join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn library() -> Value {
    let mut tools = fixture("../../tests/fixtures/placeholder-tools.json");
    // These tall synthetic stocks exercise missing tools, independently of vise grip limits.
    tools["machines"][0].as_object_mut().unwrap().remove("workholding");
    tools
}
fn simple(ops: Value) -> Value {
    json!({"setups": [{"id": "top", "stock": {"min": [-20,-20,-140], "max": [20,20,0]}, "operations": ops}],
        "skipped": [], "warnings": [], "covered_by_roughing": []})
}

#[test]
fn missing_hole_tools_use_library_tolerance_and_merge_identical_queries() {
    let mut tools = library();
    tools["drill_tolerance_mm"] = json!(0.2);
    // Keep a valid library, with nothing narrow enough for these holes.
    tools["tools"].as_array_mut().unwrap().retain(|t| t["type"] == "end_mill");
    for tool in tools["tools"].as_array_mut().unwrap() { tool["diameter"] = json!(20); tool["holder_diameter"] = json!(30); }
    let mut plan = simple(json!([
        {"kind":"holes", "diameter":18.5, "top_z":0, "bore_top_z":0, "bottom_z":-131.7, "bottom":"blind", "centers":[], "features":["holes:0"]},
        {"kind":"holes", "diameter":18.5, "top_z":0, "bore_top_z":0, "bottom_z":-131.7, "bottom":"blind", "centers":[], "features":["holes:1"]}
    ]));
    capp::finish(&mut plan, &tools, &[]).unwrap();
    let need = "a drill of ⌀18.30–18.50 with flutes of at least 131.71 mm plus point length, or an end mill under ⌀18.45 with flutes of at least 132 mm";
    assert_eq!(plan["tools_needed"].as_array().unwrap().len(), 1);
    assert_eq!(plan["tools_needed"][0]["description"], need);
    assert_eq!(plan["tools_needed"][0]["features"], json!(["holes:0", "holes:1"]));
    for op in plan["setups"][0]["operations"].as_array().unwrap() {
        let note = op["tool_note"].as_str().unwrap();
        assert!(note.starts_with("no drill of ⌀18.5 and no end mill small enough to bore it"));
        assert!(note.ends_with(&format!(": needs {need}")));
    }
}

#[test]
fn notch_query_rounds_diameter_down_and_reach_up() {
    let mut plan = simple(json!([{"kind":"notch", "width":1.237, "length":1.237, "open":false, "outer":[], "top_z":0, "floor_z":-12.01, "wall_top":0, "bottom":"blind", "features":["notches:0"]}]));
    capp::finish(&mut plan, &library(), &[]).unwrap();
    assert_eq!(plan["tools_needed"][0]["description"], "an end mill under ⌀1.18 with flutes of at least 13 mm");
    assert_eq!(plan["tools_needed"][0]["features"], json!(["notches:0"]));
    assert!(plan["setups"][0]["operations"][0]["tool_note"].as_str().unwrap().ends_with(": needs an end mill under ⌀1.18 with flutes of at least 13 mm"));
}

#[test]
fn actual_camlib_reach_refusals_keep_the_reason_and_index_every_feature() {
    let mut tools = library();
    for tool in tools["tools"].as_array_mut().unwrap() {
        if tool["type"] == "drill" || tool["type"] == "end_mill" {
            tool["flute_length"] = json!(2);
        }
    }
    let out = step2cam::process(&fixture("plate.analysis.json"), &fixture("plate.mesh.json"), &tools).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let plan = &out["plan"];
    let validator = jsonschema::validator_for(&fixture("../../schemas/plan-1.schema.json")).unwrap();
    assert!(validator.is_valid(plan), "the process plan with nonempty tools_needed must match its schema");
    let skipped: Vec<_> = plan["skipped"].as_array().unwrap().iter()
        .filter(|s| s["reason"].as_str().unwrap().contains("below the stock top") && s["reason"].as_str().unwrap().contains("flutes are 2.0 mm")).collect();
    assert!(skipped.len() >= 8, "{}", plan["skipped"]);
    for skip in &skipped {
        let reason = skip["reason"].as_str().unwrap();
        let (_, need) = reason.rsplit_once(": needs ").expect("reach refusal needs a tool");
        assert!(reason.starts_with("left out: camlib refused the holes: "), "{reason}");
        assert!(!reason.contains("top-"), "{reason}");
        assert!(reason.contains("flutes are 2.0 mm"));
        assert!(need.contains("with flutes of at least 1"), "{need}");
        let matches: Vec<_> = plan["tools_needed"].as_array().unwrap().iter().filter(|n| n["description"] == need).collect();
        assert_eq!(matches.len(), 1);
        assert!(matches[0]["features"].as_array().unwrap().contains(&skip["feature"]));
    }
    let group = skipped.iter().find(|s| s["feature"] == "holes:6").unwrap();
    let need = group["reason"].as_str().unwrap().rsplit_once(": needs ").unwrap().1;
    assert_eq!(need, "a drill of ⌀6.55–6.60 with flutes of at least 12.01 mm plus point length, or an end mill under ⌀6.55 with flutes of at least 12 mm");
    let entry = plan["tools_needed"].as_array().unwrap().iter().find(|n| n["description"] == need).unwrap();
    assert!(entry["features"].as_array().unwrap().len() > 1, "{entry}");
    assert_eq!(out["log"].as_array().unwrap().iter().filter(|l| l.as_str().unwrap().starts_with("tools needed: ")).count(), 1);
}

#[test]
fn non_tool_refusals_and_stepdown_do_not_name_a_tool() {
    for reason in ["helical entry bound exceeded", "contour gouge Uncertain", "invalid pocket parameters: stepdown_mm 8 exceeds the cutting length 2"] {
        let mut plan = simple(json!([{"kind":"holes", "diameter":8, "top_z":0, "bore_top_z":0, "bottom_z":-10, "bottom":"blind", "centers":[], "features":["holes:0"]}]));
        capp::finish(&mut plan, &library(), &[json!({"operation":"top-1-holes", "message":reason})]).unwrap();
        assert_eq!(plan["skipped"][0]["reason"], format!("left out: {reason}"));
        assert_eq!(plan["tools_needed"], json!([]));
    }
}

#[test]
fn refused_roughing_passes_its_tool_query_to_covered_features() {
    let mut plan = simple(json!([{"kind":"roughing", "top_z":0, "floor_z":-131.7, "web_mm":0, "surface_error_mm":0, "features":[]}]));
    plan["covered_by_roughing"] = json!(["pockets:0", "pockets:1"]);
    capp::finish(&mut plan, &library(), &[json!({"operation":"top-1-roughing", "message":"invalid pocket parameters: pocket depth 131.7 exceeds the cutting length 75", "reach":{"limit":"cutting_length","depth_needed_mm":131.7,"depth_available_mm":75.,"top_z_mm":0.}})]).unwrap();
    assert_eq!(plan["tools_needed"], json!([{"description":"an end mill with flutes of at least 132 mm", "features":["pockets:0", "pockets:1"], "operations":["top-1-roughing"]} ]));
    assert_eq!(plan["skipped"][0]["reason"], "no operation of its own, and roughing was refused: needs an end mill with flutes of at least 132 mm");
}

#[test]
fn v_channel_and_chamfer_reach_refusals_name_suitable_bits() {
    for (op, operation, reason, expected) in [
        (json!({"kind":"v_channel", "angle":60, "width":2, "depth":20.1, "length":20, "a":[-10,0], "b":[10,0], "top_z":0, "root_z":-20.1, "on_flat":false, "features":["v:0"]}),
         "top-1-v_channel", "invalid trace parameters: depth 20.1 below the stock top exceeds the cutting length 5",
         "a sharp V-bit with zero tip diameter and angle at most 60.00° with flutes of at least 21 mm"),
        (json!({"kind":"edge_chamfer", "width":0.5, "radius":1, "top_z":-2, "outer":[], "holes":[], "boundaries":"outer", "features":["r:0"]}),
         "top-1-edge_chamfer", "invalid chamfer parameters: tool tip Z-3 is deeper below the stock top than the cutting length 2",
         "a 90° chamfer mill with (diameter - tip diameter) / 2 at least 1.00 mm, with zero tip diameter and flutes of at least 4 mm"),
    ] {
        let mut plan = simple(json!([op]));
        capp::finish(&mut plan, &library(), &[json!({"operation":operation, "message":reason, "reach":{"limit":"cutting_length","depth_needed_mm":if operation.ends_with("v_channel") {20.1} else {3.},"top_z_mm":0.}})]).unwrap();
        assert_eq!(plan["tools_needed"][0]["description"], expected);
        assert!(plan["skipped"][0]["reason"].as_str().unwrap().ends_with(&format!(": needs {expected}")));
    }
}

#[test]
fn zero_drill_tolerance_requests_an_exact_size_instead_of_an_empty_rounded_range() {
    let mut tools = library();
    tools["drill_tolerance_mm"] = json!(0);
    let lib = capp::resources::Library::new(&tools).unwrap();
    assert_eq!(lib.needed("drill", &[("diameter", 4.211), ("depth", 10.)]).unwrap(), "a drill of ⌀4.211 with flutes of at least 10.01 mm plus point length");
}

#[test]
fn decimal_tool_bounds_ignore_binary_noise_without_losing_helix_clearance() {
    let mut tools = library();
    for (tolerance, expected) in [
        (0.05, "a drill of ⌀5.05–5.10"),
        (0.01, "a drill of ⌀5.09–5.10"),
        (0.0, "a drill of ⌀5.10–5.10"),
    ] {
        tools["drill_tolerance_mm"] = json!(tolerance);
        let lib = capp::resources::Library::new(&tools).unwrap();
        assert_eq!(lib.needed("drill", &[("diameter", 5.1), ("depth", 10.)]).unwrap(), format!("{expected} with flutes of at least 10.01 mm plus point length"));
    }
    let lib = capp::resources::Library::new(&tools).unwrap();
    // The bore selector reserves 0.05 mm for helix room: 8.25 - 0.05 = 8.20.
    assert_eq!(lib.needed("bore_mill", &[("diameter", 8.25)]).unwrap(), "an end mill under ⌀8.20");
}

#[test]
fn decimal_bound_formatting_preserves_hundredths_and_rounds_real_fractions_outward() {
    for value in [0.0, 5.1, 8.25, 4.1, 8.2, 10.2, 2.05, -2.05] {
        let expected = format!("{value:.2}/{value:.2}");
        assert_eq!(capp::rules::fill("{x:up2}/{x:down2}", &Value::Null, &|_| Ok(value)).unwrap(), expected);
    }
    for (value, expected) in [(5.101, "5.11/5.10"), (5.099, "5.10/5.09"), (-2.051, "-2.05/-2.06")] {
        assert_eq!(capp::rules::fill("{x:up2}/{x:down2}", &Value::Null, &|_| Ok(value)).unwrap(), expected);
    }
}

/// What a finished plan says about needed tools and considered candidates, as fields: every
/// decision whose text ends with `: needs X` has `needs` X (and only those have `needs`), every
/// entry of `tools_needed` is some decision's `needs`, and a considered candidate has
/// `qualifies` exactly when its text says it qualifies. Returns how many decisions name a need.
fn needs_as_fields(plan: &Value) -> usize {
    let decisions = plan["decisions"].as_array().unwrap();
    let mut named = 0;
    for d in decisions {
        let suffix = d["says"].as_str().and_then(|says| says.rsplit_once(": needs ")).map(|(_, need)| need);
        assert_eq!(d["needs"].as_str(), suffix, "{d}");
        named += usize::from(suffix.is_some());
        for c in d["considered"].as_array().into_iter().flatten().filter(|c| c.get("rejected").is_some()) {
            assert_eq!(c.get("qualifies").is_some(), c["rejected"].as_str().unwrap().starts_with("qualifies, but"), "{c}");
            assert!(c.get("qualifies").is_none_or(|q| q == true), "{c}");
        }
    }
    for need in plan["tools_needed"].as_array().unwrap() {
        assert!(decisions.iter().any(|d| d["needs"] == need["description"]), "{need}");
    }
    named
}

#[test]
fn needs_suffixes_are_decision_fields() {
    let parts = ["plate", "mounting-plate", "synthetic/stepped-plate", "synthetic/stepped-plate-chamfered", "synthetic/stepped-plate-chamfer-mouths",
        "synthetic/stepped-plate-large-chamfer", "synthetic/tall-wall-pocket", "synthetic/square-slots"];
    let libraries = ["../../demo/tools.json", "../../libraries/haas-vf2.json", "../../libraries/linuxcnc-vmc.json", "../../tests/fixtures/placeholder-tools.json"];
    let (mut plans, mut named, mut refused) = (0, 0, 0);
    for part in parts {
        let analysis = fixture(&format!("{part}.analysis.json"));
        let mesh = fixture(&format!("{part}.mesh.json"));
        let planned = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
        for path in libraries {
            let library = fixture(path);
            let mut done = planned.clone();
            capp::finish(&mut done, &library, &[]).unwrap();
            named += needs_as_fields(&done);
            plans += 1;
            // Each operation refused: by the engine in words that name a need, and for reach.
            for setup in done["setups"].as_array().unwrap() {
                let top = setup["stock"]["max"][2].as_f64().unwrap();
                for (i, op) in setup["operations"].as_array().unwrap().iter().enumerate() {
                    let id = format!("{}-{}-{}", setup["id"].as_str().unwrap(), i + 1, op["kind"].as_str().unwrap());
                    for exclusion in [
                        json!({"operation": id, "message": "refused: the holder hits the stock: needs an end mill of ⌀6 with its holder at least 40 mm above the tip"}),
                        json!({"operation": id, "message": "depth exceeds the cutting length", "reach": {"limit": "cutting_length", "depth_needed_mm": 12., "depth_available_mm": 5., "top_z_mm": top}}),
                        json!({"operation": id, "message": "holder collides", "reach": {"limit": "holder", "holder_section": 0, "depth_needed_mm": 12., "top_z_mm": top - 1., "z_mm": top - 12.}}),
                    ] {
                        let mut plan = planned.clone();
                        if capp::finish(&mut plan, &library, &[exclusion]).is_err() { continue; }
                        named += needs_as_fields(&plan);
                        plans += 1;
                        refused += usize::from(plan["decisions"].as_array().unwrap().iter().any(|d| d["rule"].as_str().unwrap().starts_with("execution.refused@") && d.get("needs").is_some()));
                    }
                }
            }
        }
    }
    // 896, 4,301 and 864 when this was written.
    assert!(plans > 800 && named > 3000 && refused > 700, "{plans} plans, {named} decisions naming a need, {refused} refusals with a need");
}
