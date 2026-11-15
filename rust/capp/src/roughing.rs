//! Neutral roughing policy; the camlib wire mapping belongs to the adapter.
use crate::{
    resources::num,
    trace::Trace,
};
use serde_json::{Value, json};
use crate::model::Operation;
use serde_json::Map;

const RULE: &str = "process.roughing.bounded_engagement";

pub struct Material<'a> {
    pub group: Option<&'a str>,
    pub name: Option<&'a str>,
}

pub fn choose(
    op: &mut Operation,
    technology: &mut Map<String, Value>,
    tool: &Value,
    material: Material<'_>,
    library: &Value,
    subject: &str,
    trace: &mut Trace,
) -> Result<String, String> {
    let group = material.group;
    let policy = group
        .and_then(|g| library["roughing_by_group"].get(g))
        .filter(|p| {
            let Some(words) = p["material_words"].as_array() else {
                return group != Some("N");
            };
            let name = material.name.unwrap_or("").to_lowercase();
            words.iter().filter_map(Value::as_str).any(|word| {
                name.split(|c: char| !c.is_alphanumeric())
                    .any(|w| w == word)
            })
        });
    let Some(policy) = policy else {
        technology.insert("clearing_strategy".into(), json!("raster"));
        return Ok(trace.decide(
            RULE,
            subject,
            "chosen",
            "Raster roughing: no sourced bounded-engagement policy for this material/group",
            json!({"chosen":"raster", "material_group":group}),
        ));
    };
    // A group override qualifies independently only when both feed and speed are
    // declared; a partial override must not relabel inherited aluminium feeds.
    let declared = group.is_some_and(|g| library["cutting_data_material_group"] == g);
    let qualified_override = group.is_some_and(|g| technology.get("source").is_some_and(|s| s == &format!("cutting_data.{g}"))
        && ["feed_mm_min", "spindle_rpm"].iter().all(|key| tool["cutting_data"][g][key].as_f64().is_some()));
    let reason = if !declared && !qualified_override {
        Some("Raster roughing: cutting data in use is not declared for this material group")
    } else if !crate::grip_force::has_reference(material.name.unwrap_or(""), group) {
        Some("Raster roughing: no applicable material force/power reference")
    } else { None };
    if let Some(reason) = reason {
        technology.insert("clearing_strategy".into(), json!("raster"));
        return Ok(trace.decide(RULE, subject, "chosen", reason, json!({"chosen":"raster", "material_group":group})));
    }
    let Some(spindle_power) = crate::machines::mill(library).and_then(|m| m["spindle"]["power_kw"].as_f64()).filter(|p| *p > 0.) else {
        technology.insert("clearing_strategy".into(), json!("raster"));
        return Ok(trace.decide(RULE, subject, "chosen", "Raster roughing: no spindle power for the adaptive power check", json!({"chosen":"raster", "material_group":group})));
    };
    let width = num(policy, "radial_width_diameter_fraction")?;
    let axial = num(policy, "axial_flute_fraction")?;
    let max_d = num(policy, "maximum_axial_diameters")?;
    if !(width > 0. && width < 0.5 && axial > 0. && axial <= 1. && max_d > 0.)
        || policy["chip_thinning"] != "radial_maximum_chip_thickness"
        || !policy["_sources"].is_object()
    {
        return Err("invalid or unsourced roughing_by_group policy".into());
    }
    op.raster_technology = Some(technology.clone());
    let diameter = num(tool, "diameter")?;
    let reach = num(tool, "flute_length")?.min(num(tool, "stickout")?) - crate::settle::REACH_MARGIN_MM;
    let mut stepdown = (num(tool, "flute_length")? * axial).min(diameter * max_d).min(reach);
    let base = technology.get("feed_mm_min").and_then(Value::as_f64).ok_or("roughing feed is missing")?;
    // Harvey: IPT = CT*D/(2*sqrt(D*ae-ae²)); no compensation at ae >= D/2.
    let factor = 1. / (2. * (width * (1. - width)).sqrt());
    let controller = crate::machines::mill(library)
        .and_then(|m| m["controller_settings"]["max_cutting_feed_mm_min"].as_f64());
    // Existing post-machine limits: rapid is the library's feed ceiling when no
    // controller cutting-feed limit is known (5000 mm/min when absent).
    let limit = controller.unwrap_or_else(|| library["rapid_mm_min"].as_f64().unwrap_or(5000.));
    // A retry must not undo the ceiling applied to the adaptive attempt when
    // a machine limit is below the tool's nominal feed.
    if base > limit {
        op.raster_technology.as_mut().unwrap().insert("feed_mm_min".into(), json!(limit));
    }
    let feed = (base * factor).min(limit);
    let rpm = technology.get("spindle_rpm").and_then(Value::as_f64).ok_or("roughing speed is missing")?;
    let flutes = tool["flutes"].as_f64().unwrap_or(crate::resources::DEFAULT_FLUTES as f64);
    let before = crate::grip_force::milling_force(stepdown, diameter * width, diameter, feed, rpm, flutes).ok_or("roughing power model has no positive chip thickness")?;
    // Step2cam judgement: reserve 40% of the stated spindle rating (which may
    // be peak power) for drive losses, transients and other unmodeled loads.
    let fraction = 0.6;
    let allowed = spindle_power * fraction;
    let original_depth = stepdown;
    if before.power_kw > allowed {
        stepdown = (stepdown * allowed / before.power_kw / 0.05).floor() * 0.05;
    }
    if stepdown < 0.05 {
        technology.insert("clearing_strategy".into(), json!("raster"));
        return Ok(trace.decide(RULE, subject, "chosen", "Raster roughing: adaptive power allowance cannot hold a 0.05 mm axial increment", json!({"chosen":"raster", "material_group":group})));
    }
    let after = crate::grip_force::milling_force(stepdown, diameter * width, diameter, feed, rpm, flutes).unwrap();
    let power = json!({"before_kw":before.power_kw,"after_kw":after.power_kw,"spindle_power_kw":spindle_power,"allowed_kw":allowed,"fraction":fraction,"stepdown_before_mm":original_depth,"stepdown_after_mm":stepdown,"quantum_mm":0.05,"basis":"mean tangential Kienzle force times cutting speed; 60% spindle rating is step2cam judgement"});
    technology.insert("clearing_strategy".into(), json!("bounded_engagement"));
    technology.insert("radial_width_mm".into(), json!(diameter * width));
    technology.insert("stepover_mm".into(), json!(diameter * width));
    technology.insert("stepdown_mm".into(), json!(stepdown));
    technology.insert("feed_mm_min".into(), json!((base * factor).min(limit)));
    Ok(trace.decide(RULE, subject, "chosen", "Bounded-engagement roughing: sourced material radial width, flute-based stepdown (reach cap unchanged), radial chip thinning and spindle-power axial cap; entry/plunge feed unchanged", json!({"chosen":technology,"material_group":group,"power":power,"chip_thinning_factor":factor,"base_feed_mm_min":base,"feed_limit_mm_min":limit,"feed_limit_source":if controller.is_some(){"controller_settings.max_cutting_feed_mm_min"}else{"library existing feed limit"},"policy":policy})))
}

/// Find the adaptive stock preparation rejected by the engine, including its dependent flat.
pub fn retry_setup(plan: &crate::model::Plan, id: &crate::kind::OpId, message: &str, strategy_refusal: bool) -> Option<String> {
    if !strategy_refusal { return None; }
    let setup = plan.setups.iter().find(|s| s.id == id.setup)?;
    let failed = setup.operations.get(id.number.checked_sub(1)?)?;
    setup.operations.iter().find(|o| matches!(o.cut, crate::model::Cut::Roughing(_))
        && o.technology.as_ref().is_some_and(|t| t.get("clearing_strategy").is_some_and(|s| s == "bounded_engagement")))?;
    (matches!(failed.cut, crate::model::Cut::Roughing(_))
        || (matches!(failed.cut, crate::model::Cut::Flats { .. })
            && message.contains("where roughing did not reach"))).then(|| setup.id.clone())
}

/// Restore slotting data after a refusal, retaining tool-selection evidence and depth checks.
pub fn fallbacks(plan: &mut crate::model::Plan, refusals: &std::collections::HashMap<String, String>) {
    let mut trace = Trace::take_plan(plan);
    for setup in &mut plan.setups {
        let Some(reason) = refusals.get(&setup.id) else { continue; };
        let subject = format!("{}:roughing", setup.id);
        for op in &mut setup.operations {
            if !matches!(op.cut, crate::model::Cut::Roughing(_)) { continue; }
            let Some(original) = &op.raster_technology else { continue; };
            let decision = trace.decide(RULE, &subject, "fallback", &format!("Adaptive roughing attempt refused: {reason}; retry raster with original cutting data and unchanged checks"), json!({"chosen":"raster","refusal":reason}));
            for earlier in &mut trace.decisions {
                if earlier["subject"] == subject && earlier["outcome"] == "chosen"
                    && earlier["rule"].as_str().is_some_and(|r| r.split('@').next() == Some(RULE)) {
                    earlier["superseded_by"] = json!(decision);
                }
            }
            let mut restored = original.clone();
            restored.insert("clearing_strategy".into(), json!("raster"));
            op.technology = Some(restored);
            crate::trace::because_op(op, [decision]);
        }
    }
    trace.store_plan(plan);
}
