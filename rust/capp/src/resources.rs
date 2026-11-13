//! The shop's resources as CAPP reads them: the tool library, and which tool can cut what.
//! Every number in it is the shop's (see `schemas/resources-1.schema.json`); nothing here is a
//! standard.
use crate::model::{Considered, Counterbore, Cut, Holes, Operation, Setup, Thread};
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

/// Legacy workholding switch; process translates it into a supplied setup.workholding choice.
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
/// Clearance kept between a through drill tip and part material beyond the exit.
pub const EXIT_MARGIN: f64 = 0.2;

/// Full-diameter travel beyond the exit needed to cover the tap lead, camlib's
/// one-pitch bottom clearance, and the same numerical guard in plan and job.
pub fn tap_through_travel(tool: &Value, pitch: f64) -> Result<f64, String> {
    Ok((num(tool, "chamfer_pitches")? + 1.) * pitch + DRILLING_GUARD)
}


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

/// Shop process gate shared by tap selection and needed-tool descriptions.
pub(crate) fn tap_policy_allows(major: f64, group: Option<&str>) -> bool {
    let policy = &crate::reference::tap_drills()["policy"];
    group == policy["iso513"].as_str()
        && major >= policy["min_major_mm"].as_f64().unwrap()
        && major <= policy["max_major_mm"].as_f64().unwrap()
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
                "tap" => &["pitch", "tap_type", "chamfer_form", "chamfer_pitches", "thread_length", "shank_diameter"],
                "thread_mill" => &["tooth_height", "neck_diameter", "neck_length", "plunge_mm_min"],
                "chamfer_mill" => &["flute_length", "angle", "tip_diameter", "plunge_mm_min"],
                other => return Err(format!("tool {name}: type must be one of end_mill, drill, spot_drill, thread_mill, tap, chamfer_mill, not {other:?}")),
            };
            let missing: Vec<&str> = TOOL_KEYS.iter().chain(extra).filter(|k| !(kind == "tap" && **k == "feed_mm_min") && tool.get(**k).is_none()).copied().collect();
            if !missing.is_empty() {
                return Err(format!("tool {name}: missing {}", missing.join(", ")));
            }
            if kind == "tap" {
                for key in ["diameter", "pitch", "chamfer_pitches", "thread_length", "shank_diameter", "overall_length", "stickout", "holder_length", "holder_diameter", "spindle_rpm"] {
                    if !num(tool, key)?.is_finite() || num(tool, key)? <= 0. { return Err(format!("tool {name}: {key} must be positive")); }
                }
                if !matches!(tool["tap_type"].as_str(), Some("cut" | "form"))
                    || tool.get("hand").is_some_and(|v| !matches!(v.as_str(), Some("right" | "left")))
                    || tool["chamfer_form"].as_str().is_none_or(str::is_empty)
                    || tool.get("flute_style").is_some_and(|v| !matches!(v.as_str(), Some("straight" | "spiral_flute" | "spiral_point"))) {
                    return Err(format!("tool {name}: invalid tap type, hand, chamfer form or flute style"));
                }
                if tool.get("feed_mm_min").is_some() || tool["cutting_data"].as_object().is_some_and(|g| g.values().any(|v| v.get("feed_mm_min").is_some())) {
                    return Err(format!("tool {name}: tap feed is derived from pitch × rpm"));
                }
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
    fn select(&self, selector: &str, inputs: &[(&str, f64)]) -> Result<(Option<&'a Value>, Vec<Considered>), String> {
        let params = HashMap::from([("drill_tolerance_mm", self.drill_tolerance), ("tip_clearance_mm", TIP_CLEARANCE), ("drilling_guard_mm", DRILLING_GUARD)]);
        crate::rules::select(selector, &self.tools, &inputs.iter().copied().collect(), &params)
    }
    /// Explicit last resort after the operation's fully qualifying alternatives are exhausted.
    fn select_deferred(&self, selector: &str, inputs: &[(&str, f64)]) -> Result<(Option<&'a Value>, Vec<Considered>), String> {
        let params = HashMap::from([("drill_tolerance_mm", self.drill_tolerance), ("tip_clearance_mm", TIP_CLEARANCE), ("drilling_guard_mm", DRILLING_GUARD)]);
        crate::rules::select_for_camlib(selector, &self.tools, &inputs.iter().copied().collect(), &params)
    }
    pub(crate) fn deferred_drill(&self, diameter: f64, depth: f64) -> Option<&'a Value> {
        self.select_deferred("drill", &[("diameter", diameter), ("depth", depth)]).unwrap_or_else(|e| panic!("{e}")).0
    }
    pub(crate) fn deferred_pocket_mill(&self, width: f64, depth: f64) -> Option<&'a Value> {
        self.select_deferred("pocket_mill", &[("width", width), ("depth", depth)]).unwrap_or_else(|e| panic!("{e}")).0
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
    /// would shorten the job. If none reaches, camlib judges the preferred fitting drill.
    pub(crate) fn drill_depth(&self, hole: &Holes, stock_top: f64) -> f64 {
        stock_top - hole.bottom_z
            + if hole.bottom == "through" { self.breakthrough.max(0.) } else { 0. }
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
    /// What a selector made of every tool of its type, for the trace (`rules::considered`), as
    /// JSON.
    pub fn considered(&self, selector: &str, inputs: &[(&str, f64)]) -> Vec<Value> {
        self.candidates(selector, inputs).iter().map(|c| serde_json::to_value(c).unwrap()).collect()
    }
    /// [`Library::considered`], as [`Considered`].
    pub(crate) fn candidates(&self, selector: &str, inputs: &[(&str, f64)]) -> Vec<Considered> {
        let params = HashMap::from([("drill_tolerance_mm", self.drill_tolerance), ("tip_clearance_mm", TIP_CLEARANCE), ("drilling_guard_mm", DRILLING_GUARD)]);
        crate::rules::considered(selector, &self.tools, &inputs.iter().copied().collect(), &params).unwrap_or_else(|e| panic!("{e}"))
    }
    /// Run the ordinary selector over exactly one supplied tool, with the operation's inputs.
    pub(crate) fn supplied_tool(&self, op: &Operation, top: f64, choice: &Value) -> Result<(&'a Value, Vec<Considered>), String> {
        self.supplied_thread_tool(op, top, choice, None, None)
    }
    pub(crate) fn supplied_thread_tool(&self, op: &Operation, top: f64, choice: &Value, setup: Option<&Setup>, group: Option<&str>) -> Result<(&'a Value, Vec<Considered>), String> {
        let name = choice.as_str().ok_or("tool choice must be a tool id string")?;
        let tool = self.tools.iter().copied().find(|t| id(t) == name).ok_or_else(|| {
            self.too_big.iter().find(|(t, _)| id(t) == name).map(|(_, why)| why.clone()).unwrap_or_else(|| format!("{name} is not in the library"))
        })?;
        if let Cut::Thread(thread) = &op.cut && tool["type"] == "tap" {
            if let Some(reason) = self.tap_rejection(tool, thread, top, self.thread_open_z(thread, setup, top)?, group)? {
                return Err(format!("{name}: {reason}"));
            }
            let (_, mut considered) = self.tap(thread, top, self.thread_open_z(thread, setup, top)?, group)?;
            considered.retain(|c| c.candidate != name);
            return Ok((tool, considered));
        }
        let (selector, inputs) = match &op.cut {
            Cut::Holes(hole) if tool["type"] == "drill" => ("drill", vec![("diameter", hole.diameter), ("depth", self.drill_depth(hole, top))]),
            Cut::Holes(Holes { diameter, .. }) | Cut::Counterbore(Counterbore { diameter, .. }) => ("bore_mill", vec![("diameter", *diameter)]),
            Cut::Slot(slot) => ("bore_mill", vec![("diameter", slot.width)]),
            Cut::Notch(notch) => ("pocket_mill", vec![("width", notch.width), ("depth", notch.top_z - notch.floor_z)]),
            Cut::Thread(thread) => ("thread_mill", vec![("major", thread.major), ("start_diameter", thread.start_diameter), ("pitch", thread.pitch), ("stock_top", top), ("bottom_z", thread.bottom_z)]),
            Cut::VChannel(channel) => ("v_bit", vec![("angle", channel.angle)]),
            Cut::EdgeChamfer(chamfer) => ("chamfer_mill", vec![("width", chamfer.width), ("maximum_diameter", chamfer.maximum_diameter.flatten().unwrap_or(f64::INFINITY)), ("depth", top - chamfer.top_z + chamfer.width + TIP_CLEARANCE), ("room", chamfer.room.flatten().unwrap_or(f64::INFINITY))]),
            Cut::Facing { .. } | Cut::Roughing(_) | Cut::Flats { .. } | Cut::SpotDrill(_) | Cut::WallFinish(_) => return Err("tool decision is not suppliable this round".into()),
        };
        let mut single = Self::new(self.data)?;
        single.tools = vec![tool];
        let (kept, why) = single.select_deferred(selector, &inputs)?;
        let kept = kept.ok_or_else(|| why.first().map(|c| format!("{}: {}", c.candidate, c.rejected)).unwrap_or_else(|| format!("{name}: wrong tool type or selector keep conditions reject it")))?;
        let mut queries = vec![(selector, inputs)];
        if let Cut::Holes(hole) = &op.cut {
            queries.push(if selector == "drill" { ("bore_mill", vec![("diameter", hole.diameter)]) }
                else { ("drill", vec![("diameter", hole.diameter), ("depth", self.drill_depth(hole, top))]) });
        }
        let mut considered = vec![];
        for (selector, inputs) in queries {
            let mut candidates = self.candidates(selector, &inputs);
            if let Some(preferred) = self.select(selector, &inputs)?.0
                && id(preferred) != name {
                candidates.push(Considered::passed_over(id(preferred), "qualifies, but supplied choice is used"));
            }
            for mut candidate in candidates {
                if candidate.candidate == name && !why.iter().any(|c| c.candidate == name) { continue; }
                if candidate.qualifies {
                    candidate.rejected = "qualifies, but supplied choice is used".into();
                }
                considered.push(candidate);
            }
        }
        Ok((kept, considered))
    }
    pub(crate) fn has_taps(&self) -> bool { self.tools.iter().any(|t| t["type"] == "tap") }

    /// Full-diameter floor expected from the hole operations. The job repeats this against
    /// the actual executed drill/bore, including exit restrictions and supplied choices.
    pub(crate) fn thread_open_z(&self, thread: &Thread, setup: Option<&Setup>, top: f64) -> Result<f64, String> {
        let mut floor = f64::NEG_INFINITY;
        let ops = setup.map_or(&[][..], |s| &s.operations);
        for feature in &thread.features {
            let Some(hole) = ops.iter().find_map(|h| match &h.cut { Cut::Holes(hole) if hole.features.contains(feature) => Some(hole), _ => None }) else { return Ok(f64::INFINITY) };
            let mut z = hole.bottom_z;
            if hole.bottom == "through" {
                z -= self.breakthrough.max(0.);
            } else if hole.bottom != "drill_point"
                && let Some(drill) = self.drill(hole.diameter, self.drill_depth(hole, top)) {
                z += drill_tip_length(drill)?;
            }
            floor = floor.max(z);
        }
        Ok(floor)
    }

    /// Full-diameter space past a through exit after allowing for the selected
    /// pre-drill's point and the job's exit margin. A bored hole has no drill point.
    pub(crate) fn tap_exit_room(&self, thread: &Thread, top: f64) -> Result<Option<f64>, String> {
        if !thread.through { return Ok(None); }
        let Some(clear) = thread.exit_clear_mm else { return Ok(None); };
        let diameter = thread.start_diameter;
        let depth = top - thread.bottom_z + self.breakthrough.max(0.);
        let point = self.drill(diameter, depth).map(drill_tip_length).transpose()?.unwrap_or(0.);
        Ok(Some((clear - EXIT_MARGIN).max(0.) - point))
    }

    /// Process rule tool.thread.tap_or_mill; first qualifying tap in library order.
    pub(crate) fn tap(&self, thread: &Thread, top: f64, open_z: f64, group: Option<&str>) -> Result<(Option<&'a Value>, Vec<Considered>), String> {
        let mut chosen = None;
        let mut considered = vec![];
        for tool in self.tools.iter().copied().filter(|t| t["type"] == "tap") {
            let reason = self.tap_rejection(tool, thread, top, open_z, group)?;
            if let Some(reason) = reason {
                considered.push(Considered::rejected(id(tool), reason));
            } else if chosen.is_none() { chosen = Some(tool); }
            else { considered.push(Considered::passed_over(id(tool), "qualifies, but first matching tap is preferred")); }
        }
        Ok((chosen, considered))
    }

    fn tap_rejection(&self, tool: &Value, thread: &Thread, top: f64, open_z: f64, group: Option<&str>) -> Result<Option<String>, String> {
        let major = thread.major;
        let pitch = thread.pitch;
        let why = if !tap_policy_allows(major, group) {
            Some("thread milling preferred: step2cam tap policy is M3–M6 in ISO 513 group N".into())
        } else if num(tool, "diameter")? != major || num(tool, "pitch")? != pitch {
            Some(format!("nominal diameter/pitch do not match M{major}×{pitch}"))
        } else if tool.get("hand").and_then(Value::as_str).unwrap_or("right") != thread.hand.as_deref().unwrap_or("right") {
            Some("thread hand does not match".into())
        } else {
            let Some(drill) = crate::reference::tap_drill(major, pitch, tool["tap_type"] == "form") else {
                return Ok(Some("no verified tap drill reference for this diameter/pitch; use thread milling".into()));
            };
            let start = thread.start_diameter;
            let tip = thread.bottom_z - num(tool, "chamfer_pitches")? * pitch;
            let tol = crate::reference::tap_drills()["diameter_tolerance_mm"].as_f64().unwrap();
            if (start - drill).abs() > tol + 1e-9 {
                Some(format!("wrong pre-drill: {} tap requires ⌀{drill:.2} ±{tol} mm; model hole is ⌀{start} mm", tool["tap_type"].as_str().unwrap()))
            } else if !thread.through && tool["flute_style"] == "spiral_point" {
                Some("spiral point pushes chips ahead: through holes only".into())
            } else if let Some(room) = self.tap_exit_room(thread, top)?
                && tap_through_travel(tool, pitch)? > room + 1e-9 {
                Some(format!("through tapping needs {:.6} mm full-diameter travel beyond the exit (chamfer + one pitch + numerical guard); available {room:.6} mm after the exit margin and drill point", tap_through_travel(tool, pitch)?))
            } else if !thread.through && tip - pitch < open_z - 1e-9 {
                Some(format!("full thread {:.2} mm + chamfer {:.2} mm + bottom clearance {pitch:.2} mm requires {:.2} mm; drilled full diameter is {:.2} mm", thread.top_z - thread.bottom_z, num(tool, "chamfer_pitches")? * pitch, thread.top_z - tip + pitch, thread.top_z - open_z))
            } else if num(tool, "thread_length")? < top - tip + DRILLING_GUARD || num(tool, "stickout")? < top - tip + DRILLING_GUARD {
                Some(format!("thread length and stickout must reach {:.3} mm below stock top (including chamfer and guard)", top - tip + DRILLING_GUARD))
            } else { None }
        };
        Ok(why)
    }

    /// The thread mill for this thread operation, and why the others were turned down.
    pub fn thread_mill(&self, thread: &Thread, stock_top: f64) -> Result<(Option<&'a Value>, Vec<Considered>), String> {
        let inputs = [
            ("major", thread.major), ("start_diameter", thread.start_diameter), ("pitch", thread.pitch),
            ("stock_top", stock_top), ("bottom_z", thread.bottom_z),
        ];
        self.select("thread_mill", &inputs)
    }
}
