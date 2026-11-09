//! Compare consequences of independently simulated runs; accept externally supplied choices.
use crate::{supplied, trace::Trace};
use serde_json::{Value, json};

pub const RULE: &str = "setup.workholding";

/// Lexicographic policy. Stable input order breaks an exact tie.
pub fn compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    b["finished"]
        .as_bool()
        .cmp(&a["finished"].as_bool())
        .then_with(|| {
            a["features_left_out"]
                .as_array()
                .map(Vec::len)
                .cmp(&b["features_left_out"].as_array().map(Vec::len))
        })
        .then_with(|| a["setups"].as_u64().cmp(&b["setups"].as_u64()))
        .then_with(|| {
            a["extra_stock_mm"]
                .as_f64()
                .unwrap_or(0.)
                .total_cmp(&b["extra_stock_mm"].as_f64().unwrap_or(0.))
        })
        .then_with(|| {
            a["estimated_seconds"]
                .as_f64()
                .unwrap_or(0.)
                .total_cmp(&b["estimated_seconds"].as_f64().unwrap_or(0.))
        })
}

/// Reasons use the first criterion that separates an alternative from the default.
pub fn loses(a: &Value, best: &Value) -> &'static str {
    if a["finished"] != best["finished"] {
        "does not finish the part"
    } else if a["features_left_out"].as_array().map(Vec::len)
        != best["features_left_out"].as_array().map(Vec::len)
    {
        "more features left out"
    } else if a["setups"] != best["setups"] {
        "more setups"
    } else if a["extra_stock_mm"] != best["extra_stock_mm"] {
        "more extra stock"
    } else if a["estimated_seconds"] != best["estimated_seconds"] {
        "longer estimated machining time"
    } else {
        "equal consequences; stable candidate order breaks the tie"
    }
}

/// Workholding is chosen after execution checks, outside finish's per-operation supplied choices.
/// Other supplied decisions have already been checked by each run.
pub fn choose(
    plan: &mut Value,
    resources: &Value,
    candidates: &[Value],
) -> Result<Option<usize>, String> {
    let mut requests = resources.clone();
    requests["decisions"] = json!(
        resources["decisions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["rule"]
                .as_str()
                .is_some_and(|r| r.split('@').next() == Some(RULE)))
            .cloned()
            .collect::<Vec<_>>()
    );
    if let Some(choice) = crate::resources::workholding_option(resources)? {
        requests["decisions"].as_array_mut().unwrap().insert(
            0,
            json!({"rule":RULE,"subject":"part",
            "choice":if choice=="none" {"vise_ordinary"} else {choice},"source":"user",
            "reason":"from resources.overrides.workholding_option"}),
        );
    }
    // Isolate this boundary from tool/clamping entries (including legacy clamping).
    requests.as_object_mut().unwrap().remove("overrides");
    let mut incoming = json!({});
    supplied::load(&mut incoming, &requests)?;
    let mut trace = Trace::take(plan);
    trace.supplied.extend(
        incoming["supplied_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .cloned(),
    );
    let default = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c["available"] == true)
        .min_by(|(_, a), (_, b)| compare(a, b))
        .map(|(i, _)| i);
    let mut chosen = default;
    let supplied = trace.supplied_for(RULE, "part", None);
    if let Some(n) = supplied {
        let choice = &trace.supplied[n]["choice"];
        if let Some(i) = candidates
            .iter()
            .position(|c| c["id"] == *choice && c["available"] == true)
        {
            chosen = Some(i);
        } else {
            let reason = candidates
                .iter()
                .find(|c| c["id"] == *choice)
                .and_then(|c| c["reason"].as_str())
                .unwrap_or_else(|| if matches!(choice.as_str(),Some("vise_ordinary" | "vise_flip_face"))
                    && crate::machines::dimensioned_vise(resources).is_none() {
                    "no dimensioned vise in the first mill (jaw width, height and opening required)"
                } else {"unknown workholding candidate"})
                .to_string();
            trace.refuse(n, &reason);
        }
    }
    let id = chosen
        .map(|i| candidates[i]["id"].clone())
        .unwrap_or(Value::Null);
    let decision = trace.decide(RULE,"part",if chosen.is_some() {"chosen"} else {"none"},
        if chosen.is_some() {"finishes the part, then fewest features left out, fewest setups, least extra stock, shortest estimated time"}
        else {"no validated workholding candidate is available; legacy plan retained for review, holding is not validated"},
        json!({"chosen":id,"default":default.map(|i|candidates[i]["id"].clone())}));
    if let Some(n) = supplied.filter(|n| trace.supplied[*n].get("accepted").is_none()) {
        trace.accept(n, &decision);
    }
    // Reject unmatched workholding subjects/operations through the normal refusal path.
    trace.refuse_unused();
    let source = trace
        .decisions
        .iter()
        .find(|d| d["id"] == decision)
        .unwrap()["source"]
        .clone();
    plan["workholding"] = json!({"chosen":id,"source":source,"candidates":candidates});
    trace.store(plan);
    Ok(chosen)
}
