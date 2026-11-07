//! Which tool cuts what, and the tool library: what tests/test_step2cam.py checked in its `Post` and
//! `Library` classes, under the same names, on the camlib manifest instead of a stand-in's recorded
//! calls. The thread tests are in threads.rs.
mod common;
use common::*;
use serde_json::{Value, json};
use step2cam::job::{Library, jobs};
use step2cam::plan::{Options, plan};

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
fn of_kind<'a>(ops: &'a [Value], kind: &str) -> Vec<&'a Value> {
    ops.iter().filter(|o| o["kind"] == kind).collect()
}

#[test]
fn plate_drills_what_has_a_drill_and_bores_the_rest() {
    let plan = plan(&plate(), Some(&fixture("plate.mesh.json")), &Default::default()).unwrap();
    let (ops, tools, notes) = posted(&plan, &example_tools());
    let kinds: Vec<&str> = ops.iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["facing", "roughing", "drilling", "drilling", "bore"]);
    let [facing, rough, d42, d66, bore] = &ops[..] else { unreachable!() };
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
    assert_eq!(number(&d66["peck_mm"]), 4.);
    // No 8 mm drill: helical bore with the largest end mill that fits, to the stock bottom,
    // which it cannot pass: a skin is expected and said.
    assert_same(json!([bore["tool"], bore["diameter_mm"], bore["floor_z_mm"]]), json!(["em6", 8.0, -11]));
    assert!(any_note(&notes, "skin or burr"), "{notes:?}");
    let mut ids: Vec<&str> = tools.iter().map(|t| t["id"].as_str().unwrap()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["drill4.2", "drill6.6", "em6"]);
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
    assert_same(json!([ops[0]["depth_reference"], ops[0]["depth_z_mm"]]), json!(["tip", -6]));
    assert_eq!(number(&ops[0]["allow_breakthrough_mm"]), 0.);
    assert!(notes.iter().any(|n| n.contains("full diameter ends") && n.contains("short")), "{notes:?}");
}

#[test]
fn drill_point_holes_reach_full_diameter_at_the_models_depth() {
    // quiddity's depth stops where the cone starts.
    let (ops, _, notes) = synthetic_post(&[down(0., 0., 4.2, 6., "drill_point")]);
    assert_same(json!([ops[0]["depth_reference"], ops[0]["depth_z_mm"]]), json!(["full_diameter", -6]));
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
    assert_eq!(ops.len(), 2, "{ops:?}");
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
fn only_table_tool_changes_are_posted() {
    let mut data = example_tools();
    data["machine"]["tool_change_position_mode"] = json!("z_home");
    let Err(error) = jobs(&plate_no_mesh(&Default::default()), &data) else { panic!("accepted") };
    assert!(error.contains("table"), "{error}");
}
