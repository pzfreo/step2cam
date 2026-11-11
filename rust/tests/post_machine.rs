//! Contract audit, upstream validation and boundary mapping checks.
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::PathBuf};
use step2cam::post_machine::machine_fields;

fn read(path: &str) -> Value {
    serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(path),
        )
        .unwrap(),
    )
    .unwrap()
}
fn rust_fields(source: &str, structure: &str) -> BTreeSet<String> {
    source
        .split_once(&format!("pub struct {structure} {{"))
        .unwrap()
        .1
        .split_once("\n}")
        .unwrap()
        .0
        .lines()
        .filter_map(|l| l.trim().strip_prefix("pub "))
        .map(|l| l.split_once(':').unwrap().0.to_string())
        .collect()
}

#[test]
fn every_upstream_field_has_an_explicit_responsibility_for_each_shipped_machine() {
    let rust = rust_fields(
        include_str!("../../vendor/camlib/src/gcode/machine.rs"),
        "Machine",
    );
    let controller_fields = |controller: &str| -> BTreeSet<String> {
        let fields = match controller {
            "haas_ngc" => rust_fields(
                include_str!("../../vendor/camlib/src/gcode/haas.rs"),
                "HaasNGC",
            ),
            "carvera" => rust_fields(
                include_str!("../../vendor/camlib/src/gcode/carvera.rs"),
                "Carvera",
            ),
            "linuxcnc" => return BTreeSet::new(),
            other => panic!("classify upstream controller {other}"),
        };
        assert!(
            !fields.is_empty(),
            "upstream controller fields must be audited"
        );
        fields
            .into_iter()
            .chain(["name".into()])
            .map(|f| format!("controller.{f}"))
            .collect()
    };
    for path in [
        "libraries/linuxcnc-vmc.json",
        "libraries/haas-vf2.json",
        "demo/tools.json",
    ] {
        let library = read(path);
        for machine in library["machines"].as_array().unwrap() {
            let mapped = machine_fields(machine, &library);
            let expected = rust
                .union(&controller_fields(machine["controller"].as_str().unwrap()))
                .cloned()
                .collect();
            assert_eq!(
                mapped.coverage.keys().cloned().collect::<BTreeSet<_>>(),
                expected,
                "{path}: contract changed; classify the new fields"
            );
            println!("\n{path}");
            for (field, status) in &mapped.coverage {
                assert!(
                    ["covered:", "controller:", "per-job:", "unknown:"]
                        .iter()
                        .any(|s| status.starts_with(s)),
                    "{field}: {status}"
                );
                println!("{field}: {status}");
                if status.starts_with("unknown:") || status.starts_with("per-job:") {
                    assert!(
                        Value::Object(mapped.fields.clone())
                            .pointer(&format!("/{}", field.replace('.', "/")))
                            .is_none(),
                        "{field}: unknown/job premise must not be invented"
                    );
                }
            }
        }
    }
}

#[test]
fn linuxcnc_sample_composes_and_validates_with_camlibs_own_dialect() {
    let library = read("libraries/linuxcnc-vmc.json");
    let mut fields = machine_fields(&library["machines"][0], &library).fields;
    // Upstream Rust exposes no LinuxCNC constructor. Do not duplicate commands:
    // use the exact dialect camlib uses for its own Python/Rust parity fixture.
    let upstream = read("vendor/camlib/fixtures/posts/rust/standalone-rest.json");
    fields.insert("dialect".into(), upstream["machine"]["dialect"].clone());
    let tool = &library["tools"][0];
    let travel = &library["machines"][0]["axes"]["travel_mm"];
    let frame = json!({"origin_mm":[0,0,0],"x":[1,0,0],"y":[0,1,0],"z":[0,0,1]});
    // Explicit sample located at machine zero; not an assumed production frame.
    for (key, value) in [
        (
            "tools",
            json!([{"tool":tool["id"], "number":tool["number"], "length_register":tool["number"],
            "gauge_length_mm":tool["stickout"].as_f64().unwrap()+tool["holder_length"].as_f64().unwrap(),
            "position_after_mm":[0,0,100],"diameter_register":null}]),
        ),
        ("work_offset", json!(1)),
        ("frame", frame.clone()),
        ("work_to_machine", frame),
        (
            "tip_travel_mm",
            json!([[0, 0, 0], [travel["x"], travel["y"], travel["z"]]]),
        ),
    ] {
        fields.insert(key.into(), value);
    }
    let machine: camlib::gcode::Machine =
        serde_json::from_value(Value::Object(fields.clone())).unwrap();
    machine.validate().unwrap();
    // The schema also admits an inch-only controller: adapter quantisation
    // choices must remain valid in millimetres after the unit conversion.
    let mut inch_library = library.clone();
    inch_library["machines"][0]["controller_settings"]["required_output_units"] = json!("inch");
    let inch_fields = machine_fields(&inch_library["machines"][0], &inch_library).fields;
    let mut inch_machine = machine.clone();
    inch_machine.output_units = inch_fields["output_units"].as_str().unwrap().into();
    inch_machine.decimals = inch_fields["decimals"].as_u64().unwrap() as u32;
    inch_machine.tolerance_mm = inch_fields["tolerance_mm"].as_f64().unwrap();
    inch_machine.validate().unwrap();

    // A genuinely incomplete/invalid machine must still fail upstream checks.
    fields.remove("max_feed_mm_min");
    assert!(serde_json::from_value::<camlib::gcode::Machine>(Value::Object(fields)).is_err());
    let mut invalid = machine;
    invalid.max_length_register = 0;
    assert!(invalid.validate().is_err());
}

#[test]
fn boundary_maps_existing_values_and_settings_without_conflating_pockets_and_addresses() {
    let mut library = read("libraries/haas-vf2.json");
    library["machines"][0]["spindle"]["max_rpm"] = json!(7000);
    library["machines"][0]["tool_changer"]["pockets"] = json!(12);
    library["machines"][0]["controller_settings"]["highest_tool_number"] = json!(200);
    library["machines"][0]["controller_settings"]["setting_22_can_cycle_delta_z_mm"] = json!(0.254);
    let mapped = machine_fields(&library["machines"][0], &library);
    assert_eq!(mapped.fields["controller"]["name"], "haas_ngc");
    assert!(!mapped.fields.contains_key("setting_22_mm"));
    assert_eq!(mapped.fields["max_spindle_rpm"], 7000);
    assert_eq!(mapped.fields["max_tool_number"], 200);
    assert_eq!(mapped.fields["controller"]["setting_22_mm"], 0.254);
    assert_eq!(mapped.fields["max_feed_mm_min"], 16500);
    assert_eq!(mapped.fields["traverse_feed_mm_min"], 16500);
    assert!(mapped.fields["controller"].get("setting_52_mm").is_none());
    assert!(mapped.fields["controller"].get("g92_shift_mm").is_none());
    assert_eq!(mapped.fields["mist"], false);
    assert!(
        machine_fields(
            &read("demo/tools.json")["machines"][0],
            &read("demo/tools.json")
        )
        .coverage["dialect"]
            .starts_with("controller:")
    );
}
