//! Numbers in and out of records: rounding and display for the plan and its messages, operation
//! nouns, and reading numbers and flags from analysis records.

use super::*;

/// Rounded for the plan, with no negative zero: the nearest 4-decimal value.
pub fn r(x: f64) -> f64 {
    format!("{x:.4}").parse::<f64>().unwrap_or(x) + 0.0
}
pub(super) fn rp(p: V3) -> V3 {
    [r(p[0]), r(p[1]), r(p[2])]
}
/// A number as messages show it.
pub fn show(x: f64) -> String {
    format!("{x:?}")
}
/// Display a position with the plan's precision, without trailing zeroes.
pub fn display_position(value: f64) -> String {
    r(value).to_string()
}

/// Join a tool query to camlib's assembly clearance requirement in one sentence.
pub fn assembly_requirement(query: &str, part: &str, depth: f64) -> String {
    let join = if query.contains(" with ") { "and" } else { "with" };
    format!("{query} {join} its {part} at least {:.0} mm above the tip", (depth + 0.001).ceil())
}

/// A short operation noun for reader-facing reasons; identifiers belong to the trace.
pub fn operation_noun(kind: &str) -> &'static str {
    kind.parse::<crate::kind::OpKind>().map_or("operation", crate::kind::OpKind::noun)
}
pub(super) fn show3(p: V3) -> String {
    format!("[{}, {}, {}]", show(p[0]), show(p[1]), show(p[2]))
}

pub(super) fn nums(v: &Value) -> Result<Vec<f64>, String> {
    v.as_array()
        .ok_or("expected a list of numbers")?
        .iter()
        .map(|x| x.as_f64().ok_or_else(|| "expected a number".to_string()))
        .collect()
}
pub(super) fn v3(v: &Value) -> Result<V3, String> {
    let n = nums(v)?;
    if n.len() != 3 {
        return Err("expected three numbers".into());
    }
    Ok([n[0], n[1], n[2]])
}
pub(super) fn f(v: &Value, key: &str) -> Result<f64, String> {
    v.get(key).and_then(Value::as_f64).ok_or_else(|| format!("missing number {key}"))
}
/// Whether a record field says anything: present, and not null, false, empty or zero.
pub(super) fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.),
    }
}
