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
    let stepdown = (num(tool, "flute_length")? * axial).min(diameter * max_d).min(reach);
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
    technology.insert("clearing_strategy".into(), json!("bounded_engagement"));
    technology.insert("radial_width_mm".into(), json!(diameter * width));
    technology.insert("stepdown_mm".into(), json!(stepdown));
    technology.insert("feed_mm_min".into(), json!((base * factor).min(limit)));
    Ok(trace.decide(RULE, subject, "chosen", "Bounded-engagement roughing: sourced material radial width, flute-based stepdown (reach cap unchanged), radial chip thinning; entry/plunge feed unchanged", json!({"chosen":technology,"material_group":group,"chip_thinning_factor":factor,"base_feed_mm_min":base,"feed_limit_mm_min":limit,"feed_limit_source":if controller.is_some(){"controller_settings.max_cutting_feed_mm_min"}else{"library existing feed limit"},"policy":policy})))
}

/// Find the adaptive stock preparation rejected by the engine, including its dependent flat.
pub fn retry_setup(plan: &crate::model::Plan, id: &crate::kind::OpId, message: &str) -> Option<String> {
    let setup = plan.setups.iter().find(|s| s.id == id.setup)?;
    let failed = setup.operations.get(id.number.checked_sub(1)?)?;
    let rough = setup.operations.iter().find(|o| matches!(o.cut, crate::model::Cut::Roughing(_))
        && o.technology.as_ref().is_some_and(|t| t.get("clearing_strategy").is_some_and(|s| s == "bounded_engagement")))?;
    (matches!(failed.cut, crate::model::Cut::Roughing(_))
        || (matches!(failed.cut, crate::model::Cut::Flats { .. }) && failed.tool == rough.tool
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
