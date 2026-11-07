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
    assert_eq!(lib.drill(5.).unwrap()["id"], "d1");
    assert_eq!(lib.drill(5.04).unwrap()["id"], "d1", "within drill_tolerance_mm (0.05)");
    assert!(lib.drill(5.06).is_none(), "beyond it");
    assert!(lib.drill(4.99).is_none(), "never larger");
    assert_eq!(lib.bore_mill(6.04).unwrap()["id"], "c4", "room to helix: 0.05 mm");
}

#[test]
fn every_kind_of_operation_has_its_place() {
    let order: Vec<usize> = ["facing", "roughing", "flats", "holes", "counterbore", "slot", "wall_finish", "edge_chamfer", "thread"].iter().map(|k| rank(k).unwrap()).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    assert_eq!(rank("notch").unwrap(), rank("slot").unwrap());
    assert!(rank("knurl").is_err());
}
