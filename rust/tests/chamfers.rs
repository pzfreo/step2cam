//! Modelled chamfers and camlib's full-cone admission requirement (issue #47).
use camlib::{chamfer::ChamferPlan, pocket::Operation, stream::StreamManifest};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};

fn read(name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(name)).unwrap()).unwrap()
}
fn run(library: &Value) -> Value {
    step2cam::process(
        &read("tests/fixtures/synthetic/stepped-plate-large-chamfer.analysis.json"),
        &read("tests/fixtures/synthetic/stepped-plate-large-chamfer.mesh.json"),
        library,
    )
    .unwrap()
}
fn chamfers(out: &Value) -> Vec<&Value> {
    out["plan"]["setups"].as_array().unwrap().iter().flat_map(|s| s["operations"].as_array().unwrap()).filter(|o| o["kind"] == "edge_chamfer").collect()
}

#[test]
fn width_passes_split_load_but_do_not_extend_the_cone() {
    let fixture: StreamManifest = serde_json::from_str(include_str!("../../vendor/camlib/fixtures/operations/chamfer.json")).unwrap();
    let Operation::Chamfer(mut op) = fixture.operations[1].clone() else { panic!("chamfer fixture") };
    // A simple 30 mm square avoids width-erasure limits of the fixture's circular rings.
    op.region = serde_json::from_value(json!({"version":1,"units":"mm","name":"square","boundary_error_mm":0.,"source":null,
        "components":[{"outer":{"vertices":[[5,5,0],[35,5,0],[35,35,0],[5,35,0]]},"holes":[]}]}))
    .unwrap();
    op.width_mm = 5.;
    let cancel = AtomicBool::new(false);
    for step in [None, Some(1.), Some(0.25)] {
        op.width_step_mm = step;
        let error =
            ChamferPlan::prepare(op.clone(), &fixture.tools[1], fixture.stock.height().unwrap(), fixture.initial_position_mm, 1, &cancel).err().unwrap();
        assert_eq!(error, "chamfer width exceeds conical cutter radius");
    }
    // A lower local top still uses the job's stock top for cutting-length admission.
    let mut local = op.clone();
    local.top_z_mm = 3.;
    local.width_mm = 1.;
    let mut short = fixture.tools[1].clone();
    short.cutter.cutting_length_mm = 4.;
    let error = ChamferPlan::prepare(local, &short, fixture.stock.height().unwrap(), fixture.initial_position_mm, 1, &cancel).err().unwrap();
    assert!(error.contains("deeper below the stock top than the cutting length 4"), "{error}");
    let mut tool = fixture.tools[1].clone();
    tool.cutter.diameter_mm = 16.;
    op.width_step_mm = Some(1.);
    let plan = ChamferPlan::prepare(op, &tool, fixture.stock.height().unwrap(), fixture.initial_position_mm, 1, &cancel).unwrap();
    assert_eq!(plan.report().width_passes, 5);
    assert!((plan.report().axial_bevel_depth_mm - 5.).abs() < 1e-9);
}

#[test]
fn a_large_chamfer_names_the_missing_tool() {
    let out = run(&read("demo/tools.json"));
    assert!(out["error"].is_null(), "{}", out["error"]);
    let ops = chamfers(&out);
    assert_eq!(ops.len(), 1, "{}", out["plan"]);
    assert!(ops[0]["tool"].is_null());
    assert!(ops[0]["tool_note"].as_str().unwrap().contains(": needs a 90° chamfer mill with (diameter - tip diameter) / 2 at least 5.50 mm"), "{}", ops[0]);
    assert!(out["plan"]["tools_needed"].to_string().contains("5.50 mm"));
}

#[test]
fn the_largest_covering_mill_cuts_a_large_chamfer_in_passes_without_gouging() {
    let mut library = read("demo/tools.json");
    library["machines"] = json!([]); // These test tools exceed the demo machine's collet.
    let base = library["tools"].as_array().unwrap().iter().find(|t| t["id"] == "cm6").unwrap().clone();
    for (id, diameter, number) in [("cm12", 12., 12), ("cm16", 16., 16), ("cm14", 14., 14)] {
        let mut tool = base.clone();
        tool["id"] = json!(id);
        tool["number"] = json!(number);
        tool["diameter"] = json!(diameter);
        tool["flute_length"] = json!(25.);
        tool["stickout"] = json!(30.);
        library["tools"].as_array_mut().unwrap().push(tool);
    }
    let mut short = base.clone();
    short["id"] = json!("cm18-short");
    short["number"] = json!(18);
    short["diameter"] = json!(18.);
    library["tools"].as_array_mut().unwrap().push(short);
    let selector = capp::resources::Library::new(&library).unwrap();
    assert_eq!(selector.chamfer_mill_in(5., 14., 6.5, f64::INFINITY).unwrap()["id"], "cm14", "the largest within loop clearance");
    assert_eq!(selector.chamfer_mill_in(5., f64::INFINITY, 6.5, 7.).unwrap()["id"], "cm12", "the cone and axis offset clear neighboring material");
    let out = run(&library);
    assert!(out["error"].is_null(), "{}", out["error"]);
    let ops = chamfers(&out);
    assert_eq!(ops.len(), 1, "{}", out["plan"]["skipped"]);
    assert_eq!(ops[0]["tool"], "cm16");
    let program = &out["programs"][0];
    let report = program["operations"].as_array().unwrap().iter().find(|o| o["id"].as_str().unwrap().ends_with("edge_chamfer")).unwrap();
    assert!(report["end"].as_u64().unwrap() > report["start"].as_u64().unwrap(), "{report}");
    let job = program["job"]["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "chamfer").unwrap();
    assert_eq!(job["width_mm"], 5.0);
    assert_eq!(job["width_step_mm"], 1.0);
    assert_eq!(program["material"]["gouge"]["definite_cells"], 0, "{}", program["material"]);
}

#[test]
fn slot_and_recognised_hole_mouths_are_cut_at_size() {
    let mut analysis = read("tests/fixtures/synthetic/stepped-plate-chamfer-mouths.analysis.json");
    let mesh = read("tests/fixtures/synthetic/stepped-plate-chamfer-mouths.mesh.json");
    // This specify-core release reports the slot's planar chamfers but omits the hole's
    // cone. Supply the same turned-chamfer record it uses for external rounded corners;
    // the frozen STEP and mesh still provide the actual geometry being cut and checked.
    let cone = analysis["faces"].as_array().unwrap().iter().find(|f| f["kind"] == "cone" && f["centroid"][0].as_f64().unwrap() < 0.).unwrap().clone();
    analysis["features"].as_array_mut().unwrap().push(json!({"id":"chamfers:hole-mouth", "family":"chamfers",
        "faces":[cone["id"]], "record":{"axis":"z","leg1":1.,"leg2":1.,"angle":45.,"turned":true,"at":cone["centroid"]}}));
    // Describe the bore from its actual top mouth, including the bevel's empty band.
    // The frozen recogniser instead starts at the cone's foot and chooses bottom entry.
    let hole = analysis["features"].as_array_mut().unwrap().iter_mut().find(|f| f["family"] == "holes").unwrap();
    hole["record"]["location"][2] = json!(12.);
    hole["record"]["depth"] = json!(12.);
    let out = step2cam::process(&analysis, &mesh, &read("demo/tools.json")).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let ops = chamfers(&out);
    assert_eq!(ops.len(), 2, "{}", out["plan"]["skipped"]);
    assert!(ops.iter().all(|o| o["tool"] == "cm6" && o["boundaries"] == "holes"));
    assert!(ops.iter().any(|o| o["features"].to_string().contains("chamfers:hole-mouth")));
    assert_eq!(out["programs"][0]["material"]["gouge"]["definite_cells"], 0);
}

#[test]
fn the_existing_chamfered_plate_cuts_with_sufficient_flute_reach() {
    let mut library = read("demo/tools.json");
    let cm6 = library["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == "cm6").unwrap();
    cm6["flute_length"] = json!(10.);
    let out = step2cam::process(
        &read("tests/fixtures/synthetic/stepped-plate-chamfered.analysis.json"),
        &read("tests/fixtures/synthetic/stepped-plate-chamfered.mesh.json"),
        &library,
    )
    .unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let ops = chamfers(&out);
    assert_eq!(ops.len(), 1, "{}", out["plan"]["skipped"]);
    assert_eq!(ops[0]["tool"], "cm6");
    assert_eq!(ops[0]["features"].as_array().unwrap().len(), 12);
    assert_eq!(out["programs"][0]["material"]["gouge"]["definite_cells"], 0);
}

#[test]
fn a_geometry_refusal_also_reports_a_missing_cone_and_refreshes_with_the_library() {
    let analysis = read("tests/fixtures/synthetic/stepped-plate-large-chamfer.analysis.json");
    let mut plan = capp::plan::plan(&analysis, None, &Default::default()).unwrap();
    let mut library = read("demo/tools.json");
    capp::finish(&mut plan, &library, &[]).unwrap();
    let reason =
        plan["skipped"].as_array().unwrap().iter().find(|s| s["feature"].as_str().unwrap().starts_with("chamfers:")).unwrap()["reason"].as_str().unwrap();
    assert!(reason.contains("no mesh") && reason.contains(": needs a 90° chamfer mill") && reason.contains("5.50 mm"), "{reason}");
    library["machines"] = json!([]);
    let cm = library["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == "cm6").unwrap();
    cm["diameter"] = json!(16.);
    cm["flute_length"] = json!(10.);
    capp::finish(&mut plan, &library, &[]).unwrap();
    assert!(!plan["skipped"].to_string().contains(": needs "));
    assert!(plan["skipped"].to_string().contains("no mesh"));
    assert!(plan["tools_needed"].as_array().unwrap().is_empty());
}
