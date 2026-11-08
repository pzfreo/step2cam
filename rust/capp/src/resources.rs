//! The shop's resources as CAPP reads them: the tool library, and which tool can cut what.
//! Every number in it is the shop's (see `schemas/resources-1.schema.json`); nothing here is a
//! standard.
use serde_json::Value;
use std::collections::HashMap;
use std::borrow::Cow;

fn same_property(a: &Value, b: &Value) -> bool {
    if a == b { return true; }
    let (Some(a), Some(b)) = (a.as_number(), b.as_number()) else { return false };
    if a.is_f64() == b.is_f64() { return false; }
    let (float, integer) = if a.is_f64() { (a, b) } else { (b, a) };
    let f = float.as_f64().expect("a JSON float");
    if f.fract() != 0. { return false; }
    // Compare the float's exact integer value, rather than rounding the integer to f64.
    let exact = format!("{f:.0}");
    exact.parse::<i64>().is_ok_and(|n| Some(n) == integer.as_i64())
        || exact.parse::<u64>().is_ok_and(|n| Some(n) == integer.as_u64())
}

/// Explicit workholding choice. Availability and selection are separate; #94
/// owns any future rule choosing between options.
pub(crate) fn workholding_option(data: &Value) -> Result<Option<&str>, String> {
    match data["overrides"].get("workholding_option") {
        None => Ok(None),
        Some(value) => match value.as_str() {
            Some(choice @ ("none" | "vise_flip_face")) => Ok(Some(choice)),
            _ => Err(format!("overrides.workholding_option must be \"none\" or \"vise_flip_face\", not {value}")),
        },
    }
}

/// Translate ISO symbols once at the library boundary. Downstream readers use current names.
/// Equal numeric pairs (including 6 and 6.0) are accepted; a disagreement names both inputs.
pub fn normalize(data: &Value) -> Result<Cow<'_, Value>, String> {
    workholding_option(data)?;
    let mut out = Cow::Borrowed(data);
    for (n, tool) in data["tools"].as_array().into_iter().flatten().enumerate() {
        for row in crate::reference::iso13399_fields()["fields"].as_array().expect("tool fields") {
            let Some(symbol) = row["symbol"].as_str() else { continue };
            if !row["types"].as_array().expect("tool types").contains(&tool["type"]) {
                continue;
            }
            let Some(value) = tool.get(symbol) else { continue };
            let field = row["field"].as_str().expect("tool field name");
            if let Some(current) = tool.get(field) && !same_property(current, value) {
                let name = tool["id"].as_str().map_or_else(|| format!("tools[{n}]"), String::from);
                return Err(format!("tool {name}: {field} {current} and {symbol} {value} disagree"));
            }
            let object = out.to_mut()["tools"][n].as_object_mut().expect("a tool with a property is an object");
            object.entry(field).or_insert_with(|| value.clone());
            object.remove(symbol);
        }
    }
    Ok(out)
}

/// Every tool needs these: no defaults, as holder size and cutting data are specific to the tool,
/// material and machine.
const TOOL_KEYS: [&str; 11] = [
    "id", "type", "number", "diameter", "overall_length", "stickout", "holder_length",
    "holder_diameter", "feed_mm_min", "spindle_rpm", "coolant",
];
/// How far out along a chamfer mill's cone the chamfer's lower edge is cut (mm), so the flank cuts.
pub const TIP_CLEARANCE: f64 = 0.5;
/// camlib drilling's tolerance; shared with the job adapter.
pub const DRILLING_TOLERANCE: f64 = 1e-5;
/// Include 1 nm for roundoff when camlib subtracts its guard from the flute length.
pub const DRILLING_GUARD: f64 = 16. * DRILLING_TOLERANCE + 1e-6;

/// Point height of the conical drill shape the job passes to camlib.
pub fn drill_tip_length(tool: &Value) -> Result<f64, String> {
    Ok(num(tool, "diameter")? / 2. / (num(tool, "point_angle")?.to_radians() / 2.).tan())
}

pub fn num(v: &Value, key: &str) -> Result<f64, String> {
    v.get(key).and_then(Value::as_f64).ok_or_else(|| format!("missing number {key}"))
}
pub fn id(t: &Value) -> &str {
    t["id"].as_str().unwrap_or("")
}

/// The tool library: which tool cuts what.
pub struct Library<'a> {
    pub data: &'a Value,
    tools: Vec<&'a Value>,
    /// Tools left out because the mill's spindle cannot hold them, and why.
    pub too_big: Vec<(&'a Value, String)>,
    drill_tolerance: f64,
    pub breakthrough: f64,
}

impl<'a> Library<'a> {
    pub fn new(data: &'a Value) -> Result<Self, String> {
        let tools: Vec<&Value> = data.get("tools").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
        if tools.is_empty() {
            return Err("the tool library has no tools".into());
        }
        for (n, tool) in tools.iter().enumerate() {
            let name = tool["id"].as_str().map_or_else(|| format!("tools[{n}]"), String::from);
            let kind = tool["type"].as_str().unwrap_or("");
            let extra: &[&str] = match kind {
                "end_mill" => &["flute_length", "plunge_mm_min", "stepdown_mm", "stepover_mm"],
                "drill" | "spot_drill" => &["flute_length", "point_angle"],
                // A single-profile thread mill: `diameter` is the tooth's, on a narrower neck.
                "thread_mill" => &["tooth_height", "neck_diameter", "neck_length", "plunge_mm_min"],
                "chamfer_mill" => &["flute_length", "angle", "tip_diameter", "plunge_mm_min"],
                other => return Err(format!("tool {name}: type must be one of end_mill, drill, spot_drill, thread_mill, chamfer_mill, not {other:?}")),
            };
            let missing: Vec<&str> = TOOL_KEYS.iter().chain(extra).filter(|k| tool.get(**k).is_none()).copied().collect();
            if !missing.is_empty() {
                return Err(format!("tool {name}: missing {}", missing.join(", ")));
            }
            let shank = tool.get("shank_diameter").and_then(Value::as_f64).unwrap_or(num(tool, "diameter")?);
            if num(tool, "holder_diameter")? < shank {
                return Err(format!("tool {name}: holder_diameter is smaller than the shank"));
            }
            if kind == "thread_mill" && num(tool, "neck_diameter")? >= num(tool, "diameter")? {
                return Err(format!("tool {name}: a thread mill's neck must be narrower than its tooth"));
            }
        }
        for (i, t) in tools.iter().enumerate() {
            if tools[..i].iter().any(|o| id(o) == id(t)) {
                return Err("tool ids must be unique".into());
            }
        }
        // A tool whose shank is wider than the mill's spindle holds is never chosen.
        let mut too_big = vec![];
        let mut tools = tools;
        if let Some(max) = crate::machines::mill(data).and_then(|m| m["spindle"]["max_tool_diameter_mm"].as_f64()) {
            tools.retain(|t| {
                let shank = t.get("shank_diameter").and_then(Value::as_f64).or(t["diameter"].as_f64()).unwrap_or(0.);
                let fits = shank <= max;
                if !fits {
                    too_big.push((*t, format!("{}: its ⌀{} shank is wider than the spindle holds (⌀{} at most), so it is not used", id(t), shank, max)));
                }
                fits
            });
        }
        Ok(Self {
            data,
            tools,
            too_big,
            drill_tolerance: data.get("drill_tolerance_mm").and_then(Value::as_f64).unwrap_or(0.05),
            breakthrough: data.get("breakthrough_mm").and_then(Value::as_f64).unwrap_or(1.),
        })
    }
    /// The tool the named selector in `rulebase/rules/tools.json` prefers, and why it turned
    /// the others down where it says.
    fn select(&self, selector: &str, inputs: &[(&str, f64)]) -> Result<(Option<&'a Value>, Vec<crate::rules::Rejection>), String> {
        let params = HashMap::from([("drill_tolerance_mm", self.drill_tolerance), ("tip_clearance_mm", TIP_CLEARANCE), ("drilling_guard_mm", DRILLING_GUARD)]);
        crate::rules::select(selector, &self.tools, &inputs.iter().copied().collect(), &params)
    }
    /// A selector can fail only if the shipped rules are wrong (tests/rules.rs runs them all); a
    /// tool library cannot make it fail, as a field that is not a number just disqualifies the
    /// tool.
    fn pick(&self, selector: &str, inputs: &[(&str, f64)]) -> Option<&'a Value> {
        self.select(selector, inputs).unwrap_or_else(|e| panic!("{e}")).0
    }
    /// The same selector expressed as a query for a tool to load or buy.
    pub fn needed(&self, selector: &str, inputs: &[(&str, f64)]) -> Result<String, String> {
        crate::rules::needed(selector, inputs, &[("drill_tolerance_mm", self.drill_tolerance), ("tip_clearance_mm", TIP_CLEARANCE), ("drilling_guard_mm", DRILLING_GUARD)])
    }
    pub fn clearing(&self, name: &str) -> Result<Option<&'a Value>, String> {
        if let Some(wanted) = self.data.get(format!("{name}_tool")).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            return self.tools.iter().copied().find(|t| id(t) == wanted).map(Some).ok_or_else(|| format!("no tool {wanted:?} in the library"));
        }
        Ok(self.pick("largest_end_mill", &[]))
    }
    /// Conservative full-breakthrough depth from the stock top. The selector adds each
    /// candidate's point even for tip-referenced holes and ignores exit obstructions that
    /// would shorten the job: a kept drill therefore always reaches camlib's target.
    pub fn drill_depth(&self, op: &Value, stock_top: f64) -> Result<f64, String> {
        Ok(stock_top - num(op, "bottom_z")?
            + if op["bottom"] == "through" { self.breakthrough.max(0.) } else { 0. })
    }
    pub(crate) fn spot(&self, diameter: f64, angle: f64, reach: f64, csink_angle: f64) -> Option<&'a Value> {
        self.pick("spot_drill", &[("diameter", diameter), ("angle", angle), ("reach", reach), ("csink_angle", csink_angle)])
    }
    pub fn drill(&self, diameter: f64, depth: f64) -> Option<&'a Value> {
        self.pick("drill", &[("diameter", diameter), ("depth", depth)])
    }
    pub fn bore_mill(&self, diameter: f64) -> Option<&'a Value> {
        self.pick("bore_mill", &[("diameter", diameter)])
    }
    pub fn pocket_mill(&self, width: f64, depth: f64) -> Option<&'a Value> {
        self.pick("pocket_mill", &[("width", width), ("depth", depth)])
    }
    pub fn v_bit(&self, angle: f64) -> Option<&'a Value> {
        self.pick("v_bit", &[("angle", angle)])
    }
    pub fn chamfer_mill(&self, width: f64) -> Option<&'a Value> {
        self.chamfer_mill_in(width, f64::INFINITY, width + TIP_CLEARANCE, f64::INFINITY)
    }
    pub fn chamfer_mill_in(&self, width: f64, maximum_diameter: f64, depth: f64, room: f64) -> Option<&'a Value> {
        self.pick("chamfer_mill", &[("width", width), ("maximum_diameter", maximum_diameter), ("depth", depth), ("room", room)])
    }
    /// What a selector made of every tool of its type, for the trace (`rules::considered`).
    pub fn considered(&self, selector: &str, inputs: &[(&str, f64)]) -> Vec<Value> {
        let params = HashMap::from([("drill_tolerance_mm", self.drill_tolerance), ("tip_clearance_mm", TIP_CLEARANCE), ("drilling_guard_mm", DRILLING_GUARD)]);
        crate::rules::considered(selector, &self.tools, &inputs.iter().copied().collect(), &params).unwrap_or_else(|e| panic!("{e}"))
    }
    /// The thread mill for this thread operation, and why the others were turned down.
    pub fn thread_mill(&self, op: &Value, stock_top: f64) -> Result<(Option<&'a Value>, Vec<crate::rules::Rejection>), String> {
        let inputs = [
            ("major", num(op, "major")?), ("start_diameter", num(op, "start_diameter")?), ("pitch", num(op, "pitch")?),
            ("stock_top", stock_top), ("bottom_z", num(op, "bottom_z")?),
        ];
        self.select("thread_mill", &inputs)
    }
}
