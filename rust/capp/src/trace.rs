//! The decision trace: every choice capp makes, and the rule it made it by.
//!
//! The rules are listed in `rulebase/rules.json`, each with an id, a version, its stage, its
//! sources and where in the code it is applied. A decision cites one as `id@version`. A plan
//! carries its decisions in `decisions`; an operation names those behind it in `because`, and an
//! entry of `skipped` names its own in `decision`. `warnings` and `skipped` stay as they were, so
//! what reads them need not change; each entry of either has a decision of its own.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::OnceLock;

/// The rules, as shipped with the crate.
pub const RULES: &str = include_str!("../rulebase/rules.json");

/// (version, stage, has warning label) of each rule, by id.
pub(crate) fn rules() -> &'static HashMap<String, (u64, String, bool)> {
    static RULES_BY_ID: OnceLock<HashMap<String, (u64, String, bool)>> = OnceLock::new();
    RULES_BY_ID.get_or_init(|| {
        let doc: Value = serde_json::from_str(RULES).expect("rulebase/rules.json is JSON");
        doc["rules"]
            .as_array()
            .expect("rulebase/rules.json lists rules")
            .iter()
            .map(|r| {
                let id = r["id"].as_str().expect("a rule has an id").to_string();
                (id, (r["version"].as_u64().expect("a rule has a version"), r["stage"].as_str().expect("a rule has a stage").to_string(), r["warning_label"].as_str().is_some_and(|s| !s.trim().is_empty())))
            })
            .collect()
    })
}

/// Decisions as they are made, with the warnings and skipped features they give.
#[derive(Default)]
pub struct Trace {
    pub decisions: Vec<Value>,
    pub warnings: Vec<String>,
    pub skipped: Vec<Value>,
    pub supplied: Vec<Value>,
}

impl Trace {
    /// The trace a plan carries so far, to go on with: its decisions, and its warnings and
    /// skipped features, taken out of it until [`Trace::store`].
    pub fn take(plan: &mut Value) -> Self {
        let mut list = |key: &str| plan.get_mut(key).and_then(Value::as_array_mut).map(std::mem::take).unwrap_or_default();
        let (decisions, skipped) = (list("decisions"), list("skipped"));
        let warnings = list("warnings").into_iter().filter_map(|w| w.as_str().map(String::from)).collect();
        Self { decisions, warnings, skipped, supplied: plan["supplied_decisions"].as_array().cloned().unwrap_or_default() }
    }

    /// Puts the trace back into the plan.
    pub fn store(self, plan: &mut Value) {
        // Only a plan that has supplied decisions, or had the list already, carries it: storing a
        // trace must not add an empty list to a plan that never had one.
        if !self.supplied.is_empty() || plan.get("supplied_decisions").is_some() {
            plan["supplied_decisions"] = json!(self.supplied);
        }
        plan["decisions"] = json!(self.decisions);
        plan["warnings"] = json!(self.warnings);
        plan["skipped"] = json!(self.skipped);
    }

    /// Records a decision by `rule` about `subject` (a feature, an operation or a setup): its
    /// outcome and what it says, with anything more in `detail` (an object, merged in). Returns
    /// the decision's id.
    pub fn decide(&mut self, rule: &str, subject: &str, outcome: &str, says: &str, detail: Value) -> String {
        let (version, stage, _) = rules().get(rule).unwrap_or_else(|| panic!("rule {rule} is not in rulebase/rules.json"));
        let id = format!("d-{}", self.decisions.len() + 1);
        let mut decision = json!({
            "id": id, "stage": stage, "rule": format!("{rule}@{version}"), "subject": subject,
            "outcome": outcome, "says": says, "source": "rule",
        });
        if let (Some(out), Some(more)) = (decision.as_object_mut(), detail.as_object()) {
            out.extend(more.clone());
        }
        self.decisions.push(decision);
        id
    }

    /// A warning, by `rule`.
    pub fn warn(&mut self, rule: &str, subject: &str, text: String, mut detail: Value) -> String {
        assert!(rules().get(rule).is_some_and(|r| r.2), "warning rule {rule} needs a warning_label");
        if detail["features"].is_null() {
            detail.as_object_mut().unwrap().remove("features");
        }
        let id = self.decide(rule, subject, "warning", &text, detail);
        self.warnings.push(text);
        id
    }

    /// A feature not cut, by `rule`, and why.
    pub fn skip(&mut self, rule: &str, feature: &str, reason: String) -> String {
        let id = self.decide(rule, feature, "skipped", &reason, Value::Null);
        self.skipped.push(json!({"feature": feature, "reason": reason, "decision": id}));
        id
    }

    /// Marks earlier decisions about `subject` with this outcome as replaced by a later one.
    pub fn supersede(&mut self, subject: &str, outcome: &str, by: &str) {
        for d in self.decisions.iter_mut().filter(|d| d["subject"] == subject && d["outcome"] == outcome && d.get("superseded_by").is_none()) {
            if d["id"] != by {
                d["superseded_by"] = json!(by);
            }
        }
    }
}

/// Adds decisions to an operation's `because`.
pub fn because(op: &mut Value, ids: impl IntoIterator<Item = String>) {
    let list = op.as_object_mut().map(|o| o.entry("because").or_insert_with(|| json!([])));
    if let Some(Value::Array(list)) = list {
        list.extend(ids.into_iter().map(Value::String));
    }
}

/// Which rulebase a plan was made with: a hash (64-bit FNV-1a) over every rule and reference file
/// shipped in the crate, in a fixed order. Two plans with the same hash were made by the same rules
/// and data.
pub fn rulebase() -> Value {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let files = [RULES, crate::rules::TOOLS, crate::rules::SEQUENCE, crate::rules::PROCESS, crate::rules::WORKHOLDING]
        .into_iter()
        .chain(crate::reference::TABLES.iter().map(|(_, text)| *text));
    for text in files {
        for byte in text.bytes().chain([0u8]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    json!({"hash": format!("fnv1a64:{hash:016x}")})
}
