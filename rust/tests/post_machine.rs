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
fn rust_fields(structure: &str) -> BTreeSet<String> {
    include_str!("../../vendor/camlib/src/gcode/machine.rs")
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
    let rust = rust_fields("Machine");
    let haas: BTreeSet<String> = include_str!("../../vendor/camlib/examples/haas_ngc_post.py")
        .split_once("class ReferenceMachine(Machine):")
        .unwrap()
        .1
        .split_once("    def __post_init__")
        .unwrap()
        .0
        .lines()
        .filter(|l| l.starts_with("    ") && !l.starts_with("        "))
        .filter_map(|l| l.trim().split_once(':'))
        .filter(|(n, _)| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .map(|(n, _)| n.to_string())
        .collect();
    assert!(!haas.is_empty());
    for path in [
        "libraries/linuxcnc-vmc.json",
        "libraries/haas-vf2.json",
        "demo/tools.json",
    ] {
        let library = read(path);
        for machine in library["machines"].as_array().unwrap() {
            let mapped = machine_fields(machine, &library);
            let expected = if machine["controller"] == "haas_ngc" {
                rust.union(&haas).cloned().collect()
            } else {
                rust.clone()
            };
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
                        !mapped.fields.contains_key(field),
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
    assert_eq!(mapped.fields["max_spindle_rpm"], 7000);
    assert_eq!(mapped.fields["max_tool_number"], 200);
    assert_eq!(mapped.fields["setting_22_mm"], 0.254);
    assert_eq!(mapped.fields["max_feed_mm_min"], 16500);
    assert_eq!(mapped.fields["traverse_feed_mm_min"], 16500);
    assert!(!mapped.fields.contains_key("setting_52_mm"));
    assert!(!mapped.fields.contains_key("g92_shift_mm"));
    assert_eq!(mapped.fields["mist"], false);
    assert!(
        machine_fields(
            &read("demo/tools.json")["machines"][0],
            &read("demo/tools.json")
        )
        .coverage["dialect"]
            .starts_with("unknown:")
    );
}
