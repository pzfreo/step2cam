//! Public-source property meanings and equivalent libraries at both entry points.
mod common;
use common::*;
use capp::reference::iso13399_fields;
use capp::resources::normalize;
use serde_json::{Value, json};

fn symbols(library: &Value) -> Value {
    let mut out = library.clone();
    for tool in out["tools"].as_array_mut().unwrap() {
        for row in iso13399_fields()["fields"].as_array().unwrap() {
            if !row["types"].as_array().unwrap().contains(&tool["type"]) { continue; }
            if let Some(symbol) = row["symbol"].as_str() {
                let object = tool.as_object_mut().unwrap();
                if let Some(value) = object.remove(row["field"].as_str().unwrap()) {
                    object.insert(symbol.into(), value);
                }
            }
        }
    }
    out
}

/// Ford's type-specific conversion key; Kyocera R5's DC, DN, APMX, LCF and OAL;
/// Dormer's and Sandvik's property lists (URLs in the table), read 2026-11-08.
#[test]
fn published_symbols_and_measurement_conventions() {
    let rows = iso13399_fields()["fields"].as_array().unwrap();
    for (field, kind, symbol) in [
        ("diameter", "end_mill", "DC"), ("diameter", "drill", "DC"),
        ("diameter", "thread_mill", "DC"), ("overall_length", "drill", "OAL"),
        ("shank_diameter", "end_mill", "DCONMS"), ("neck_diameter", "thread_mill", "DN"),
        ("point_angle", "drill", "SIG"), ("point_angle", "spot_drill", "SIG"),
        ("flute_length", "spot_drill", "LCF"), ("flute_length", "end_mill", "APMX"),
        ("flute_length", "chamfer_mill", "APMX"), ("flute_length", "drill", "LCF"),
    ] {
        let row = rows.iter().find(|r| r["field"] == field && r["types"].as_array().unwrap().contains(&json!(kind))).unwrap();
        assert_eq!(row["symbol"], symbol);
        assert_eq!(row["unit"], if symbol == "SIG" { "degree" } else { "mm" });
        assert!(row["sources"].as_array().unwrap().len() >= 2);
    }
    for field in ["stickout", "holder_length", "holder_diameter", "neck_length", "tip_diameter", "angle"] {
        assert!(rows.iter().filter(|r| r["field"] == field).all(|r| r["symbol"].is_null()), "{field}");
    }
    // Every allowed field in each tool variant has exactly one documented decision; no
    // misspelling escapes additionalProperties, and schema aliases cannot drift from the table.
    let schema = fixture("../../schemas/resources-1.schema.json");
    for variant in schema["$defs"]["tool"]["oneOf"].as_array().unwrap() {
        let kind = &variant["properties"]["type"]["const"];
        let relevant: Vec<_> = rows.iter().filter(|r| r["types"].as_array().unwrap().contains(kind)).collect();
        for (field, _) in variant["properties"].as_object().unwrap() {
            assert_eq!(relevant.iter().filter(|r| r["field"] == *field || r["symbol"] == *field).count(), 1, "{kind}: {field}");
        }
        for row in relevant {
            assert!(!row["reason"].as_str().unwrap().is_empty());
            for source in row["sources"].as_array().unwrap() {
                assert!(iso13399_fields()["source"]["obtained_from"].as_array().unwrap().contains(source));
            }
            assert!(variant["properties"].get(row["field"].as_str().unwrap()).is_some());
            if let Some(symbol) = row["symbol"].as_str() {
                assert!(variant["properties"].get(symbol).is_some());
            }
        }
    }
}

#[test]
fn symbol_libraries_validate_and_normalize_without_changing_inputs() {
    let validator = jsonschema::validator_for(&fixture("../../schemas/resources-1.schema.json")).unwrap();
    for path in ["../../demo/tools.json", "../../examples/tools.example.json", "../../tools/benchmark/tools.json", "../../libraries/haas-vf2.json", "../../libraries/linuxcnc-vmc.json"] {
        let current = fixture(path);
        let iso = symbols(&current);
        assert_ne!(current, iso);
        assert!(validator.is_valid(&iso), "{path}: {:?}", validator.iter_errors(&iso).collect::<Vec<_>>());
        assert_eq!(normalize(&iso).unwrap().as_ref(), &current);
        assert_eq!(normalize(normalize(&iso).unwrap().as_ref()).unwrap().as_ref(), &current);
        // Every mapped alias can also coexist with its primary field.
        let mut paired = iso.clone();
        for (a, b) in paired["tools"].as_array_mut().unwrap().iter_mut().zip(current["tools"].as_array().unwrap()) {
            a.as_object_mut().unwrap().extend(b.as_object().unwrap().clone());
        }
        assert!(validator.is_valid(&paired));
        assert_eq!(normalize(&paired).unwrap().as_ref(), &current);
    }
}

#[test]
fn conflicts_name_both_fields_and_missing_fields_keep_current_names() {
    let mut library = fixture("../../tests/fixtures/placeholder-tools.json");
    let index = library["tools"].as_array().unwrap().iter().position(|t| t["id"] == "em6").unwrap();
    library["tools"][index]["DC"] = json!(8);
    let mut plan = json!({});
    let message = capp::finish(&mut plan, &library, &[]).unwrap_err();
    assert_eq!(message, "tool em6: diameter 6 and DC 8 disagree");
    assert_eq!(step2cam::process(&json!({}), &json!({}), &library).unwrap_err(), message);
    library["tools"][index]["DC"] = json!(6.0);
    assert!(normalize(&library).is_ok(), "integer and float compare by value");
    library["tools"][index]["diameter"] = json!(9_007_199_254_740_993u64);
    for other in [json!(9_007_199_254_740_992u64), json!(9_007_199_254_740_992f64)] {
        library["tools"][index]["DC"] = other;
        assert!(normalize(&library).is_err(), "distinct integers must not become equal by f64 rounding");
    }
    let object = library["tools"][index].as_object_mut().unwrap();
    object.remove("DC");
    object.remove("diameter");
    assert_eq!(capp::finish(&mut plan, &library, &[]).unwrap_err(), "tool em6: missing diameter");
    let validator = jsonschema::validator_for(&fixture("../../schemas/resources-1.schema.json")).unwrap();
    assert!(!validator.is_valid(&library));
}

#[test]
fn alias_constraints_and_required_fields_match_the_primary_fields() {
    let mut current = fixture("../../tests/fixtures/placeholder-tools.json");
    // Exercise the added tap schema variant independently of the production inventories.
    current["tools"].as_array_mut().unwrap().push(example_tap(5., 0.8, 2.));
    let iso = symbols(&current);
    let schema = fixture("../../schemas/resources-1.schema.json");
    let validator = jsonschema::validator_for(&schema).unwrap();
    for variant in schema["$defs"]["tool"]["oneOf"].as_array().unwrap() {
        let tool = iso["tools"].as_array().unwrap().iter().find(|t| t["type"] == variant["properties"]["type"]["const"]).unwrap();
        for row in iso13399_fields()["fields"].as_array().unwrap() {
            if !row["types"].as_array().unwrap().contains(&tool["type"]) { continue; }
            let Some(symbol) = row["symbol"].as_str() else { continue };
            let field = row["field"].as_str().unwrap();
            let mut invalid = tool.clone();
            invalid[symbol] = json!("not a dimension");
            assert!(!validator.is_valid(&json!({"tools": [invalid]})), "{symbol}: type constraint");
            let mut pair = tool.clone();
            pair[symbol] = json!(1);
            pair[field] = json!(999);
            assert!(normalize(&json!({"tools": [pair]})).unwrap_err().contains(&format!("and {symbol}")));
            // The library requires every mapped dimension except the optional shank.
            if field != "shank_diameter" {
                let mut missing = tool.clone();
                missing.as_object_mut().unwrap().remove(symbol);
                assert!(!validator.is_valid(&json!({"tools": [missing]})), "{field}: required");
            }
        }
    }
    // A drill's LCF cannot accidentally become an end mill's APMX, nor can KAPR become
    // a chamfer's included angle. Unsupported names remain schema errors.
    for (kind, symbol) in [("end_mill", "LCF"), ("drill", "APMX"), ("chamfer_mill", "KAPR"), ("thread_mill", "LU")] {
        let mut tool = iso["tools"].as_array().unwrap().iter().find(|t| t["type"] == kind).unwrap().clone();
        tool[symbol] = json!(45);
        assert!(!validator.is_valid(&json!({"tools": [tool]})), "{kind}: {symbol}");
    }
}

#[test]
fn plate_has_identical_plans_jobs_and_reason_texts_with_symbols() {
    let library = fixture("../../tests/fixtures/placeholder-tools.json");
    let iso = symbols(&library);
    let analysis = fixture("plate.analysis.json");
    let mesh = fixture("plate.mesh.json");
    let plan = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    assert_eq!(finished(&plan, &library), finished(&plan, &iso));
    // process must normalize the library for both capp and the camlib job adapter.
    let current = step2cam::process(&analysis, &mesh, &library).unwrap();
    let aliased = step2cam::process(&analysis, &mesh, &iso).unwrap();
    assert!(!current["programs"].as_array().unwrap().is_empty());
    assert_eq!(current, aliased);
}
