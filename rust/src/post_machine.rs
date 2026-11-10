//! Boundary to camlib's posting machine, deliberately unused by planning.
//!
//! `machine_fields` is the sole translation of resource names into camlib names.
//! It returns a partial definition plus a field-by-field audit, never a postable
//! machine with guessed premises. #52 must supply the dialect and setup fields,
//! resolve required unknowns, then deserialize and validate with camlib.
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct MachineFields {
    /// Camlib names, only at this boundary. Haas extras are for its Python
    /// ReferenceMachine contract; the current Rust Machine rejects those extras.
    pub fields: Map<String, Value>,
    /// Every Rust Machine field, plus Haas ReferenceMachine fields when selected.
    /// Unknown optional limits are recorded even though camlib permits omission.
    pub coverage: BTreeMap<String, String>,
}

/// Translate one machine entry and the library that owns its existing rapid rate.
///
/// Post policy (not machine facts): metric, 3 decimals, .001 mm coordinate
/// tolerance, .01 mm/min feed tolerance, one-turn curves, no feed scheduling.
/// Haas uses linear curves until #52 opts into exact-IJK, no wear compensation,
/// and a feed traverse at the cutting ceiling (G0's dogleg is not our path).
/// LinuxCNC uses native arcs/helices in XYZ planes and G0 (null feed traverse).
/// Controller code/cycles/framing stay upstream; nothing here constructs them.
/// Unknown limits remain absent, including optional acceleration/crash geometry.
/// Physical axis lengths need the job's frame/origin to become tip travel bounds;
/// pocket count establishes changer capacity, NOT the addressable T/H range.
pub fn machine_fields(machine: &Value, library: &Value) -> MachineFields {
    let mut out = MachineFields {
        fields: Map::new(),
        coverage: BTreeMap::new(),
    };
    let settings = &machine["controller_settings"];
    let haas = machine["controller"] == "haas_ngc";
    let supported = haas || machine["controller"] == "linuxcnc";
    let mut put = |field: &str, value: Option<Value>, source: &str| {
        let status = if let Some(value) = value.filter(|v| !v.is_null()) {
            out.fields.insert(field.into(), value);
            format!("covered: {source}")
        } else {
            format!("unknown: {source}")
        };
        out.coverage.insert(field.into(), status);
    };
    put(
        "name",
        machine.get("name").or_else(|| machine.get("id")).cloned(),
        "machine.name/id",
    );
    put(
        "max_spindle_rpm",
        machine["spindle"].get("max_rpm").cloned(),
        "spindle.max_rpm",
    );
    put(
        "rapid_mm_min",
        library.get("rapid_mm_min").cloned(),
        "library.rapid_mm_min (may be an estimate; verify for posting)",
    );
    put(
        "tool_changes",
        machine["tool_changer"]["kind"]
            .as_str()
            .map(|_| json!(true)),
        "tool_changer.kind; pockets remain a setup capacity check",
    );
    for (field, kind) in [("flood", "flood"), ("mist", "mist")] {
        put(
            field,
            machine["coolant"]
                .as_array()
                .map(|v| json!(v.contains(&json!(kind)))),
            "machine.coolant; air is not mist",
        );
    }
    // These are step2cam machining/controller names on the left and upstream
    // names on the right. Do not move these aliases into CAPP or resources.
    for (ours, theirs) in [
        ("max_cutting_feed_mm_min", "max_feed_mm_min"),
        ("highest_tool_number", "max_tool_number"),
        ("highest_length_offset_number", "max_length_register"),
        ("tool_change_gauge_home_z_mm", "tool_change_home_z_mm"),
        ("normal_acceleration_limit_mm_s2", "max_normal_accel_mm_s2"),
        ("axis_feed_limits_mm_min", "max_axis_feed_mm_min"),
    ] {
        put(
            theirs,
            settings.get(ours).cloned(),
            &format!("controller_settings.{ours}"),
        );
    }
    let change = match settings["tool_change_location"].as_str() {
        Some("declared_position") => Some("table"),
        Some("unchanged_tip") => Some("preserve_tip"),
        Some("machine_z_home") => Some("z_home"),
        _ => None,
    };
    put(
        "tool_change_position_mode",
        change.map(|s| json!(s)),
        "controller_settings.tool_change_location",
    );
    put(
        "output_units",
        Some(
            settings
                .get("required_output_units")
                .cloned()
                .unwrap_or(json!("mm")),
        ),
        "adapter metric policy, overridden only by controller unit requirement",
    );
    for (field, value) in [
        ("decimals", json!(3)),
        ("tolerance_mm", json!(0.001)),
        ("feed_tolerance_mm_min", json!(0.01)),
        ("max_sweep_rad", json!(std::f64::consts::TAU)),
        ("revision", json!("step2cam-post-machine-1")),
        ("schedule_post_feeds", json!(false)),
    ] {
        put(
            field,
            Some(value),
            "adapter post policy; not stored machine facts",
        );
    }
    if haas {
        for (ours, theirs) in [
            ("setting_33_coordinate_system", "coordinate_system"),
            ("setting_15_h_and_t_code_agreement", "h_t_agreement"),
            ("g92_coordinate_shift_mm", "g92_shift_mm"),
            ("rotation_and_scaling_option", "rotation_scaling"),
            ("rigid_tapping_option", "rigid_tapping"),
            ("setting_22_can_cycle_delta_z_mm", "setting_22_mm"),
            ("setting_52_g83_retract_above_r_mm", "setting_52_mm"),
            ("optional_stop_switch_enabled", "optional_stop_enabled"),
            ("setting_45_46_47_mirror_image_xyz", "mirror_settings_xyz"),
        ] {
            put(
                theirs,
                settings.get(ours).cloned(),
                &format!("controller_settings.{ours}; installed value must be verified"),
            );
        }
        put(
            "curve_policy",
            Some(json!("linear")),
            "adapter conservative curve policy",
        );
        put(
            "cutter_compensation",
            Some(json!(false)),
            "adapter disables wear compensation until setup wear registers are supplied",
        );
        put(
            "reference",
            Some(json!("Haas NGC milling;96-8210 RevN December2020")),
            "upstream reference post identity",
        );
        put(
            "traverse_feed_mm_min",
            settings.get("max_cutting_feed_mm_min").cloned(),
            "adapter feed traverse at declared cutting ceiling",
        );
    }
    // Null here means deliberately unused, unlike an absent unknown premise.
    if change.is_some_and(|c| c != "z_home") {
        out.fields
            .insert("tool_change_home_z_mm".into(), Value::Null);
        out.coverage.insert(
            "tool_change_home_z_mm".into(),
            "covered: home Z not applicable to selected change location".into(),
        );
    }
    out.coverage.insert(
        "crash_model".into(),
        "unknown: full machine collision geometry; vise dimensions alone are insufficient".into(),
    );
    out.coverage.insert(
        "motion_policy".into(),
        "covered: adapter does not request optional scheduling policy".into(),
    );
    if supported {
        out.fields.insert(
            "planes".into(),
            if haas {
                json!(["xy"])
            } else {
                json!(["xy", "xz", "yz"])
            },
        );
        out.fields.insert("arcs".into(), json!(!haas));
        out.fields.insert("helices".into(), json!(!haas));
        for field in ["planes", "arcs", "helices"] {
            out.coverage.insert(
                field.into(),
                "controller: capabilities restricted by adapter curve policy".into(),
            );
        }
    } else {
        for field in ["planes", "arcs", "helices"] {
            out.coverage.insert(
                field.into(),
                "unknown: controller post not implemented".into(),
            );
        }
    }
    if !haas {
        if supported {
            out.fields
                .insert("traverse_feed_mm_min".into(), Value::Null);
        }
        out.coverage.insert(
            "traverse_feed_mm_min".into(),
            if supported {
                "covered: adapter uses native G0"
            } else {
                "unknown: controller traverse semantics not implemented"
            }
            .into(),
        );
    }
    out.coverage.insert(
        "dialect".into(),
        if supported {
            "controller: upstream dialect selected by machine.controller; #52 supplies it"
        } else {
            "unknown: Carvera has no upstream post"
        }
        .into(),
    );
    for field in [
        "tools",
        "work_offset",
        "frame",
        "work_to_machine",
        "tip_travel_mm",
    ] {
        out.coverage.insert(
            field.into(),
            "per-job: tools/offsets/setup frames; transform axes.travel_mm into setup tip bounds"
                .into(),
        );
    }
    if haas {
        out.coverage
            .insert("program_number".into(), "per-job: program identity".into());
    }
    out
}
