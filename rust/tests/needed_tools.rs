//! Missing-tool queries report resources without changing the plan's operations.
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures").join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn library() -> Value { fixture("../../demo/tools.json") }
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
        {"kind":"holes", "diameter":18.5, "bottom_z":-131.7, "features":["holes:0"]},
        {"kind":"holes", "diameter":18.5, "bottom_z":-131.7, "features":["holes:1"]}
    ]));
    capp::finish(&mut plan, &tools, &[]).unwrap();
    let need = "a drill of ⌀18.30–18.50 with flutes of at least 132 mm (plus point length when drilling to full diameter), or an end mill under ⌀18.45 with flutes of at least 132 mm";
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
    let mut plan = simple(json!([{"kind":"notch", "width":1.237, "top_z":0, "floor_z":-12.01, "features":["notches:0"]}]));
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
    let skipped: Vec<_> = plan["skipped"].as_array().unwrap().iter()
        .filter(|s| s["reason"].as_str().unwrap().contains("depth") && s["reason"].as_str().unwrap().contains("exceeds the cutting length")).collect();
    assert!(skipped.len() >= 8, "{}", plan["skipped"]);
    for skip in &skipped {
        let reason = skip["reason"].as_str().unwrap();
        let (_, need) = reason.rsplit_once(": needs ").expect("reach refusal needs a tool");
        assert!(reason.starts_with("left out: setup top: invalid "));
        assert!(reason.contains("exceeds the cutting length 2"));
        assert!(need.contains("with flutes of at least 1"), "{need}");
        let matches: Vec<_> = plan["tools_needed"].as_array().unwrap().iter().filter(|n| n["description"] == need).collect();
        assert_eq!(matches.len(), 1);
        assert!(matches[0]["features"].as_array().unwrap().contains(&skip["feature"]));
    }
    let group = skipped.iter().find(|s| s["feature"] == "holes:6").unwrap();
    let need = group["reason"].as_str().unwrap().rsplit_once(": needs ").unwrap().1;
    assert_eq!(need, "a drill of ⌀6.55–6.60 with flutes of at least 12 mm (plus point length when drilling to full diameter), or an end mill under ⌀6.55 with flutes of at least 12 mm");
    let entry = plan["tools_needed"].as_array().unwrap().iter().find(|n| n["description"] == need).unwrap();
    assert!(entry["features"].as_array().unwrap().len() > 1, "{entry}");
    assert_eq!(out["log"].as_array().unwrap().iter().filter(|l| l.as_str().unwrap().starts_with("tools needed: ")).count(), 1);
}

#[test]
fn non_tool_refusals_and_stepdown_do_not_name_a_tool() {
    for reason in ["helical entry bound exceeded", "contour gouge Uncertain", "invalid pocket parameters: stepdown_mm 8 exceeds the cutting length 2"] {
        let mut plan = simple(json!([{"kind":"holes", "diameter":8, "bottom_z":-10, "features":["holes:0"]}]));
        capp::finish(&mut plan, &library(), &[json!({"operation":"top-1-holes", "message":reason})]).unwrap();
        assert_eq!(plan["skipped"][0]["reason"], format!("left out: {reason}"));
        assert_eq!(plan["tools_needed"], json!([]));
    }
}

#[test]
fn refused_roughing_passes_its_tool_query_to_covered_features() {
    let mut plan = simple(json!([{"kind":"roughing", "top_z":0, "floor_z":-131.7, "features":[]}]));
    plan["covered_by_roughing"] = json!(["pockets:0", "pockets:1"]);
    capp::finish(&mut plan, &library(), &[json!({"operation":"top-1-roughing", "message":"invalid pocket parameters: pocket depth 131.7 exceeds the cutting length 75"})]).unwrap();
    assert_eq!(plan["tools_needed"], json!([{"description":"an end mill with flutes of at least 132 mm", "features":["pockets:0", "pockets:1"], "operations":["top-1-roughing"]} ]));
    assert_eq!(plan["skipped"][0]["reason"], "no operation of its own, and roughing was refused: needs an end mill with flutes of at least 132 mm");
}

#[test]
fn v_channel_and_chamfer_reach_refusals_name_suitable_bits() {
    for (op, operation, reason, expected) in [
        (json!({"kind":"v_channel", "angle":60, "width":2, "root_z":-20.1, "features":["v:0"]}),
         "top-1-v_channel", "invalid trace parameters: depth 20.1 below the stock top exceeds the cutting length 5",
         "a sharp V-bit with zero tip diameter and angle at most 60.00° with flutes of at least 21 mm"),
        (json!({"kind":"edge_chamfer", "width":0.5, "radius":1, "top_z":-2, "features":["r:0"]}),
         "top-1-edge_chamfer", "invalid chamfer parameters: tool tip Z-3 is deeper below the stock top than the cutting length 2",
         "a 90° chamfer mill with (diameter - tip diameter) / 2 at least 1.00 mm, with zero tip diameter and flutes of at least 4 mm"),
    ] {
        let mut plan = simple(json!([op]));
        capp::finish(&mut plan, &library(), &[json!({"operation":operation, "message":reason})]).unwrap();
        assert_eq!(plan["tools_needed"][0]["description"], expected);
        assert!(plan["skipped"][0]["reason"].as_str().unwrap().ends_with(&format!(": needs {expected}")));
    }
}
