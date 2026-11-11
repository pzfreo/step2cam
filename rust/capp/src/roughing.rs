//! Neutral roughing policy; the camlib wire mapping belongs to the adapter.
use crate::{
    resources::num,
    trace::{Trace, because},
};
use serde_json::{Value, json};
use std::collections::HashMap;

const RULE: &str = "process.roughing.bounded_engagement";

pub struct Material<'a> {
    pub group: Option<&'a str>,
    pub name: Option<&'a str>,
}

pub fn choose(
    op: &mut Value,
    technology: &mut Value,
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
                return true;
            };
            let name = material.name.unwrap_or("").to_lowercase();
            words.iter().filter_map(Value::as_str).any(|word| {
                name.split(|c: char| !c.is_alphanumeric())
                    .any(|w| w == word)
            })
        });
    let Some(policy) = policy else {
        technology["clearing_strategy"] = json!("raster");
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
    op["raster_technology"] = technology.clone();
    let diameter = num(tool, "diameter")?;
    let stepdown = (num(tool, "flute_length")? * axial).min(diameter * max_d);
    let base = num(technology, "feed_mm_min")?;
    // Harvey: IPT = CT*D/(2*sqrt(D*ae-ae²)); no compensation at ae >= D/2.
    let factor = 1. / (2. * (width * (1. - width)).sqrt());
    let controller = crate::machines::mill(library)
        .and_then(|m| m["controller_settings"]["max_cutting_feed_mm_min"].as_f64());
    // Existing post-machine limits: rapid is the library's feed ceiling when no
    // controller cutting-feed limit is known (5000 mm/min when absent).
    let limit = controller.unwrap_or_else(|| library["rapid_mm_min"].as_f64().unwrap_or(5000.));
    technology["clearing_strategy"] = json!("bounded_engagement");
    technology["radial_width_mm"] = json!(diameter * width);
    technology["stepdown_mm"] = json!(stepdown);
    technology["feed_mm_min"] = json!((base * factor).min(limit));
    Ok(trace.decide(RULE, subject, "chosen", "Bounded-engagement roughing: sourced material radial width, flute-based stepdown (reach cap unchanged), radial chip thinning; entry/plunge feed unchanged", json!({"chosen":technology,"material_group":group,"chip_thinning_factor":factor,"base_feed_mm_min":base,"feed_limit_mm_min":limit,"feed_limit_source":if controller.is_some(){"controller_settings.max_cutting_feed_mm_min"}else{"library existing feed limit"},"policy":policy})))
}

/// Only an attempted adaptive roughing can trigger the one raster retry.
pub fn adaptive_setup(plan: &Value, operation: &str) -> Option<String> {
    for setup in plan["setups"].as_array()? {
        for (index, op) in setup["operations"].as_array()?.iter().enumerate() {
            if op["kind"] == "roughing"
                && op["technology"]["clearing_strategy"] == "bounded_engagement"
                && operation == format!("{}-{}-roughing", setup["id"].as_str()?, index + 1)
            {
                return setup["id"].as_str().map(String::from);
            }
        }
    }
    None
}

/// A flat on the same tool depends on roughing's prepared stock. Its existing
/// residual-depth refusal rejects that preparation, before the flat is excluded.
pub fn prepared_setup(plan: &Value, operation: &str) -> Option<String> {
    for setup in plan["setups"].as_array()? {
        let name = setup["id"].as_str()?;
        for (index, op) in setup["operations"].as_array()?.iter().enumerate() {
            if op["kind"] == "flats"
                && operation == format!("{name}-{}-flats", index + 1)
                && setup["operations"].as_array()?.iter().any(|rough| {
                    rough["kind"] == "roughing"
                        && rough["tool"] == op["tool"]
                        && rough["technology"]["clearing_strategy"] == "bounded_engagement"
                })
            {
                return Some(name.to_owned());
            }
        }
    }
    None
}

/// Restore original slotting data as well as strategy. A shallow raster retry must
/// never inherit adaptive's deep layers or chip-thinned feed.
pub fn fallbacks(plan: &mut Value, refusals: &HashMap<String, String>) {
    let mut trace = Trace::take(plan);
    for setup in plan["setups"].as_array_mut().into_iter().flatten() {
        let name = setup["id"].as_str().unwrap_or("").to_owned();
        let Some(reason) = refusals.get(&name) else {
            continue;
        };
        let subject = format!("{name}:roughing");
        let decision = trace.decide(RULE, &subject, "fallback", &format!("Adaptive roughing attempt refused: {reason}; retry raster with original cutting data and unchanged checks"), json!({"chosen":"raster","refusal":reason}));
        for earlier in &mut trace.decisions {
            if earlier["subject"] == subject
                && earlier["outcome"] == "chosen"
                && earlier["rule"]
                    .as_str()
                    .is_some_and(|r| r.split('@').next() == Some(RULE))
            {
                earlier["superseded_by"] = json!(decision);
            }
        }
        for op in setup["operations"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .filter(|o| o["kind"] == "roughing")
        {
            if op["raster_technology"].is_object() {
                op["technology"] = op["raster_technology"].clone();
                op["technology"]["clearing_strategy"] = json!("raster");
                because(op, [decision.clone()]);
            }
        }
    }
    trace.store(plan);
}
