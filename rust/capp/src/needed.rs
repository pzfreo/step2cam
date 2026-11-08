//! Missing-tool queries. These describe resources only; they never change tool selection.
use crate::resources::{Library, num};
use serde_json::{Value, json};

/// Round reach upwards with a micron of clearance. This exceeds camlib's 16 × 1e-5 mm
/// numerical guard in step2cam jobs, including when depth is an exact integer.
pub(crate) fn reach(depth: f64) -> String {
    format!("{:.0}", (depth + 0.001).ceil())
}

pub(crate) fn for_operation(op: &Value, top: f64, library: &Library, required: Option<f64>) -> Result<Option<String>, String> {
    let kind = op["kind"].as_str().unwrap_or("");
    let bottom = op["bottom_z"].as_f64().or(op["floor_z"].as_f64()).or(op["root_z"].as_f64()).unwrap_or(top);
    let depth = required.unwrap_or_else(|| top - bottom + if op["bottom"] == "through" { library.breakthrough.max(0.) } else { 0. });
    let long = format!("with flutes of at least {} mm", reach(depth));
    let mill = |selector, inputs: &[(&str, f64)]| -> Result<String, String> {
        Ok(format!("{} {long}", library.needed(selector, inputs)?))
    };
    Ok(Some(match kind {
        "spot_drill" => library.needed("spot_drill", &[("diameter", num(op, "diameter")?), ("angle", num(op, "angle")?), ("reach", num(op, "reach")?)])?,
        "holes" => format!("{}, or {}",
            library.needed("drill", &[("diameter", num(op, "diameter")?), ("depth", library.drill_depth(op, top)?.max(depth))])?,
            mill("bore_mill", &[("diameter", num(op, "diameter")?)])?),
        "counterbore" => mill("bore_mill", &[("diameter", num(op, "diameter")?)])?,
        "slot" => mill("bore_mill", &[("diameter", num(op, "width")?)])?,
        "notch" => mill("pocket_mill", &[("width", num(op, "width")?), ("depth", depth)])?,
        "facing" | "roughing" | "flats" | "wall_finish" => mill("largest_end_mill", &[])?,
        "edge_chamfer" => {
            let width = num(op, "width")?;
            let axial = top - num(op, "top_z")? + width + crate::resources::TIP_CLEARANCE;
            let query = library.needed("chamfer_mill", &[("width", width), ("maximum_diameter", op["maximum_diameter"].as_f64().unwrap_or(f64::INFINITY)), ("depth", axial)])?;
            if required.is_some() { format!("{query}, with zero tip diameter and flutes of at least {} mm", reach(depth)) } else {
                let query = format!("{query}, with flutes of at least {:.2} mm", ((axial+0.00016)*100.).ceil()/100.);
                let query = match op["room"].as_f64() {
                    Some(room) => format!("{query}, with cutter radius + tip radius + 0.5 mm at most {} mm", crate::plan::show(room)),
                    None => query,
                };
                match op["maximum_diameter"].as_f64() {
                    Some(maximum) => format!("{query}, diameter at most {} mm for this loop", crate::plan::show(maximum)),
                    None => query,
                }
            }
        },
        "v_channel" => format!("{} {long}", library.needed("v_bit", &[("angle", num(op, "angle")?)])?),
        "thread" => library.needed("thread_mill", &[("major", num(op, "major")?), ("start_diameter", num(op, "start_diameter")?),
            ("pitch", num(op, "pitch")?), ("stock_top", top), ("bottom_z", bottom)])?,
        _ => return Ok(None),
    }))
}

/// Only camlib's explicit depth/flute refusal is a missing tool. In particular a stepdown
/// refusal can be fixed with cutting data, and says nothing about the required tool's reach.
pub(crate) fn refusal(plan: &Value, operation: &str, message: &str, refused_tool: Option<&str>, library: &Library) -> Result<String, String> {
    let mut parts = operation.splitn(3, '-');
    let setup_id = parts.next().unwrap_or("");
    let number = parts.next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
    let Some(setup) = plan["setups"].as_array().into_iter().flatten().find(|s| s["id"] == setup_id) else { return Ok(message.into()); };
    let Some(op) = number.checked_sub(1).and_then(|n| setup["operations"].get(n)) else { return Ok(message.into()); };
    let top = setup["stock"]["max"][2].as_f64().ok_or("bad stock")?;
    let depth = message.split_once(" exceeds the cutting length ").and_then(|(before, _)| {
        ["bore depth ", "hole depth ", "contour depth ", "pocket depth ", "slot depth ", "facing depth ", "parameters: depth "]
            .iter().find_map(|prefix| before.split_once(prefix).and_then(|(_, rest)| rest.split_whitespace().next()?.parse::<f64>().ok()))
    }).or_else(|| {
        if op["kind"] != "edge_chamfer" || !message.contains("is deeper below the stock top than the cutting length ") { return None; }
        let tip = message.split_once("tool tip Z")?.1.split_whitespace().next()?.parse::<f64>().ok()?;
        Some(top - tip)
    });
    let Some(depth) = depth.filter(|d| d.is_finite() && *d > 0.) else { return Ok(message.into()); };
    let tool_id = refused_tool.or(op["tool"].as_str()).unwrap_or("the tool");
    let tool = library.data["tools"].as_array().and_then(|ts| ts.iter().find(|t| t["id"] == tool_id));
    let noun = crate::plan::operation_noun(op["kind"].as_str().unwrap_or(""));
    let plain = tool.and_then(|t| t["flute_length"].as_f64()).map(|flutes| format!("camlib refused the {noun}: the cut is {} mm below the stock top and {tool_id}'s flutes are {} mm", crate::plan::show(depth), crate::plan::show(flutes))).unwrap_or_else(|| message.into());
    Ok(match for_operation(op, top, library, Some(depth))? {
        Some(need) => format!("{plain}: needs {need}"),
        None => message.into(),
    })
}

/// Derive the index from the live reasons, so settling repeatedly cannot retain obsolete needs.
pub(crate) fn collect(plan: &mut Value) {
    let mut needs: Vec<Value> = vec![];
    let mut add = |text: &Value, features: Vec<Value>, operation: Option<String>| {
        let Some((_, description)) = text.as_str().and_then(|s| s.rsplit_once(": needs ")) else { return; };
        let index = needs.iter().position(|n| n["description"] == description).unwrap_or_else(|| {
            needs.push(json!({"description": description, "features": [], "operations": []}));
            needs.len() - 1
        });
        for feature in features {
            let list = needs[index]["features"].as_array_mut().unwrap();
            if !list.contains(&feature) { list.push(feature); }
        }
        if let Some(operation) = operation {
            let list = needs[index]["operations"].as_array_mut().unwrap();
            let operation = json!(operation);
            if !list.contains(&operation) { list.push(operation); }
        }
    };
    for skipped in plan["skipped"].as_array().into_iter().flatten() {
        add(&skipped["reason"], vec![skipped["feature"].clone()], None);
    }
    for setup in plan["setups"].as_array().into_iter().flatten() {
        for (n, op) in setup["operations"].as_array().into_iter().flatten().enumerate() {
            add(&op["tool_note"], op["features"].as_array().cloned().unwrap_or_default(),
                Some(format!("{}-{}-{}", setup["id"].as_str().unwrap_or(""), n + 1, op["kind"].as_str().unwrap_or(""))));
        }
    }
    // Settling can remove a V-channel or a wall before tool resolution.
    for decision in plan["decisions"].as_array().into_iter().flatten().filter(|d| d["outcome"] == "warning" && d.get("superseded_by").is_none()) {
        add(&decision["says"], vec![], decision["subject"].as_str().map(String::from));
    }
    plan["tools_needed"] = json!(needs);
}
