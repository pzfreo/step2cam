//! What the camlib adapter tells camlib of each tool, and the rules its notes cite for the process
//! choices it makes.
mod common;
use common::*;
use serde_json::{Value, json};

fn run(part: &str, library: &Value) -> Value {
    let out = step2cam::process(&fixture(&format!("{part}.analysis.json")), &fixture(&format!("{part}.mesh.json")), library).unwrap();
    assert!(out["error"].is_null(), "{part}: {}", out["error"]);
    out
}

/// (flutes, material) camlib's job gives the tool, from every program that uses it.
fn cutter(out: &Value, tool: &str) -> (u64, String) {
    let found: Vec<_> = list(&out["programs"]).iter().flat_map(|p| list(&p["job"]["tools"])).filter(|t| t["id"] == tool)
        .map(|t| (t["cutter"]["flutes"].as_u64().unwrap(), t["cutter"]["material"].as_str().unwrap().to_string())).collect();
    assert!(!found.is_empty(), "no program uses {tool}");
    assert!(found.windows(2).all(|w| w[0] == w[1]));
    found[0].clone()
}

fn log(out: &Value) -> Vec<&str> {
    list(&out["log"]).iter().filter_map(Value::as_str).collect()
}

#[test]
fn a_single_flute_carvera_cutter_reaches_camlib_single_flute() {
    let library = fixture("../../demo/tools.json");
    let out = run("mounting-plate", &library);
    assert_eq!(cutter(&out, "em6"), (1, "carbide".into()));
    assert_eq!(cutter(&out, "cm3.175").0, 2);
    let log = log(&out);
    // Every demo tool states its flutes; none used here states its material.
    assert!(!log.iter().any(|l| l.contains("tool.flutes.default")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("the library gives no material for") && l.contains("em6") && l.ends_with("camlib is told carbide (tool.material.default@1)")), "{log:?}");
    // The roughing's clearing, allowances and entry, and the slot's helix, by rule.
    let rough = log.iter().find(|l| l.contains("-roughing: clears in raster passes")).unwrap();
    assert!(rough.contains("(process.roughing.clearing@1), leaving 0.3 mm on the walls and 0.2 mm on the floors (process.roughing.allowance@1)"), "{rough}");
    assert!(rough.contains("by a 3° helix (process.entry.helix@1)"), "{rough}");
    assert!(rough.contains("its tool centre stays within one cutter radius of the stock (process.roughing.region@1)"), "{rough}");
    assert!(log.iter().any(|l| l.contains("-slot enter by a 3° helix (process.entry.helix@1)")), "{log:?}");
    // Each helix the notes cite is the one the job gives camlib.
    for program in list(&out["programs"]) {
        for op in list(&program["job"]["operations"]).iter().filter(|o| o["entry"].is_object()) {
            assert_close(number(&op["entry"]["angle_rad"]), 3f64.to_radians());
        }
    }
    // Roughing never moves its centre more than a cutter radius (and 0.1 mm of air) off the stock.
    let mut roughed = 0;
    for program in list(&out["programs"]) {
        let job = &program["job"];
        let (lo, hi) = (list(&job["stock"]["min_mm"]), list(&job["stock"]["max_mm"]));
        for op in list(&job["operations"]).iter().filter(|o| o["kind"] == "roughing") {
            let tool = list(&job["tools"]).iter().find(|t| t["id"] == op["tool"]).unwrap();
            let radius = number(&tool["cutter"]["diameter_mm"]) / 2.;
            let reach = radius + 0.1;
            // The whole cutter stays inside the containment: the stock grown by its diameter and the air.
            let outer = list(&op["limits"]["contain"]["components"][0]["outer"]["vertices"]);
            assert_close(number(&outer[0][0]), number(&lo[0]) - radius - reach);
            assert_close(number(&outer[2][1]), number(&hi[1]) + radius + reach);
            for m in list(&program["moves"]).iter().filter(|m| m["op"] == op["id"] && m["kind"] != "rapid") {
                for q in list(&m["points"]) {
                    for k in 0..2 {
                        assert!(number(&q[k]) >= number(&lo[k]) - reach && number(&q[k]) <= number(&hi[k]) + reach, "{} at {q} beyond the stock", op["id"]);
                    }
                }
                roughed += 1;
            }
        }
    }
    assert!(roughed > 0);
}

#[test]
fn an_hss_drill_reaches_camlib_as_hss_and_unsaid_flutes_take_the_default() {
    let library = fixture("../../libraries/haas-vf2.json");
    let out = run("plate", &library);
    assert_eq!(cutter(&out, "drill4.2"), (2, "hss".into()));
    assert_eq!(cutter(&out, "spot10-120"), (2, "carbide".into()));
    assert_eq!(cutter(&out, "em20"), (4, "carbide".into()));
    let log = log(&out);
    let flutes = log.iter().find(|l| l.contains("the library gives no flute count for")).unwrap();
    assert!(flutes.contains("drill4.2") && flutes.contains("spot10-120") && !flutes.contains("em20"), "{flutes}");
    assert!(flutes.ends_with("camlib is told 2 for a drill or spot drill, 3 for any other tool (tool.flutes.default@1)"), "{flutes}");
    let material = log.iter().find(|l| l.contains("the library gives no material for")).unwrap();
    assert!(material.contains("em20") && !material.contains("drill4.2") && !material.contains("spot10-120"), "{material}");
}

#[test]
fn a_library_without_flutes_or_materials_uses_the_stated_defaults_and_says_so() {
    let library = fixture("placeholder-tools.json");
    assert!(list(&library["tools"]).iter().all(|t| t.get("flutes").is_none() && t.get("material").is_none()));
    let out = run("plate", &library);
    assert_eq!(cutter(&out, "em6"), (3, "carbide".into()));
    assert_eq!(cutter(&out, "drill4.2"), (2, "carbide".into()));
    assert_eq!(cutter(&out, "spot8"), (2, "carbide".into()));
    let log = log(&out);
    for program in list(&out["programs"]) {
        let setup = program["setup"].as_str().unwrap();
        let ids: Vec<&str> = list(&program["job"]["tools"]).iter().map(|t| t["id"].as_str().unwrap()).collect();
        for (what, rule) in [("flute count", "tool.flutes.default@1"), ("material", "tool.material.default@1")] {
            let note = log.iter().find(|l| l.starts_with(&format!("note: setup {setup}: the library gives no {what} for"))).unwrap();
            assert!(note.contains(rule) && ids.iter().all(|id| note.contains(id)), "{note}");
        }
    }
}

#[test]
fn flutes_and_material_are_checked_in_the_library() {
    let mut plan = json!({"setups": [], "covered_by_roughing": [], "skipped": [], "warnings": []});
    for (key, value, message) in [
        ("flutes", json!(0), "tool em6: flutes must be a whole number from 1 to 1000"),
        ("flutes", json!(2.5), "tool em6: flutes must be a whole number from 1 to 1000"),
        ("material", json!("steel"), "tool em6: material must be one of carbide, hss"),
    ] {
        let mut library = fixture("placeholder-tools.json");
        let index = list(&library["tools"]).iter().position(|t| t["id"] == "em6").unwrap();
        library["tools"][index][key] = value;
        assert_eq!(capp::finish(&mut plan, &library, &[]).unwrap_err(), message);
    }
}
