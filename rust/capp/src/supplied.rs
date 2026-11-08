//! One boundary for externally supplied choices, their provenance and their disposition.
use crate::trace::Trace;
use serde_json::{Value, json};

pub fn load(plan: &mut Value, resources: &Value) -> Result<(), String> {
    let mut entries = vec![];
    let legacy = &resources["overrides"]["clamping"];
    if !legacy.is_null() {
        for (subject, choice) in legacy.as_object().ok_or("overrides.clamping must be an object keyed by setup ID")? {
            entries.push(json!({"rule":"machine.clamping", "subject":subject, "choice":choice, "source":"user", "reason":"from deprecated resources.overrides.clamping"}));
        }
    }
    let mut merged: Vec<Value> = vec![];
    // Stored entries first, legacy next, explicit resources last: last writer wins.
    for list in [plan.get("supplied_decisions"), Some(&Value::Array(entries)), resources.get("decisions")].into_iter().flatten() {
        for entry in list.as_array().ok_or("supplied decisions must be an array")? {
            let mut entry = entry.clone();
            let obj = entry.as_object_mut().ok_or("a supplied decision must be an object")?;
            for field in ["rule", "subject", "source", "reason"] {
                if !obj.get(field).is_some_and(Value::is_string) { return Err(format!("supplied decision needs string {field}")); }
            }
            if !obj.contains_key("choice") { return Err("supplied decision needs choice".into()); }
            if obj.get("operation").is_some_and(|v| !v.is_string()) { return Err("supplied operation must be a string".into()); }
            let rule = obj["rule"].as_str().unwrap().split('@').next().unwrap().to_owned();
            if !crate::trace::rules().contains_key(&rule) { return Err(format!("unknown supplied rule {rule}")); }
            let source = obj["source"].as_str().unwrap();
            if !matches!(source, "user" | "agent") && !source.strip_prefix("agent:").is_some_and(|name| !name.trim().is_empty()) { return Err(format!("bad supplied source {source}")); }
            obj.insert("rule".into(), json!(rule));
            for field in ["accepted", "refusal_reason", "decision"] { obj.remove(field); }
            merged.retain(|old| !(old["rule"] == entry["rule"] && old["subject"] == entry["subject"] && old["operation"] == entry["operation"]));
            merged.push(entry);
        }
    }
    plan["supplied_decisions"] = json!(merged);
    Ok(())
}

impl Trace {
    pub(crate) fn supplied_for(&self, rule: &str, subject: &str, operation: Option<&str>) -> Option<usize> {
        self.supplied.iter().position(|s| s.get("accepted").is_none() && s["rule"] == rule && s["subject"] == subject && (s.get("operation").is_none() || s["operation"].as_str() == operation))
    }
    pub(crate) fn accept(&mut self, n: usize, decision: &str) {
        let s = &mut self.supplied[n];
        s["accepted"] = json!(true);
        s["decision"] = json!(decision);
        if let Some(d) = self.decisions.iter_mut().find(|d| d["id"] == decision) {
            d["source"] = s["source"].clone();
            d["reason"] = s["reason"].clone();
        }
    }
    pub(crate) fn refuse(&mut self, n: usize, reason: &str) {
        let s = &self.supplied[n];
        let subject = s["subject"].as_str().unwrap().to_owned();
        let text = format!("supplied {} for {subject} refused: {reason}; the rule's own choice is used", s["rule"].as_str().unwrap());
        let id = self.warn("requirement.supplied_refused", &subject, text);
        self.supplied[n]["accepted"] = json!(false);
        self.supplied[n]["refusal_reason"] = json!(reason);
        self.supplied[n]["decision"] = json!(id);
    }
    pub(crate) fn refuse_unused(&mut self) {
        for n in 0..self.supplied.len() {
            if self.supplied[n].get("accepted").is_none() {
                let rule = self.supplied[n]["rule"].as_str().unwrap();
                let unavailable = self.decisions.iter().rev().find(|d| d["subject"] == self.supplied[n]["subject"] && d["stage"] == "feasibility");
                let reason = if supported(rule) && let Some(d) = unavailable {
                    format!("operation unavailable: {}", d["says"].as_str().unwrap_or("failed feasibility checks"))
                } else if rule == "machine.clamping" || supported(rule) {
                    "subject or operation matches no decision".into()
                } else {
                    "rule is not suppliable this round (clearing uses roughing_tool/facing_tool; spot drills are not suppliable)".into()
                };
                self.refuse(n, &reason);
            }
        }
    }
}
pub fn supported(rule: &str) -> bool {
    matches!(rule, "tool.hole.drill_or_bore" | "tool.counterbore.bore_mill" | "tool.slot.bore_mill" | "tool.notch.pocket_mill" | "tool.thread_mill" | "tool.v_channel.v_bit" | "tool.edge_chamfer.chamfer_mill")
}
