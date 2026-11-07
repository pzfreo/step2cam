//! Finishing walls and flats: shapes an outline alone would cut wrongly, in tests/fixtures/synthetic/walls.
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use step2cam::post::{Machine, post};

fn fixture(path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
fn tools() -> Value {
    fixture("demo/tools.json")
}
fn machine() -> Machine {
    serde_json::from_value(tools()["machine"].clone()).unwrap()
}
fn part(name: &str) -> (Value, Value) {
    (fixture(&format!("tests/fixtures/synthetic/{name}.analysis.json")), fixture(&format!("tests/fixtures/synthetic/{name}.mesh.json")))
}
fn process(name: &str) -> Value {
    let (analysis, mesh) = part(name);
    let out = step2cam::process(&analysis, &mesh, &tools()).unwrap();
    assert!(out["post_error"].is_null(), "{name}: {}", out["post_error"]);
    out
}
fn walls(out: &Value) -> Vec<Value> {
    out["plan"]["setups"][0]["operations"].as_array().unwrap().iter().filter(|o| o["kind"] == "wall_finish").cloned().collect()
}
fn warnings(out: &Value) -> String {
    out["plan"]["warnings"].to_string()
}

/// A cap wider than the tool: its section encloses the stem's and looks like room.
#[test]
fn a_wall_under_an_overhang_is_left_alone_and_the_part_still_posts() {
    let out = process("walls/overhang");
    assert!(warnings(&out).contains("0 mm beside the wall is too narrow"), "{}", warnings(&out));
    assert_eq!(walls(&out).len(), 1, "only the plate's");
    assert_eq!(out["programs"][0]["material"]["gouge"]["definite_cells"], 0);
}

/// The island's vertices are all far from the pocket's long walls: only their spans are near.
#[test]
fn a_wall_too_close_to_another_is_measured_along_that_wall() {
    let out = process("walls/island");
    assert!(warnings(&out).contains("11.7 mm beside the wall is too narrow"), "{}", warnings(&out));
    assert_eq!(walls(&out).len(), 1, "only the block's");
}

/// The cutter fits, but roughing, with its allowance on both sides, does not get in.
#[test]
fn a_wall_needs_room_for_the_allowance_opposite_too() {
    let out = process("walls/channel");
    assert!(warnings(&out).contains("12.3 mm beside the wall is too narrow for em12 (it needs 12.6)"), "{}", warnings(&out));
}

/// A rib flush with the plate's edge is the plate's own wall, not something in its way.
#[test]
fn a_rib_flush_with_a_wall_does_not_stop_that_wall() {
    let out = process("walls/channel");
    let walls = walls(&out);
    assert_eq!(walls.len(), 1);
    assert!(walls[0]["room"].is_null(), "{}", walls[0]);
}

#[test]
fn a_round_boss_is_finished_as_a_full_circle() {
    let out = process("walls/round-boss");
    assert!(!warnings(&out).contains("bulge"), "{}", warnings(&out));
    assert_eq!(walls(&out).len(), 2);
    let circle = walls(&out).into_iter().find(|w| w["on_flat"] == true).unwrap();
    assert!(circle["outer"].as_array().unwrap().iter().all(|v| v[2].as_f64().unwrap().abs() <= 1.));
    assert!(out["programs"][0]["nc"].as_str().unwrap().contains("G2 "), "climb milled, clockwise round the outside");
}

/// With no roughing a contour would slot through solid stock, unchecked.
#[test]
fn with_roughing_refused_no_wall_or_flat_is_cut() {
    let mut library = tools();
    for tool in library["tools"].as_array_mut().unwrap() {
        if tool["type"] == "end_mill" {
            tool["flute_length"] = 1.into();
            tool["stickout"] = 1.5.into();
        }
    }
    let (analysis, mesh) = part("stepped-plate");
    let out = step2cam::process(&analysis, &mesh, &library).unwrap();
    assert!(warnings(&out).contains("roughing left out"), "{}", warnings(&out));
    let kinds = out["plan"]["setups"][0]["operations"].to_string();
    assert!(!kinds.contains("wall_finish") && !kinds.contains("flats"), "{kinds}");
    assert!(warnings(&out).contains("no roughing has cleared the stock"), "{}", warnings(&out));
}

/// camlib proves a flat clear of the part, not how deep it goes. Where roughing did not reach, it
/// would take the whole thickness in one pass at finishing feed.
#[test]
fn a_flat_where_roughing_did_not_reach_is_refused() {
    let (analysis, mesh) = part("stepped-plate");
    let plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&plan, &tools()).unwrap();
    let mut manifest = jobs[0].manifest.clone();
    for op in manifest["operations"].as_array_mut().unwrap() {
        if op["kind"] == "roughing" {
            op["floor_z_mm"] = (-1.5).into();
        }
    }
    let Err(failure) = post(&manifest.to_string(), machine()) else { panic!("a flat through uncut stock passed") };
    assert!(failure.message.contains("a flat would cut"), "{}", failure.message);
    assert!(failure.operation.as_deref().is_some_and(|id| id.ends_with("flats")), "{:?}", failure.operation);
}

#[test]
fn a_flat_on_a_roughed_part_stays_within_its_limit_and_enters_from_the_retract_height() {
    let out = process("stepped-plate");
    assert!(out["programs"][0]["operations"].to_string().contains("flats"));
    let (analysis, mesh) = part("stepped-plate");
    let plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&plan, &tools()).unwrap();
    let flat = jobs[0].manifest["operations"].as_array().unwrap().iter().find(|o| o["kind"] == "flat").unwrap().clone();
    assert!(flat["retract_z_mm"].is_number() && flat["feed_z_mm"].is_number(), "{flat}");
}

/// A chamfer is planned for a wall at size. Without that wall it would leave a step.
#[test]
fn a_chamfer_goes_with_its_wall() {
    let analysis = fixture("tests/fixtures/mounting-plate.analysis.json");
    let mesh = fixture("tests/fixtures/mounting-plate.mesh.json");
    let mut plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let ops = plan["setups"][0]["operations"].as_array_mut().unwrap();
    let needed = ops.iter().find(|o| o["kind"] == "edge_chamfer" && !o["needs_wall"].is_null()).expect("an outside chamfer")["needs_wall"].clone();
    let wall = ops.iter_mut().find(|o| o["wall"] == needed).unwrap();
    wall["room"] = json!(1.0);
    step2cam::job::settle(&mut plan, &tools()).unwrap();
    let ops = plan["setups"][0]["operations"].as_array().unwrap();
    assert!(!ops.iter().any(|o| o["wall"] == needed), "the wall stayed");
    for chamfer in ops.iter().filter(|o| o["kind"] == "edge_chamfer") {
        assert_eq!(chamfer["boundaries"], "holes", "{chamfer}");
    }
    assert!(plan["skipped"].to_string().contains("its wall keeps its roughing allowance"), "{}", plan["skipped"]);
    assert!(plan["warnings"].to_string().contains("too narrow"), "{}", plan["warnings"]);
}

/// Climb: the part on the cutter's right, clockwise round the outside.
#[test]
fn the_wall_finish_climbs() {
    let (analysis, mesh) = part("walls/round-boss");
    let plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&plan, &tools()).unwrap();
    let contours: Vec<&Value> = jobs[0].manifest["operations"].as_array().unwrap().iter().filter(|o| o["kind"] == "contour").collect();
    assert!(!contours.is_empty());
    assert!(contours.iter().all(|c| c["reverse"] == true));
}

fn stepped_operations(out: &Value) -> Vec<Value> {
    out["plan"]["setups"][0]["operations"].as_array().unwrap().clone()
}

/// The notches, the square slot and the V-channel: too narrow for roughing's Ø12 mill.
#[test]
fn notches_slots_and_the_v_channel_get_tools_that_fit() {
    let out = process("stepped-plate");
    let ops = stepped_operations(&out);
    let of = |kind: &str| ops.iter().filter(|o| o["kind"] == kind).count();
    assert_eq!((of("notch"), of("v_channel")), (3, 1), "{ops:?}");
    let log = out["log"].to_string();
    assert!(log.contains("corners stay r1.5, the radius of em3"), "{log}");
    assert!(log.contains("corners stay r3.0, the radius of em6"), "{log}");
    assert!(log.contains("vb60 is a 60.0° bit in a 61.9275° channel: its walls keep up to 0.034 mm"), "{log}");
    assert!(out["plan"]["warnings"].as_array().unwrap().is_empty(), "{}", out["plan"]["warnings"]);
    let material = &out["programs"][0]["material"];
    assert_eq!(material["gouge"]["definite_cells"], 0);
    assert!(material["residual"]["volume_mm3"][0].as_f64().unwrap() < 500., "{material}");
    for part in ["stepped-plate-chamfered"] {
        assert_eq!(process(part)["programs"][0]["material"]["gouge"]["definite_cells"], 0);
    }
}

fn job_of_stepped() -> Value {
    let (analysis, mesh) = part("stepped-plate");
    let plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&plan, &tools()).unwrap();
    jobs[0].manifest.clone()
}

/// A notch moved 1 mm sideways cuts into its wall: the comparison with the part must see that.
#[test]
fn a_notch_in_the_wrong_place_is_refused() {
    let mut manifest = job_of_stepped();
    let notch = manifest["operations"].as_array_mut().unwrap().iter_mut().find(|o| o["id"].as_str().unwrap().ends_with("notch") && o["tool"] == "em6").unwrap();
    for v in notch["region"]["components"][0]["outer"]["vertices"].as_array_mut().unwrap() {
        v[0] = (v[0].as_f64().unwrap() + 1.).into();
    }
    let Err(failure) = post(&manifest.to_string(), machine()) else { panic!("a notch 1 mm out of place passed") };
    assert!(failure.message.contains("cuts into the part"), "{}", failure.message);
}

/// The V-channel's cut is under what the check can see from above if it goes deeper by less than a
/// mesh error, but not by a millimetre.
#[test]
fn a_v_channel_cut_too_deep_is_refused() {
    let mut manifest = job_of_stepped();
    let v = manifest["operations"].as_array_mut().unwrap().iter_mut().find(|o| o["kind"] == "trace").unwrap();
    v["depth_z_mm"] = (v["depth_z_mm"].as_f64().unwrap() - 2.).into();
    let Err(failure) = post(&manifest.to_string(), machine()) else { panic!("a V-channel 2 mm too deep passed") };
    assert!(failure.message.contains("cuts into the part"), "{}", failure.message);
}

/// A record the mesh disagrees with is not cut: the box it describes must be empty.
#[test]
fn a_notch_the_mesh_disagrees_with_is_not_cut() {
    let (mut analysis, mesh) = part("stepped-plate");
    for feature in analysis["features"].as_array_mut().unwrap() {
        if feature["family"] == "pockets" {
            feature["record"]["w_center"] = 9.0.into();
        }
    }
    let plan = step2cam::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let notches = plan["setups"][0]["operations"].as_array().unwrap().iter().filter(|o| o["kind"] == "notch").count();
    assert_eq!(notches, 1, "only the square slot");
}

/// A wider bit would cut the channel's walls, and a flat tip could not reach its root.
#[test]
fn a_v_channel_needs_a_sharp_bit_no_wider_than_it() {
    for change in [json!({"angle": 90}), json!({"tip_diameter": 0.5})] {
        let mut library = tools();
        let bit = library["tools"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == "vb60").unwrap();
        for (key, value) in change.as_object().unwrap() {
            bit[key] = value.clone();
        }
        let (analysis, mesh) = part("stepped-plate");
        let out = step2cam::process(&analysis, &mesh, &library).unwrap();
        assert!(out["plan"]["warnings"].to_string().contains("no sharp V-bit"), "{change}: {}", out["plan"]["warnings"]);
        assert!(out["plan"]["skipped"].to_string().contains("no sharp V-bit"), "{change}");
        assert!(!out["plan"]["setups"][0]["operations"].to_string().contains("v_channel"), "{change}");
    }
}

/// With its flat not cut, stock is still above the channel: the bit would take that in passes meant
/// for a V.
#[test]
fn a_v_channel_on_a_flat_goes_with_the_flat() {
    let mut library = tools();
    for tool in library["tools"].as_array_mut().unwrap() {
        if tool["type"] == "end_mill" && tool["diameter"].as_f64().unwrap() > 6. {
            tool["flute_length"] = 1.into();
            tool["stickout"] = 1.5.into();
        }
    }
    let (analysis, mesh) = part("stepped-plate");
    let out = step2cam::process(&analysis, &mesh, &library).unwrap();
    assert!(!out["plan"]["setups"][0]["operations"].to_string().contains("v_channel"));
    assert!(out["plan"]["warnings"].to_string().contains("V-channel left out"), "{}", out["plan"]["warnings"]);
}
