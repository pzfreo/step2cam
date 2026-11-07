//! The shop's resources as CAPP reads them: the tool library, and which tool can cut what.
//! Every number in it is the shop's (see `schemas/resources-1.schema.json`); nothing here is a
//! standard.
use crate::plan::{r, show};
use serde_json::Value;

/// Every tool needs these: no defaults, as holder size and cutting data are specific to the tool,
/// material and machine.
const TOOL_KEYS: [&str; 11] = [
    "id", "type", "number", "diameter", "overall_length", "stickout", "holder_length",
    "holder_diameter", "feed_mm_min", "spindle_rpm", "coolant",
];
/// How far out along a chamfer mill's cone the chamfer's lower edge is cut (mm), so the flank cuts.
pub const TIP_CLEARANCE: f64 = 0.5;

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
                "drill" => &["flute_length", "point_angle"],
                // A single-profile thread mill: `diameter` is the tooth's, on a narrower neck.
                "thread_mill" => &["tooth_height", "neck_diameter", "neck_length", "plunge_mm_min"],
                "chamfer_mill" => &["flute_length", "angle", "tip_diameter", "plunge_mm_min"],
                other => return Err(format!("tool {name}: type must be one of end_mill, drill, thread_mill, chamfer_mill, not {other:?}")),
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
        Ok(Self {
            data,
            tools,
            drill_tolerance: data.get("drill_tolerance_mm").and_then(Value::as_f64).unwrap_or(0.05),
            breakthrough: data.get("breakthrough_mm").and_then(Value::as_f64).unwrap_or(1.),
        })
    }
    pub fn end_mills(&self) -> Vec<&'a Value> {
        let mut mills: Vec<&Value> = self.tools.iter().copied().filter(|t| t["type"] == "end_mill").collect();
        mills.sort_by(|a, b| a["diameter"].as_f64().unwrap_or(0.).total_cmp(&b["diameter"].as_f64().unwrap_or(0.)));
        mills
    }
    pub fn clearing(&self, name: &str) -> Result<Option<&'a Value>, String> {
        if let Some(wanted) = self.data.get(format!("{name}_tool")).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            return self.tools.iter().copied().find(|t| id(t) == wanted).map(Some).ok_or_else(|| format!("no tool {wanted:?} in the library"));
        }
        Ok(self.end_mills().last().copied())
    }
    /// The drill closest to the diameter and never larger, at most `drill_tolerance_mm` under it.
    pub fn drill(&self, diameter: f64) -> Option<&'a Value> {
        let mut best: Option<&Value> = None;
        for t in self.tools.iter().copied().filter(|t| t["type"] == "drill") {
            let d = t["diameter"].as_f64().unwrap_or(0.);
            let gap = diameter - d;
            if (-1e-9..=self.drill_tolerance).contains(&gap) && best.is_none_or(|b| d > b["diameter"].as_f64().unwrap_or(0.)) {
                best = Some(t);
            }
        }
        best
    }
    /// The largest end mill that fits inside the diameter with room to helix.
    pub fn bore_mill(&self, diameter: f64) -> Option<&'a Value> {
        self.end_mills().into_iter().rfind(|t| t["diameter"].as_f64().unwrap_or(f64::MAX) < diameter - 0.05)
    }
    /// The largest end mill that fits a width and reaches a depth.
    pub fn pocket_mill(&self, width: f64, depth: f64) -> Option<&'a Value> {
        self.end_mills().into_iter().rfind(|t| t["diameter"].as_f64().unwrap_or(f64::MAX) < width - 0.05 && t["flute_length"].as_f64().unwrap_or(0.) >= depth)
    }
    /// The widest sharp V-bit that is no wider than a channel: a wider one would cut its walls. A
    /// flat tip could not reach the root.
    pub fn v_bit(&self, angle: f64) -> Option<&'a Value> {
        self.tools.iter().copied()
            .filter(|t| t["type"] == "chamfer_mill" && t["tip_diameter"].as_f64() == Some(0.) && t["angle"].as_f64().is_some_and(|a| a <= angle + 0.01))
            .max_by(|a, b| a["angle"].as_f64().unwrap_or(0.).total_cmp(&b["angle"].as_f64().unwrap_or(0.)))
    }
    /// A 90° chamfer mill wide enough for the width, with the tip clear of the edge.
    pub fn chamfer_mill(&self, width: f64) -> Option<&'a Value> {
        self.tools.iter().copied().find(|t| {
            t["type"] == "chamfer_mill"
                && t["angle"].as_f64() == Some(90.)
                && t["diameter"].as_f64().unwrap_or(0.) / 2. >= t["tip_diameter"].as_f64().unwrap_or(f64::MAX) / 2. + TIP_CLEARANCE + width
        })
    }
    /// The largest thread mill that can cut this thread: its pitch range (if it states one) takes
    /// the pitch, it fits the drilled hole, its neck clears the crests and it reaches the thread's
    /// bottom. With none, the reasons the others were refused.
    pub fn thread_mill(&self, op: &Value, stock_top: f64) -> Result<(Option<&'a Value>, Vec<String>), String> {
        let (major, start, pitch) = (num(op, "major")?, num(op, "start_diameter")?, num(op, "pitch")?);
        let (mut best, mut why): (Option<&Value>, Vec<String>) = (None, vec![]);
        for t in self.tools.iter().copied().filter(|t| t["type"] == "thread_mill") {
            let (diameter, neck) = (num(t, "diameter")?, num(t, "neck_diameter")?);
            let room = start / 2. - (major / 2. - diameter / 2.); // from the axis-side path to the drilled wall
            let reach = num(t, "tooth_height")? + num(t, "neck_length")?;
            let below = stock_top - num(op, "bottom_z")?;
            let lowest = t.get("min_pitch_mm").and_then(Value::as_f64).unwrap_or(0.);
            let highest = t.get("max_pitch_mm").and_then(Value::as_f64).unwrap_or(f64::INFINITY);
            if !(lowest <= pitch && pitch <= highest) {
                why.push(format!("{}: pitch {} outside its range", id(t), show(pitch)));
            } else if diameter >= start - 0.05 {
                why.push(format!("{}: ⌀{} does not fit the ⌀{} hole", id(t), t["diameter"], show(start)));
            } else if neck / 2. >= room - 0.01 {
                why.push(format!("{}: its ⌀{} neck would rub the thread crests", id(t), t["neck_diameter"]));
            } else if reach <= below + 0.01 {
                why.push(format!("{}: reaches {} mm, the thread ends {} mm below the stock top", id(t), show(reach), show(r(below))));
            } else if best.is_none_or(|b| diameter > b["diameter"].as_f64().unwrap_or(0.)) {
                best = Some(t);
            }
        }
        Ok((best, why))
    }
}
