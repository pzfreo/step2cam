//! Missing-tool queries. These describe resources only; they never change tool selection.
use crate::kind::OpId;
use crate::model::{Cut, Need, Operation, Plan, ReachEvidence, ReachLimit, Setup};
use crate::resources::Library;

/// Round reach upwards with a micron of clearance. This exceeds camlib's 16 × 1e-5 mm
/// numerical guard in step2cam jobs, including when depth is an exact integer.
pub(crate) fn reach(depth: f64) -> String {
    format!("{:.0}", (depth + 0.001).ceil())
}

pub(crate) fn for_operation(op: &Operation, top: f64, library: &Library, required: Option<f64>) -> Result<Option<String>, String> {
    for_operation_with_context(op, top, library, required, None, None)
}

/// Thread queries need the same material and drilled-floor context as tap selection.
pub(crate) fn for_operation_with_context(op: &Operation, top: f64, library: &Library, required: Option<f64>, setup: Option<&Setup>, group: Option<&str>) -> Result<Option<String>, String> {
    let (bottom, through) = match &op.cut {
        Cut::Facing { floor_z } | Cut::Flats { floor_z, .. } => (*floor_z, false),
        Cut::Roughing(c) => (c.floor_z, false),
        Cut::Holes(c) => (c.bottom_z, c.bottom == "through"),
        Cut::Counterbore(c) => (c.floor_z, false),
        Cut::Thread(c) => (c.bottom_z, false),
        Cut::Slot(c) => (c.floor_z, c.bottom == "through"),
        Cut::Notch(c) => (c.floor_z, c.bottom == "through"),
        Cut::VChannel(c) => (c.root_z, false),
        Cut::WallFinish(c) => (c.floor_z, false),
        Cut::SpotDrill(_) | Cut::EdgeChamfer(_) => (top, false),
    };
    let depth = required.unwrap_or_else(|| top - bottom + if through { library.breakthrough.max(0.) } else { 0. });
    let long = format!("with flutes of at least {} mm", reach(depth));
    let mill = |selector, inputs: &[(&str, f64)]| -> Result<String, String> {
        Ok(format!("{} {long}", library.needed(selector, inputs)?))
    };
    Ok(Some(match &op.cut {
        Cut::SpotDrill(spot) => library.needed("spot_drill", &[("diameter", spot.diameter), ("angle", spot.angle), ("reach", spot.reach)])?,
        Cut::Holes(hole) => format!("{}, or {}",
            library.needed("drill", &[("diameter", hole.diameter), ("depth", library.drill_depth(hole, top).max(depth))])?,
            mill("bore_mill", &[("diameter", hole.diameter)])?),
        Cut::Counterbore(counterbore) => mill("bore_mill", &[("diameter", counterbore.diameter)])?,
        Cut::Slot(slot) => mill("bore_mill", &[("diameter", slot.width)])?,
        Cut::Notch(notch) => mill("pocket_mill", &[("width", notch.width), ("depth", depth)])?,
        Cut::Facing { .. } | Cut::Roughing(_) | Cut::Flats { .. } | Cut::WallFinish(_) => mill("largest_end_mill", &[])?,
        Cut::EdgeChamfer(chamfer) => {
            let width = chamfer.width;
            let axial = top - chamfer.top_z + width + crate::resources::TIP_CLEARANCE;
            let query = library.needed("chamfer_mill", &[("width", width), ("maximum_diameter", chamfer.maximum_diameter.flatten().unwrap_or(f64::INFINITY)), ("depth", axial)])?;
            if required.is_some() { format!("{query}, with zero tip diameter and flutes of at least {} mm", reach(depth)) } else {
                let query = format!("{query}, with flutes of at least {:.2} mm", ((axial+0.00016)*100.).ceil()/100.);
                let query = match chamfer.room.flatten() {
                    Some(room) => format!("{query}, with cutter radius + tip radius + 0.5 mm at most {} mm", crate::plan::show(room)),
                    None => query,
                };
                match chamfer.maximum_diameter.flatten() {
                    Some(maximum) => format!("{query}, diameter at most {} mm for this loop", crate::plan::show(maximum)),
                    None => query,
                }
            }
        },
        Cut::VChannel(channel) => format!("{} {long}", library.needed("v_bit", &[("angle", channel.angle)])?),
        Cut::Thread(thread) => {
            let major = thread.major;
            let pitch = thread.pitch;
            let query = library.needed("thread_mill", &[("major", major), ("start_diameter", thread.start_diameter),
                ("pitch", pitch), ("stock_top", top), ("bottom_z", bottom)])?;
            let tolerance = crate::reference::tap_drills()["diameter_tolerance_mm"].as_f64().unwrap();
            let drill = crate::reference::tap_drill(major, pitch, false).filter(|d|
                crate::resources::tap_policy_allows(major, group)
                && (*d - thread.start_diameter).abs() <= tolerance + 1e-9);
            match drill {
                Some(drill) => {
                    let number = |x: f64| format!("{x:.2}").trim_end_matches('0').trim_end_matches('.').to_string();
                    let tap = format!("an M{}×{} cut tap (⌀{} tap drill)", number(major), number(pitch), number(drill));
                    let reach = top - bottom + crate::resources::DRILLING_GUARD;
                    if thread.through {
                        let maximum = library.tap_exit_room(thread, top)?.map(|room| crate::rules::down2(room - pitch - crate::resources::DRILLING_GUARD));
                        match maximum {
                            Some(chamfer) if chamfer <= 0. => query,
                            Some(chamfer) => format!("{tap} with thread length at least {} mm plus its chamfer, and a chamfer of at most {} mm ({} pitches) for the exit clearance, or {query}", number(crate::rules::up2(reach)), number(chamfer), number(crate::rules::down2(chamfer / pitch))),
                            None => format!("{tap} with thread length at least {} mm plus its chamfer, or {query}", number(crate::rules::up2(reach))),
                        }
                    } else {
                        let open_z = library.thread_open_z(thread, setup, top)?;
                        let chamfer = crate::rules::down2(bottom - open_z - pitch);
                        if !chamfer.is_finite() || chamfer <= 0. { query } else {
                            format!("{tap} with a chamfer of at most {} mm ({} pitches) and thread length at least {} mm, or {query}", number(chamfer), number(crate::rules::down2(chamfer / pitch)), number(crate::rules::up2(reach + chamfer)))
                        }
                    }
                }
                None => query,
            }
        },
    }))
}

/// Reach evidence is decoded by the camlib adapter, never inferred from error prose.
/// Stepdown and other untyped refusals do not request a different tool.
pub(crate) fn refusal(plan: &Plan, operation: &OpId, message: &str, refused_tool: Option<&str>, evidence: Option<&ReachEvidence>, library: &Library) -> Result<String, String> {
    let Some(setup) = plan.setups.iter().find(|s| s.id == operation.setup) else { return Ok(message.into()); };
    let Some(op) = operation.number.checked_sub(1).and_then(|n| setup.operations.get(n)) else { return Ok(message.into()); };
    let top = setup.stock.max[2];
    let Some(evidence) = evidence.filter(|e| e.depth_needed_mm.is_finite() && e.depth_needed_mm > 0.) else { return Ok(message.into()); };
    let depth = evidence.depth_needed_mm;
    let tool_id = refused_tool.or(op.tool.as_ref().and_then(Option::as_deref)).unwrap_or("the tool");
    let tool = library.data["tools"].as_array().and_then(|ts| ts.iter().find(|t| t["id"] == tool_id));
    let noun = crate::plan::operation_noun(op.cut.kind().name());
    let group = plan.material.as_ref().and_then(|m| m["iso513"].as_str());
    if evidence.limit != ReachLimit::CuttingLength {
        let part = evidence.limit.name();
        let z = evidence.z_mm.unwrap_or(top - depth);
        let obstruction = evidence.top_z_mm.unwrap_or(top);
        let section = evidence.holder_section.map(|n| format!(" section {n}")).unwrap_or_default();
        // Assembly reach is not flute reach. Thread mills have a tooth height instead.
        let cutter_length = tool.and_then(|t| t["flute_length"].as_f64()
            .or_else(|| t["tooth_height"].as_f64()).or_else(|| t["thread_length"].as_f64()));
        let mut query = for_operation_with_context(op, top, library, Some(cutter_length.unwrap_or(0.)), Some(setup),
            group)?.unwrap_or_else(|| "a tool".into());
        if cutter_length.is_none() && let Some((base, _)) = query.split_once(" with flutes of at least ") {
            query = base.to_string();
        }
        let need = crate::plan::assembly_requirement(&query, part, depth);
        return Ok(format!("camlib refused the {noun}: {tool_id}'s {part}{section} cannot clear the remaining stock beside the {noun} at Z{} (tip Z{}): needs {need}",
            crate::plan::display_position(obstruction), crate::plan::display_position(z)));
    }
    let flutes = evidence.depth_available_mm.or_else(|| tool.and_then(|t| t["flute_length"].as_f64()));
    let reference = if evidence.top_z_mm.is_some_and(|z| z < top_z(op).unwrap_or(top) - 0.001) { "material top" } else { "stock top" };
    let plain = flutes.map(|flutes| format!("camlib refused the {noun}: the cut is {} mm below the {reference} and {tool_id}'s flutes are {} mm", crate::plan::show(depth), crate::plan::show(flutes))).unwrap_or_else(|| message.into());
    Ok(match for_operation_with_context(op, top, library, Some(depth), Some(setup), group)? {
        Some(need) => format!("{plain}: needs {need}"),
        None => message.into(),
    })
}

/// The operation's `top_z`, for the kinds that have one.
fn top_z(op: &Operation) -> Option<f64> {
    match &op.cut {
        Cut::Roughing(c) => Some(c.top_z),
        Cut::Holes(c) => Some(c.top_z),
        Cut::Counterbore(c) => Some(c.top_z),
        Cut::Thread(c) => Some(c.top_z),
        Cut::Slot(c) => Some(c.top_z),
        Cut::Notch(c) => Some(c.top_z),
        Cut::VChannel(c) => Some(c.top_z),
        Cut::EdgeChamfer(c) => Some(c.top_z),
        Cut::Facing { .. } | Cut::Flats { .. } | Cut::SpotDrill(_) | Cut::WallFinish(_) => None,
    }
}

/// Derive the index from the live reasons, so settling repeatedly cannot retain obsolete needs.
pub(crate) fn collect(plan: &mut Plan) {
    let mut needs: Vec<Need> = vec![];
    let mut add = |text: Option<&str>, features: &[String], operation: Option<String>| {
        let Some((_, description)) = text.and_then(|s| s.rsplit_once(": needs ")) else { return; };
        let index = needs.iter().position(|n| n.description == description).unwrap_or_else(|| {
            needs.push(Need { description: description.into(), features: vec![], operations: vec![] });
            needs.len() - 1
        });
        for feature in features {
            let list = &mut needs[index].features;
            if !list.contains(feature) { list.push(feature.clone()); }
        }
        if let Some(operation) = operation {
            let list = &mut needs[index].operations;
            if !list.contains(&operation) { list.push(operation); }
        }
    };
    for skipped in &plan.skipped {
        add(Some(&skipped.reason), std::slice::from_ref(&skipped.feature), None);
    }
    for setup in &plan.setups {
        for (n, op) in setup.operations.iter().enumerate() {
            add(op.tool_note.as_ref().and_then(Option::as_deref), op.cut.features().unwrap_or_default(),
                Some(format!("{}-{}-{}", setup.id, n + 1, op.cut.kind().name())));
        }
    }
    // Settling can remove a V-channel or a wall before tool resolution.
    for decision in plan.decisions.iter().flatten().filter(|d| d["outcome"] == "warning" && d.get("superseded_by").is_none()) {
        add(decision["says"].as_str(), &[], decision["subject"].as_str().map(String::from));
    }
    plan.tools_needed = Some(needs);
}

#[cfg(test)]
mod assembly_wording_tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn thread_mill_without_flute_length_uses_its_cutter_length_not_holder_reach() {
        let data: Value = serde_json::from_str(include_str!("../../../libraries/haas-vf2.json")).unwrap();
        let library = Library::new(&data).unwrap();
        let tool = data["tools"].as_array().unwrap().iter().find(|t| t["id"] == "tmM4").unwrap();
        assert!(tool.get("flute_length").is_none());
        assert_eq!(tool["tooth_height"], 0.693);
        // Exercise the refusal formatter independently of operation/tool selection.
        let plan: Plan = serde_json::from_value(json!({"setups":[{"id":"top","stock":{"min":[0,0,-64],"max":[20,20,0]},
            "operations":[{"kind":"notch","width":6.,"length":6.,"open":false,"outer":[[0,0,0],[6,0,0]],"top_z":0.,"floor_z":-64.,"wall_top":0.,"bottom":"blind"}]}],
            "covered_by_roughing":[],"skipped":[],"warnings":[]})).unwrap();
        let evidence = json!({"limit":"holder","holder_section":0,"depth_needed_mm":64.00001,
            "depth_available_mm":14.,"top_z_mm":-0.99999,"z_mm":-65.});
        let reason = refusal(&plan, &"top-1-notch".parse().unwrap(), "original evidence", Some("tmM4"), Some(&serde_json::from_value(evidence.clone()).unwrap()), &library).unwrap();
        assert_eq!(reason, "camlib refused the notch: tmM4's holder section 0 cannot clear the remaining stock beside the notch at Z-1 (tip Z-65): needs an end mill under ⌀5.95 with flutes of at least 1 mm and its holder at least 65 mm above the tip");
        assert_eq!(evidence["top_z_mm"], -0.99999, "display rounding must not alter evidence");
    }
}
