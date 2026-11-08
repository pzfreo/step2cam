//! The rules kept as data (`capp/rulebase/rules/`) and their evaluator.
use capp::resources::Library;
use capp::rules::{eval, rank};
use serde_json::{Value, json};

fn none(name: &str) -> Result<f64, String> {
    Err(format!("no {name}"))
}

#[test]
fn expressions_follow_the_usual_precedence() {
    assert_eq!(eval("1 + 2 * 3", &none).unwrap(), 7.);
    assert_eq!(eval("(1 + 2) * 3", &none).unwrap(), 9.);
    assert_eq!(eval("10 - 4 - 3", &none).unwrap(), 3.);
    assert_eq!(eval("-1e-9 <= 0 && 2 < 1", &none).unwrap(), 0.);
    assert_eq!(eval("x / 2 >= 1.5", &|n| if n == "x" { Ok(3.) } else { none(n) }).unwrap(), 1.);
    assert!(eval("y + 1", &none).is_err());
}

fn library(tools: Value) -> Value {
    json!({"tools": tools})
}
fn mill(id: &str, d: f64) -> Value {
    json!({"id": id, "type": "end_mill", "number": 1, "diameter": d, "overall_length": 50, "stickout": 30, "holder_length": 20,
           "holder_diameter": 20, "feed_mm_min": 500, "spindle_rpm": 8000, "coolant": "flood", "flute_length": 20,
           "plunge_mm_min": 100, "stepdown_mm": 2, "stepover_mm": 2})
}
fn drill(id: &str, d: f64) -> Value {
    json!({"id": id, "type": "drill", "number": 2, "diameter": d, "overall_length": 50, "stickout": 30, "holder_length": 20,
           "holder_diameter": 20, "feed_mm_min": 200, "spindle_rpm": 3000, "coolant": "flood", "flute_length": 30, "point_angle": 118})
}

/// Ties go as the code they replaced sent them: the last end mill of a size, the first drill.
#[test]
fn ties_go_as_before() {
    let data = library(json!([mill("a6", 6.), mill("b6", 6.), mill("c4", 4.), drill("d1", 5.), drill("d2", 5.)]));
    let lib = Library::new(&data).unwrap();
    assert_eq!(lib.bore_mill(10.).unwrap()["id"], "b6");
    assert_eq!(lib.clearing("roughing").unwrap().unwrap()["id"], "b6");
    assert_eq!(lib.drill(5., 5.).unwrap()["id"], "d1");
    assert_eq!(lib.drill(5.04, 5.).unwrap()["id"], "d1", "within drill_tolerance_mm (0.05)");
    assert!(lib.drill(5.06, 5.).is_none(), "beyond it");
    assert!(lib.drill(4.99, 5.).is_none(), "never larger");
    assert_eq!(lib.bore_mill(6.04).unwrap()["id"], "c4", "room to helix: 0.05 mm");
}

#[test]
fn every_kind_of_operation_has_its_place() {
    let order: Vec<usize> = ["facing", "roughing", "flats", "spot_drill", "holes", "counterbore", "slot", "wall_finish", "edge_chamfer", "thread"].iter().map(|k| rank(k).unwrap()).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    assert_eq!(rank("notch").unwrap(), rank("slot").unwrap());
    assert!(rank("knurl").is_err());
}

/// A tool field the library does not give as a number disqualifies that tool, as it did before
/// the rules were data: it neither panics nor is chosen.
#[test]
fn a_tool_with_a_field_that_is_not_a_number_is_not_chosen() {
    let mut short = mill("bad", 6.);
    short["flute_length"] = Value::Null;
    let mut bit = json!({"id": "vb", "type": "chamfer_mill", "number": 3, "diameter": 6, "overall_length": 50, "stickout": 30,
        "holder_length": 20, "holder_diameter": 20, "feed_mm_min": 300, "spindle_rpm": 9000, "coolant": "flood",
        "flute_length": 5, "angle": 60, "tip_diameter": "0", "plunge_mm_min": 100, "stepdown_mm": 1});
    let data = library(json!([short, mill("good", 4.), bit.clone()]));
    let lib = Library::new(&data).unwrap();
    assert_eq!(lib.pocket_mill(10., 5.).unwrap()["id"], "good");
    assert!(lib.v_bit(90.).is_none(), "a tip diameter written as text does not qualify");
    bit["tip_diameter"] = json!(0);
    let data = library(json!([bit]));
    assert_eq!(Library::new(&data).unwrap().v_bit(90.).unwrap()["id"], "vb");
}

#[test]
fn and_does_not_look_past_a_false_left_side() {
    assert_eq!(eval("0 && nope > 1", &none).unwrap(), 0.);
    assert!(eval("1 && nope > 1", &none).is_err());
    assert_eq!(eval("1E3", &none).unwrap(), 1000.);
}

#[test]
fn drill_selector_rejects_short_flutes_and_keeps_a_longer_equal_diameter() {
    let mut short = drill("short", 5.);
    let point = capp::resources::drill_tip_length(&short).unwrap();
    short["flute_length"] = json!(25. + point);
    let mut long = short.clone();
    long["id"] = json!("long");
    long["flute_length"] = json!(25. + point + capp::resources::DRILLING_GUARD);
    let data = library(json!([short, long]));
    let lib = Library::new(&data).unwrap();
    assert_eq!(lib.drill(5., 25.).unwrap()["id"], "long");
    let candidates = lib.considered("drill", &[("diameter", 5.), ("depth", 25.)]);
    assert!(candidates.iter().any(|c| c["candidate"] == "short"
        && c["rejected"].as_str().is_some_and(|s| s.contains("flutes do not reach 25.0 mm down") && s.contains("point"))), "{candidates:?}");
    assert!(lib.drill(5., 25.001).is_none());
}

#[test]
fn only_pocket_and_drill_reach_conditions_are_deferred() {
    let rules: Value = serde_json::from_str(include_str!("../capp/rulebase/rules/tools.json")).unwrap();
    let mut marked = vec![];
    for selector in rules["selectors"].as_array().unwrap() {
        for keep in selector["keep"].as_array().unwrap() {
            if let Some(flag) = keep.get("reach") {
                assert_eq!(flag, true);
                assert!(keep["if"].as_str().unwrap().starts_with("tool.flute_length >="));
                marked.push(selector["id"].as_str().unwrap());
            }
        }
    }
    assert_eq!(marked, ["drill", "pocket_mill"]);
    let data = library(json!([mill("wide", 12.), mill("a6", 6.), mill("b6", 6.)]));
    let lib = Library::new(&data).unwrap();
    assert!(lib.pocket_mill(10., 60.).is_none(), "the selector API stays strict; resolution defers reach");
    let mut plan = json!({"setups":[{"id":"top", "stock":{"min":[0,0,-60],"max":[20,20,0]}, "operations":[
        {"kind":"notch", "width":10., "top_z":0., "floor_z":-60., "features":["pockets:0"]}
    ]}]});
    capp::finish(&mut plan, &data, &[]).unwrap();
    assert_eq!(plan["setups"][0]["operations"][0]["tool"], "b6", "deferred resolution preserves the last-tie preference and rejects the wide mill");
    assert!(plan["decisions"].as_array().unwrap().iter().any(|d| d["chosen"] == "b6" && d["says"].as_str().unwrap().contains("reach is left to camlib")));
    assert!(lib.pocket_mill(1., 60.).is_none());
    assert!(lib.considered("pocket_mill", &[("width", 10.), ("depth", 60.)]).iter().any(|c| c["candidate"] == "b6" && c["rejected"].as_str().unwrap().contains("flutes")));
}
