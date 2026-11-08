//! Which tool cuts what, and the tool library: what tests/test_step2cam.py checked in its `Post` and
//! `Library` classes, under the same names, on the camlib manifest instead of a stand-in's recorded
//! calls. The thread tests are in threads.rs.
mod common;
use common::*;
use serde_json::{Value, json};
use capp::resources::Library;
use capp::plan::{Options, plan};

fn tip(diameter: f64) -> f64 {
    diameter / 2. / 59f64.to_radians().tan()
}
fn plate() -> Value {
    fixture("plate.analysis.json")
}
fn plate_no_mesh(options: &Options) -> Value {
    plan(&plate(), None, options).unwrap()
}
fn synthetic_post(holes: &[Value]) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    posted(&plan(&holes_only(holes), None, &top_stock(0.)).unwrap(), &example_tools())
}
// These drilling assertions concern the twist drills; spot cycles are checked separately.
fn of_kind<'a>(ops: &'a [Value], kind: &str) -> Vec<&'a Value> {
    ops.iter().filter(|o| o["kind"] == kind && (kind != "drilling" || !o["id"].as_str().unwrap().contains("-spot_drill-"))).collect()
}

#[test]
fn plate_drills_what_has_a_drill_and_bores_the_rest() {
    let plan = plan(&plate(), Some(&fixture("plate.mesh.json")), &Default::default()).unwrap();
    let (ops, tools, notes) = posted(&plan, &example_tools());
    let kinds: Vec<&str> = ops.iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["facing", "roughing", "drilling", "drilling", "drilling", "drilling", "bore", "contour"]);
    let [facing, rough, spot42, spot66, d42, d66, bore, _wall] = &ops[..] else { unreachable!() };
    assert_eq!(spot42["tool"], "spot8");
    assert_eq!(spot66["tool"], "spot8");
    assert_eq!(facing["tool"], "em6");
    assert_eq!(number(&facing["floor_z_mm"]), -1.);
    let mesh = &rough["target"];
    assert_eq!(list(&mesh["indices"]).len(), 1312); // triangles
    // The allowance covers the mesh's own error twice over.
    assert!(number(&rough["radial_allowance_mm"]) >= 2. * number(&mesh["surface_error_mm"]));
    assert_eq!(d42["tool"], "drill4.2");
    assert_eq!(d66["tool"], "drill6.6");
    assert_eq!(list(&d66["centers_mm"]).len(), 6);
    // Through, onto the part's face: full diameter 1 mm past it, the tip below the stock
    // bottom (-11) allowed and said.
    assert_eq!(d66["depth_reference"], "full_diameter");
    assert_eq!(number(&d66["depth_z_mm"]), -12.);
    assert_close(number(&d66["allow_breakthrough_mm"]), 1. + tip(6.6) + 0.01);
    assert!(any_note(&notes, "below the stock bottom"), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("web") && n.contains("vise")), "{notes:?}");
    assert!(d66["peck_mm"].is_null(), "under 3D: one plunge despite the tool cap");
    // No 8 mm drill: helical bore with the largest end mill that fits, to the stock bottom,
    // which it cannot pass: a skin is expected and said.
    assert_same(json!([bore["tool"], bore["diameter_mm"], bore["floor_z_mm"]]), json!(["em6", 8.0, -11]));
    assert!(any_note(&notes, "skin or burr"), "{notes:?}");
    let mut ids: Vec<&str> = tools.iter().map(|t| t["id"].as_str().unwrap()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["drill4.2", "drill6.6", "em6", "spot8"]);
    for tool in &tools {
        assert!(number(&tool["gauge_length_mm"]) > number(&tool["stickout_mm"]), "{}", tool["id"]);
    }
}

#[test]
fn stock_below_lets_a_bore_break_through() {
    let plan = plate_no_mesh(&Options { bottom_stock: 2., ..Default::default() });
    let (ops, _, notes) = posted(&plan, &example_tools());
    assert_eq!(number(&of_kind(&ops, "bore")[0]["floor_z_mm"]), -12.);
    assert!(!any_note(&notes, "skin or burr"), "{notes:?}");
}

#[test]
fn blind_flat_holes_stop_at_the_bottom_with_the_tip() {
    let (ops, _, notes) = synthetic_post(&[down(0., 0., 4.2, 6., "flat")]);
    assert_same(json!([ops[1]["depth_reference"], ops[1]["depth_z_mm"]]), json!(["tip", -6]));
    assert_eq!(number(&ops[1]["allow_breakthrough_mm"]), 0.);
    assert!(notes.iter().any(|n| n.contains("full diameter ends") && n.contains("short")), "{notes:?}");
}

#[test]
fn drill_point_holes_reach_full_diameter_at_the_models_depth() {
    // quiddity's depth stops where the cone starts.
    let (ops, _, notes) = synthetic_post(&[down(0., 0., 4.2, 6., "drill_point")]);
    assert_same(json!([ops[1]["depth_reference"], ops[1]["depth_z_mm"]]), json!(["full_diameter", -6]));
    assert!(any_note(&notes, "drill-point"), "{notes:?}");
}

#[test]
fn part_material_beyond_the_exit_limits_the_drill() {
    // The far wall of a cavity 2 mm below the exit (EXIT_MARGIN 0.2 kept from it).
    let a = holes_only(&[down(0., 0., 6.6, 6., "through"), down(10., 0., 4.2, 6., "through")]);
    let mesh = mesh_of(&square(-8., 15., 5., 0.), BOX);
    let (ops, _, notes) = posted(&plan(&a, Some(&mesh), &top_stock(0.)).unwrap(), &example_tools());
    let drills = of_kind(&ops, "drilling");
    let by_tool = |id: &str| *drills.iter().find(|o| o["tool"] == id).unwrap();
    // ⌀4.2: its tip fits in the 1.8 mm, so full diameter goes 1.8 - tip past the exit.
    let d42 = by_tool("drill4.2");
    assert_eq!(d42["depth_reference"], "full_diameter");
    assert_close(number(&d42["depth_z_mm"]), -6. - (1.8 - tip(4.2)));
    assert_close(number(&d42["depth_z_mm"]) - tip(4.2), -7.8);
    // ⌀6.6: its tip is longer than the room: tip at -7.8, full diameter never reaches the
    // exit, and the note says so.
    let d66 = by_tool("drill6.6");
    assert_same(json!([d66["depth_reference"], d66["depth_z_mm"]]), json!(["tip", -7.8]));
    assert!(any_note(&notes, "cannot reach full diameter"), "{notes:?}");
    for op in drills {
        assert_eq!(number(&op["allow_breakthrough_mm"]), 0.);
    }
}

#[test]
fn a_hole_no_tool_fits_is_a_note_not_a_guess() {
    let (ops, _, notes) = synthetic_post(&[down(0., 0., 2.5, 6., "through")]);
    assert!(ops.is_empty(), "{ops:?}");
    assert!(any_note(&notes, "no drill of ⌀2.5"), "{notes:?}");
}

#[test]
fn a_drill_is_never_larger_than_the_hole() {
    let (ops, _, _) = synthetic_post(&[down(0., 0., 6.58, 10., "through"), down(9., 0., 6.64, 10., "through")]);
    // 6.58: bored; 6.64: the 6.6 drill.
    assert_eq!(ops.len(), 3, "{ops:?}");
    let bore = of_kind(&ops, "bore");
    let drill = of_kind(&ops, "drilling");
    assert_eq!((bore.len(), drill.len()), (1, 1), "{ops:?}");
    assert_eq!(number(&bore[0]["diameter_mm"]), 6.58);
    assert_eq!(drill[0]["tool"], "drill6.6");
}

#[test]
fn counterbores_are_bored_with_an_end_mill() {
    let (ops, _, _) = synthetic_post(&[with(down(0., 0., 6.6, 6.5, "through"), "cbore", json!({"diameter": 11, "depth": 3.5}))]);
    let cbore = of_kind(&ops, "bore")[0];
    assert_same(json!([cbore["diameter_mm"], cbore["floor_z_mm"]]), json!([11, -3.5]));
}

#[test]
fn holder_and_cutting_data_have_no_defaults() {
    for key in ["holder_diameter", "holder_length", "coolant", "point_angle", "plunge_mm_min"] {
        let mut data = example_tools();
        for tool in data["tools"].as_array_mut().unwrap() {
            tool.as_object_mut().unwrap().remove(key);
        }
        let Err(error) = Library::new(&data) else { panic!("accepted without {key}") };
        assert!(error.contains(key), "{key}: {error}");
    }
}

#[test]
fn a_holder_narrower_than_the_shank_is_refused() {
    let mut data = example_tools();
    data["tools"][0]["holder_diameter"] = json!(3);
    assert!(Library::new(&data).is_err());
}

#[test]
fn drilling_cycles_follow_depth_and_positive_library_caps() {
    // Test the emitted camlib options, not just the reference table helper.
    let mut library = example_tools();
    for tool in library["tools"].as_array_mut().unwrap().iter_mut().filter(|t| t["type"] == "drill") {
        tool["flute_length"] = json!(60);
        tool["overall_length"] = json!(100);
        tool["stickout"] = json!(70);
    }
    for (depth, cap, expected) in [
        (12.6, Some(0.1), None), // exactly 3D: cap cannot force pecking
        (16.8, None, Some(2.1)), // 4D: rule increment, no library cap
        (25.2, Some(10.), Some(2.1)), // 6D: full retract, rule caps the tool
        (25.2, Some(1.), Some(1.)), // smaller library increment is preserved
        (25.2, Some(0.), Some(2.1)), // zero means no cap
    ] {
        let tool = library["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == "drill4.2").unwrap();
        match cap {
            Some(v) => { tool["peck_mm"] = json!(v); }
            None => { tool.as_object_mut().unwrap().remove("peck_mm"); }
        }
        let plan = plan(&holes_only(&[down(0., 0., 4.2, depth, "flat")]), None, &top_stock(0.)).unwrap();
        let (ops, _, _) = posted(&plan, &library);
        let drilling = of_kind(&ops, "drilling");
        assert_eq!(drilling.len(), 1, "{ops:?}");
        assert_eq!(drilling[0]["peck_mm"], json!(expected), "depth {depth}, cap {cap:?}");
        assert_eq!(drilling[0]["peck_mode"], "full_retract");
    }
}

#[test]
fn hole_tool_selection_checks_stock_breakthrough_and_point_before_the_job() {
    let plan = plan(&holes_only(&[down(0., 0., 4.2, 10., "through")]), None, &top_stock(2.)).unwrap();
    let mut library = example_tools();
    let tools = library["tools"].as_array_mut().unwrap();
    let short = tools.iter_mut().find(|t| t["id"] == "drill4.2").unwrap();
    short["flute_length"] = json!(13.); // stock 2 + hole 10 + breakthrough 1; point still missing
    let mut long = short.clone();
    long["id"] = json!("long4.2");
    long["flute_length"] = json!(20.);
    tools.push(long);
    let settled = finished(&plan, &library);
    let hole = settled["setups"][0]["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "holes").unwrap();
    assert_eq!(hole["tool"], "long4.2");
    assert!(settled["decisions"].as_array().unwrap().iter().any(|d| d["considered"].as_array().is_some_and(|cs| cs.iter().any(|c| c["candidate"] == "drill4.2" && c["rejected"].as_str().is_some_and(|s| s.contains("flutes do not reach"))))));
    let (ops, _, _) = posted(&plan, &library);
    assert_eq!(of_kind(&ops, "drilling")[0]["tool"], "long4.2");
    library["tools"].as_array_mut().unwrap().retain(|t| t["id"] != "long4.2");
    let (ops, _, _) = posted(&plan, &library);
    assert!(of_kind(&ops, "drilling").is_empty());
    assert_eq!(of_kind(&ops, "bore").len(), 1);
}

#[test]
fn a_short_drill_is_deferred_only_when_no_end_mill_can_bore() {
    let plan = plan(&holes_only(&[down(0., 0., 4.2, 10., "through")]), None, &top_stock(2.)).unwrap();
    let mut library = example_tools();
    library["tools"].as_array_mut().unwrap().retain(|t| t["id"] == "drill4.2");
    library["tools"][0]["flute_length"] = json!(13.);
    let settled = finished(&plan, &library);
    let hole = settled["setups"][0]["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "holes").unwrap();
    assert_eq!(hole["tool"], "drill4.2");
    assert!(hole["tool_note"].is_null());
    assert!(settled["decisions"].as_array().unwrap().iter().any(|d| d["says"].as_str().is_some_and(|s| s.contains("reach is left to camlib"))));
    let (ops, _, _) = posted(&plan, &library);
    assert_eq!(of_kind(&ops, "drilling")[0]["tool"], "drill4.2");
}

#[test]
fn deep_drilled_holes_warn_only_above_five_diameters_and_link_the_trace() {
    let mut library = example_tools();
    let drill = library["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == "drill4.2").unwrap();
    drill["flute_length"] = json!(60);
    drill["overall_length"] = json!(100);
    drill["stickout"] = json!(70);
    for depth in [20.99, 21., 21.01, 25.2] {
        let original = plan(&holes_only(&[down(0., 0., 4.2, depth, "flat")]), None, &top_stock(0.)).unwrap();
        let plan = finished(&original, &library);
        let warnings: Vec<_> = list(&plan["warnings"]).iter().filter_map(Value::as_str)
            .filter(|s| s.contains("deep-hole drilling")).collect();
        if depth <= 21. {
            assert!(warnings.is_empty(), "depth {depth}: {warnings:?}");
        } else {
            assert_eq!(warnings.len(), 1, "depth {depth}: {warnings:?}");
            let warning = warnings[0];
            assert!(warning.contains("4.2 mm holes"), "{warning}");
            assert!(warning.contains(&format!("depth/D {:.1}", depth / 4.2)), "{warning}");
            assert!(warning.contains("deep-hole drilling (pilot plus deep-hole drill) is not planned"), "{warning}");
            assert!(warning.contains("drilled with full-retract pecks"), "{warning}");
            let decision = list(&plan["decisions"]).iter().find(|d| d["outcome"] == "warning" && d["says"] == warning).unwrap();
            assert_eq!(decision["rule"], "process.hole.peck_by_depth@2");
            assert_eq!(decision["features"], json!(["holes:0"]));
            assert_close(number(&decision["depth_diameter_ratio"]), depth / 4.2);
            let op = list(&plan["setups"][0]["operations"]).iter().find(|o| o["kind"] == "holes").unwrap();
            assert!(list(&op["because"]).contains(&decision["id"]));
            assert_eq!(op["technology"]["peck_mode"], "full_retract");
            assert_eq!(op["technology"]["peck_mm"], 2.1);
        }
    }
}
