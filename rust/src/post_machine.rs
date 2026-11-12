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
    /// Camlib names, only at this boundary, including the nested controller block.
    pub fields: Map<String, Value>,
    /// Every Rust Machine field, plus qualified fields of the selected Controller.
    /// Unknown optional limits are recorded even though camlib permits omission.
    pub coverage: BTreeMap<String, String>,
}

/// Translate one machine entry and the library that owns its existing rapid rate.
///
/// Post policy (not machine facts): metric, 3 decimals, .001 mm coordinate
/// tolerance, .01 mm/min feed tolerance, one-turn curves, no feed scheduling.
/// If a controller requires inches: 4 decimals and .003 mm tolerance admit the
/// 3D rounding bound (.0022 mm), including Haas's 4-place inch precision limit.
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
    let carvera = machine["controller"] == "carvera";
    let supported = haas || machine["controller"] == "linuxcnc";
    let change = machine_facts(&mut out, machine, library, settings);
    post_policy(&mut out, settings);
    named_controller(&mut out, machine, settings, haas, carvera);
    curves_and_traverse(&mut out, change, haas, supported);
    left_to_post(&mut out, haas, supported);
    out
}

impl MachineFields {
    /// Record one field: its value when known, and in either case where it comes from.
    fn put(&mut self, field: &str, value: Option<Value>, source: &str) {
        let status = if let Some(value) = value.filter(|v| !v.is_null()) {
            if let Some(key) = field.strip_prefix("controller.") {
                self.fields
                    .entry("controller")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .unwrap()
                    .insert(key.into(), value);
            } else {
                self.fields.insert(field.into(), value);
            }
            format!("covered: {source}")
        } else {
            format!("unknown: {source}")
        };
        self.coverage.insert(field.into(), status);
    }
}

/// Facts the machine and its library state, and its tool change location (camlib's name).
fn machine_facts(
    out: &mut MachineFields,
    machine: &Value,
    library: &Value,
    settings: &Value,
) -> Option<&'static str> {
    out.put(
        "name",
        machine.get("name").or_else(|| machine.get("id")).cloned(),
        "machine.name/id",
    );
    out.put(
        "max_spindle_rpm",
        machine["spindle"].get("max_rpm").cloned(),
        "spindle.max_rpm",
    );
    out.put(
        "rapid_mm_min",
        library.get("rapid_mm_min").cloned(),
        "library.rapid_mm_min (may be an estimate; verify for posting)",
    );
    out.put(
        "tool_changes",
        machine["tool_changer"]["kind"]
            .as_str()
            .map(|_| json!(true)),
        "tool_changer.kind; pockets remain a setup capacity check",
    );
    for (field, kind) in [("flood", "flood"), ("mist", "mist")] {
        out.put(
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
        out.put(
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
    out.put(
        "tool_change_position_mode",
        change.map(|s| json!(s)),
        "controller_settings.tool_change_location",
    );
    change
}

/// The adapter's post policy, in the controller's output units.
fn post_policy(out: &mut MachineFields, settings: &Value) {
    out.put(
        "output_units",
        Some(
            settings
                .get("required_output_units")
                .cloned()
                .unwrap_or(json!("mm")),
        ),
        "adapter metric policy, overridden only by controller unit requirement",
    );
    let inch = settings["required_output_units"] == "inch";
    for (field, value) in [
        ("decimals", json!(if inch { 4 } else { 3 })),
        ("tolerance_mm", json!(if inch { 0.003 } else { 0.001 })),
        ("feed_tolerance_mm_min", json!(0.01)),
        ("max_sweep_rad", json!(std::f64::consts::TAU)),
        ("revision", json!("step2cam-post-machine-2")),
        ("schedule_post_feeds", json!(false)),
    ] {
        out.put(
            field,
            Some(value),
            "adapter post policy; not stored machine facts",
        );
    }
}

/// The named controller block for Haas and Carvera: what is known, and the premises #52 resolves.
fn named_controller(
    out: &mut MachineFields,
    machine: &Value,
    settings: &Value,
    haas: bool,
    carvera: bool,
) {
    if haas || carvera {
        out.put(
            "controller.name",
            machine.get("controller").cloned(),
            "machine.controller selects upstream named post",
        );
    }
    if carvera {
        for field in [
            "initial_gauge_length_mm",
            "unmodified_coordinates",
            "automatic_tool_length",
        ] {
            out.put(
                &format!("controller.{field}"),
                None,
                "Carvera calibration/setup premise required by #52",
            );
        }
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
            out.put(
                &format!("controller.{theirs}"),
                settings.get(ours).cloned(),
                &format!("controller_settings.{ours}; installed value must be verified"),
            );
        }
        out.put(
            "controller.curve_policy",
            Some(json!("linear")),
            "adapter conservative curve policy",
        );
        out.put(
            "controller.cutter_compensation",
            Some(json!(false)),
            "adapter disables wear compensation until setup wear registers are supplied",
        );
        out.put(
            "controller.reference",
            Some(json!("Haas NGC milling;96-8210 RevN December2020")),
            "upstream reference post identity",
        );
        out.put(
            "traverse_feed_mm_min",
            settings.get("max_cutting_feed_mm_min").cloned(),
            "adapter feed traverse at declared cutting ceiling",
        );
    }
    out.coverage.insert(
        "controller".into(),
        if haas || carvera {
            "controller: partial named controller block; #52 resolves all premises"
        } else {
            "covered: generic dialect; no named controller"
        }
        .into(),
    );
}

/// Home Z, curve capabilities and traverse, as the change location and curve policy allow.
fn curves_and_traverse(out: &mut MachineFields, change: Option<&str>, haas: bool, supported: bool) {
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
                "unknown: adapter curve policy awaits #52".into(),
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
                "unknown: adapter traverse policy awaits #52"
            }
            .into(),
        );
    }
}

/// What the post or each job supplies: the dialect, tools, offsets and frames.
fn left_to_post(out: &mut MachineFields, haas: bool, supported: bool) {
    out.coverage.insert(
        "dialect".into(),
        if supported {
            "controller: upstream dialect selected by machine.controller; #52 supplies it"
        } else {
            "controller: upstream Carvera dialect; #52 supplies it"
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
        out.coverage.insert(
            "controller.program_number".into(),
            "per-job: program identity".into(),
        );
    }
}
