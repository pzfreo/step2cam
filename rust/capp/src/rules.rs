//! Rules kept as data in `rulebase/rules/`, and the small evaluator they need: arithmetic and
//! comparisons on numbers, over a rule's inputs, the tool's fields and the library's values.
//!
//! - `tools.json`: tool selectors. Each keeps the tools of a type that meet its conditions, in
//!   library order, says why each other one was turned down where it has words for that, and
//!   prefers one: the first, or the largest by some value (ties going to the first or the last).
//! - `sequence.json`: the order of operations in a setup.
//! - `process.json`: what becomes of a feature of each family.
//!
//! An expression is evaluated left to right with the usual precedence, so it does the same
//! floating-point arithmetic, in the same order, as the code it replaced.
use crate::plan::{r, show};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const TOOLS: &str = include_str!("../rulebase/rules/tools.json");
pub const SEQUENCE: &str = include_str!("../rulebase/rules/sequence.json");
pub const PROCESS: &str = include_str!("../rulebase/rules/process.json");

fn parsed(text: &'static str, name: &str, cell: &'static OnceLock<Value>) -> &'static Value {
    cell.get_or_init(|| serde_json::from_str(text).unwrap_or_else(|e| panic!("rulebase/rules/{name}: {e}")))
}
pub fn tools() -> &'static Value {
    static CELL: OnceLock<Value> = OnceLock::new();
    parsed(TOOLS, "tools.json", &CELL)
}
pub fn sequence() -> &'static Value {
    static CELL: OnceLock<Value> = OnceLock::new();
    parsed(SEQUENCE, "sequence.json", &CELL)
}
pub fn process() -> &'static Value {
    static CELL: OnceLock<Value> = OnceLock::new();
    parsed(PROCESS, "process.json", &CELL)
}

// ---- Expressions ----

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(f64),
    Name(String),
    Op(&'static str),
}

fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == 'e' || chars[i] == 'E' || ((chars[i] == '-' || chars[i] == '+') && (chars[i - 1] == 'e' || chars[i - 1] == 'E'))) {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push(Token::Num(s.parse().map_err(|_| format!("bad number {s} in {text:?}"))?));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            out.push(Token::Name(chars[start..i].iter().collect()));
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            let op = ["<=", ">=", "==", "&&"].into_iter().find(|o| *o == two);
            if let Some(op) = op {
                out.push(Token::Op(op));
                i += 2;
            } else {
                let op = ["+", "-", "*", "/", "(", ")", "<", ">"].into_iter().find(|o| o.starts_with(c)).ok_or_else(|| format!("unexpected {c:?} in {text:?}"))?;
                out.push(Token::Op(op));
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Looks a name up: a rule's input or value, a tool's field (`tool.diameter`), `inf`.
pub type Lookup<'a> = dyn Fn(&str) -> Result<f64, String> + 'a;

struct Parser<'a, 'b> {
    tokens: Vec<Token>,
    at: usize,
    lookup: &'b Lookup<'a>,
    /// False while parsing the right of an `&&` whose left is false: names are not looked up.
    live: bool,
}

impl Parser<'_, '_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }
    fn eat(&mut self, op: &str) -> bool {
        if self.peek() == Some(&Token::Op(match op {
            "+" => "+", "-" => "-", "*" => "*", "/" => "/", "(" => "(", ")" => ")", "<" => "<", ">" => ">",
            "<=" => "<=", ">=" => ">=", "==" => "==", "&&" => "&&", _ => return false,
        })) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    // all := cmp ("&&" cmp)*      (true is 1, false is 0)
    fn all(&mut self) -> Result<f64, String> {
        let mut v = self.cmp()?;
        while self.eat("&&") {
            let was = self.live;
            self.live = was && v != 0.;
            let w = self.cmp()?;
            self.live = was;
            v = if v != 0. && w != 0. { 1. } else { 0. };
        }
        Ok(v)
    }
    // cmp := sum (op sum)?
    fn cmp(&mut self) -> Result<f64, String> {
        let a = self.sum()?;
        for op in ["<=", ">=", "==", "<", ">"] {
            if self.eat(op) {
                let b = self.sum()?;
                let t = match op { "<=" => a <= b, ">=" => a >= b, "==" => a == b, "<" => a < b, _ => a > b };
                return Ok(if t { 1. } else { 0. });
            }
        }
        Ok(a)
    }
    fn sum(&mut self) -> Result<f64, String> {
        let mut v = self.product()?;
        loop {
            if self.eat("+") {
                v += self.product()?;
            } else if self.eat("-") {
                v -= self.product()?;
            } else {
                return Ok(v);
            }
        }
    }
    fn product(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            if self.eat("*") {
                v *= self.unary()?;
            } else if self.eat("/") {
                v /= self.unary()?;
            } else {
                return Ok(v);
            }
        }
    }
    fn unary(&mut self) -> Result<f64, String> {
        if self.eat("-") {
            return Ok(-self.unary()?);
        }
        if self.eat("(") {
            let v = self.all()?;
            if !self.eat(")") {
                return Err("missing )".into());
            }
            return Ok(v);
        }
        match self.tokens.get(self.at).cloned() {
            Some(Token::Num(n)) => {
                self.at += 1;
                Ok(n)
            }
            Some(Token::Name(name)) => {
                self.at += 1;
                if name == "inf" {
                    Ok(f64::INFINITY)
                } else if !self.live {
                    Ok(0.)
                } else {
                    (self.lookup)(&name)
                }
            }
            other => Err(format!("expected a value, found {other:?}")),
        }
    }
}

/// The value of an expression.
pub fn eval(text: &str, lookup: &Lookup) -> Result<f64, String> {
    let mut p = Parser { tokens: tokens(text)?, at: 0, lookup, live: true };
    let v = p.all()?;
    if p.at != p.tokens.len() {
        return Err(format!("trailing input in {text:?}"));
    }
    Ok(v)
}

/// Fills `{name}` in a template: numbers as messages show them (`{x}`), rounded first (`{x:r}`),
/// or a field of the tool as the library wrote it (`{tool.x:raw}`, `{tool.id}`).
pub fn fill(template: &str, tool: &Value, lookup: &Lookup) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}').ok_or("unclosed { in a template")? + open;
        let key = &rest[open + 1..close];
        let (name, how) = key.split_once(':').unwrap_or((key, ""));
        let field = name.strip_prefix("tool.");
        let text = match (field, how) {
            (Some(f), "raw") | (Some(f @ "id"), _) => match &tool[f] {
                Value::String(s) => s.clone(),
                v => v.to_string(),
            },
            (_, "r") => show(r(lookup(name)?)),
            _ => show(lookup(name)?),
        };
        out.push_str(&text);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

// ---- Tool selectors ----

/// A tool turned down, and why in full (`<tool id>: <reason>`).
pub type Rejection = (String, String);

/// The tool a selector prefers among those that qualify, and why each other one was turned down
/// (where the selector has words for that), in library order.
pub fn select<'a>(selector: &str, tools: &[&'a Value], inputs: &HashMap<&str, f64>, params: &HashMap<&str, f64>) -> Result<(Option<&'a Value>, Vec<Rejection>), String> {
    let rule = tools_rule(selector)?;
    let wanted = rule["tool_type"].as_str().ok_or_else(|| format!("selector {selector} has no tool_type"))?;
    let defaults = &rule["defaults"];
    let (mut kept, mut why): (Vec<&Value>, Vec<Rejection>) = (vec![], vec![]);
    for tool in tools.iter().copied().filter(|t| t["type"] == wanted) {
        let mut values: HashMap<String, f64> = HashMap::new();
        let lookup_in = |values: &HashMap<String, f64>, name: &str| -> Result<f64, String> {
            if let Some(field) = name.strip_prefix("tool.") {
                // A field the tool does not give as a number cannot qualify it: NaN fails every
                // comparison, so the tool is not chosen. A default stands in where the selector
                // gives one.
                return Ok(match tool[field].as_f64() {
                    Some(v) => v,
                    None => match &defaults[name] {
                        Value::String(s) if s == "inf" => f64::INFINITY,
                        v => v.as_f64().unwrap_or(f64::NAN),
                    },
                });
            }
            values.get(name).or_else(|| inputs.get(name)).or_else(|| params.get(name)).copied().ok_or_else(|| format!("selector {selector}: no value {name}"))
        };
        // Values worked out for each tool, in order: [name, expression] pairs.
        for pair in rule["let"].as_array().into_iter().flatten() {
            let (name, expr) = (pair[0].as_str().unwrap_or(""), pair[1].as_str().unwrap_or(""));
            let v = eval(expr, &|n| lookup_in(&values, n))?;
            values.insert(name.to_string(), v);
        }
        let mut ok = true;
        for condition in rule["keep"].as_array().into_iter().flatten() {
            let lookup = |n: &str| lookup_in(&values, n);
            if eval(condition["if"].as_str().unwrap_or(""), &lookup)? == 0. {
                if let Some(template) = condition["else_reject"].as_str() {
                    why.push((tool["id"].as_str().unwrap_or("").to_string(), fill(template, tool, &lookup)?));
                }
                ok = false;
                break;
            }
        }
        if ok {
            kept.push(tool);
        }
    }
    let prefer = &rule["prefer"];
    let chosen = match prefer["largest"].as_str() {
        None => kept.first().copied(),
        Some(by) => {
            let key = |t: &Value| -> f64 { t[by.strip_prefix("tool.").unwrap_or(by)].as_f64().unwrap_or(0.) };
            let last_wins = prefer["ties"] == "last";
            let mut best: Option<&Value> = None;
            for t in kept {
                let better = match best {
                    None => true,
                    Some(b) => key(t) > key(b) || (last_wins && key(t) == key(b)),
                };
                if better {
                    best = Some(t);
                }
            }
            best
        }
    };
    Ok((chosen, why))
}

fn tools_rule(selector: &str) -> Result<&'static Value, String> {
    tools()["selectors"].as_array().into_iter().flatten().find(|s| s["id"] == selector).ok_or_else(|| format!("no tool selector {selector} in rulebase/rules/tools.json"))
}

// ---- Sequence ----

/// Where an operation of this kind goes in its setup: operations are ordered by this, and those
/// of the same rank keep the order they were planned in.
pub fn rank(kind: &str) -> Result<usize, String> {
    sequence()["order"].as_array().into_iter().flatten().position(|group| group.as_array().into_iter().flatten().any(|k| k == kind)).ok_or_else(|| format!("rulebase/rules/sequence.json does not place {kind}"))
}

// ---- Process ----

/// The first case in `process.json` whose family and conditions hold for this feature: what
/// becomes of it. `holds` answers a named condition.
pub fn case_for(family: &str, holds: &mut dyn FnMut(&str) -> Result<bool, String>) -> Result<&'static Value, String> {
    for case in process()["cases"].as_array().into_iter().flatten() {
        if let Some(families) = case["families"].as_array() {
            if !families.iter().any(|f| f == family) {
                continue;
            }
        }
        let mut all = true;
        for condition in case["when"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !holds(condition)? {
                all = false;
                break;
            }
        }
        if all {
            return Ok(case);
        }
    }
    Err(format!("rulebase/rules/process.json has no case for a {family} feature"))
}
