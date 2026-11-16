//! On-demand Haas posting boundary. Program premises are explicit, not measured facts.
use serde_json::{Value, json};

pub const CARVERA_REASON: &str = "Carvera G-code waits for a measured spindle-to-bed height.";

fn refused(message: String) -> Value {
    json!({"released": false, "program": null, "accepted": [],
        "refused": {"message": message, "statuses": ["invalid_input"]},
        "findings": [{"status": "invalid_input", "findings": [{"message": message}]}]})
}

/// Post exactly the saved setup manifest, without replanning or accepting any
/// uncertain/violation finding. The declared controller and simulated setup
/// premises are returned beside the findings for display before machine use.
pub fn post_setup(manifest: &Value, library: &Value, machine_id: &str, setup: &str) -> Value {
    let Some(machine) = library["machines"].as_array().and_then(|machines|
        machines.iter().find(|m| m["id"] == machine_id)) else {
        return refused(format!("No machine {machine_id} in the library."));
    };
    match machine["controller"].as_str() {
        Some("carvera") => return refused(CARVERA_REASON.into()),
        Some("haas_ngc") => {},
        _ => return refused("G-code saving is supported only for the Haas VF-2; LinuxCNC posting is not enabled.".into()),
    }
    // Only this adapter translates neutral declared premises into camlib fields.
    let mut declared = machine.clone();
    let required = [
        "setting_33_coordinate_system", "setting_15_h_and_t_code_agreement",
        "g92_coordinate_shift_mm", "setting_45_46_47_mirror_image_xyz",
        "rotation_and_scaling_option", "setting_22_can_cycle_delta_z_mm",
        "setting_52_g83_retract_above_r_mm", "rigid_tapping_option",
        "optional_stop_switch_enabled", "traverse_feed_mm_min", "arc_line_chord_tolerance_mm",
    ];
    for name in required {
        if machine["controller_settings"]["declared_premises"][name].get("value").is_none() {
            return refused(format!("Haas posting needs the declared premise {name}."));
        }
    }
    let mut overrides = serde_json::Map::new();
    if let Some(premises) = machine["controller_settings"]["declared_premises"].as_object() {
        for (name, entry) in premises {
            let installed = &machine["controller_settings"][name];
            if !installed.is_null() && *installed != entry["value"] {
                overrides.insert(name.clone(), json!({"installed": installed, "declared": entry["value"]}));
            }
            declared["controller_settings"][name] = entry["value"].clone();
        }
    }
    let mut fields = crate::post_machine::machine_fields(&declared, library).fields;
    fields.insert("dialect".into(), serde_json::from_str(include_str!("haas_dialect.json")).unwrap());
    let Some(gauge_z) = manifest["tool_change"]["gauge_z_mm"].as_f64() else {
        return refused("Haas posting needs the setup's modeled tool-change gauge Z.".into());
    };
    let Some(xy) = manifest["tool_change"]["xy_mm"].as_array().filter(|xy| xy.len() == 2) else {
        return refused("Haas posting needs the setup's modeled tool-change XY.".into());
    };
    let number = match setup {
        "top" => 1000,
        "bottom" => 1001,
        "front" => 1002,
        _ => return refused(format!("No Haas program slot assigned for setup {setup}.")),
    };
    fields.get_mut("controller").unwrap()["program_number"] = json!(number);
    // Upstream requires present nullable fields to distinguish undeclared cycle
    // distances from an accidentally incomplete controller definition.
    fields.get_mut("controller").unwrap()["setting_22_mm"] = declared["controller_settings"]["setting_22_can_cycle_delta_z_mm"].clone();
    fields.get_mut("controller").unwrap()["setting_52_mm"] = declared["controller_settings"]["setting_52_g83_retract_above_r_mm"].clone();
    // Reference post's physical T/H table subset (96-8210 Rev N), not pocket
    // capacity. Each setup still uses only the library's selected tools.
    fields.insert("max_tool_number".into(), json!(200));
    fields.insert("traverse_feed_mm_min".into(), declared["controller_settings"]["traverse_feed_mm_min"].clone());
    fields.insert("tool_change_position_mode".into(), json!("table"));
    fields.insert("tool_change_home_z_mm".into(), Value::Null);
    // No machine-coordinate limits can be derived from travel lengths alone.
    fields.insert("tip_travel_mm".into(), Value::Null);
    let frame = json!({"origin_mm": [0,0,0], "x": [1,0,0], "y": [0,1,0], "z": [0,0,1]});
    fields.insert("frame".into(), frame.clone());
    fields.insert("work_offset".into(), json!(1));
    fields.insert("work_to_machine".into(), frame.clone());
    let tools: Vec<Value> = manifest["tools"].as_array().into_iter().flatten().map(|tool| {
        let spec = library["tools"].as_array().and_then(|tools| tools.iter().find(|t| t["id"] == tool["id"]));
        json!({"tool": tool["id"], "number": spec.map(|t| &t["number"]),
            "length_register": spec.map(|t| &t["number"]), "gauge_length_mm": tool["gauge_length_mm"],
            "position_after_mm": [xy[0], xy[1], tool["gauge_length_mm"].as_f64().map(|g| gauge_z-g)], "diameter_register": null})
    }).collect();
    fields.insert("tools".into(), json!(tools));
    let Some(chord) = declared["controller_settings"]["arc_line_chord_tolerance_mm"].as_f64() else {
        return refused("Haas posting needs an explicit arc-to-line chord tolerance premise.".into());
    };
    let lowering = camlib::gcode::LoweringOptions {
        chord_tolerance_mm: chord, native_arcs: true,
        ..Default::default()
    };
    let settings = match capp::resources::Settings::new(library) {
        Ok(settings) => settings,
        Err(error) => return refused(error),
    };
    if let Some(tolerance) = settings.gouge_tolerance_mm
        && manifest["verification"]["material"].as_array().into_iter().flatten()
            .any(|check| check["options"]["gouge_tolerance_mm"].as_f64() != Some(tolerance)) {
        return refused("The saved job's gouge tolerance differs from the library declaration; process the setup again.".into());
    }
    let premises = vec![
        format!("Load this setup alone in available O{number}; G54 at the local setup origin is treated as machine zero for coordinate translation, not measured machine placement. No machine travel or collision geometry is checked."),
        format!("M6 is assumed to leave the spindle gauge at setup Z {gauge_z} mm over X {} Y {} mm; this is the planned pose, not a measured Haas home or swept M6 route. Verify the actual pose and route.", xy[0], xy[1]),
        "H=T; T/H addresses assume the reference post's 1–200 table subset, not 200 loaded pockets. Gauge lengths are the library stickout plus holder projection. Verify actual tool addresses, assemblies and length geometry/wear offsets.".into(),
        format!("Traverse uses G1 at {} mm/min as declared; clearance moves use G1 because G0 doglegs are not the straight-path model. Cutting feed limit: {} mm/min.", fields["traverse_feed_mm_min"], fields["max_feed_mm_min"]),
        format!("Arcs that can't be written exactly at 0.001 mm are written as straight moves within 0.001 mm, with a {chord} mm chord budget plus coordinate rounding. The existing Haas linear profile requires this for every curve; camlib independently checks the full decoded source intervals and reports arc-output reasons."),
    ];
    let input = json!({"manifest": manifest, "program_id": setup, "frame": frame,
        "machine": fields, "lowering": lowering, "accept": ["unchecked_machine"]});
    let mut report = match camlib::post_job::run(&input.to_string(), &|| false) {
        Ok(report) => serde_json::from_str(&report).unwrap(),
        Err(error) => {
            let mut report = refused(error);
            report["refusal_source"] = json!("camlib");
            report
        },
    };
    report["premises"] = json!(premises);
    report["controller_settings"] = declared["controller_settings"].clone();
    report["premise_overrides"] = json!(overrides);
    report["lowering_policy"] = json!(lowering);
    if let Some(tolerance) = settings.gouge_tolerance_mm { report["gouge_tolerance_mm"] = json!(tolerance); }
    report
}
