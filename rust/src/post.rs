//! A camlib job -> checked LinuxCNC G-code, in-process.
//!
//! camlib plans and simulates each motion against the stock, with its stock-contact checks. Every
//! block written is then read back from its text, and camlib's intent checker compares what it says
//! with the source, event by event: motions with camlib's decoder, the other blocks with
//! `decode_control` here. The fixed preamble and end are compared word for word, and the plane an
//! arc is decoded in is read from the line written. A block that reads differently from the plan is
//! a fault in this post, so it stops the whole program: see [`Stop`]. An arc that cannot be written
//! exactly, or that the machine does not accept, becomes line segments within the chord tolerance.
use camlib::{
    motion::{Capabilities, Coolant, Curve, Event, Motion, MoveKind, Plane, Program as Header, SpindleDirection, Vec3, WorkFrame},
    operation::OperationPlan,
    post_decode::{Config, Decoded, Decoder, Rule, format_words},
    post_intent::{IntentChecker, Settings},
    post_lowering::Options as Lowering,
    post_rounding::{RadiusTolerance, circular_words},
    material::Assessment,
    stream::{DEFAULT_FRAME_LIMIT, StreamManifest, StreamProducer},
    stream_verification::{Binding, Check},
    verification::Status,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

const GEOMETRY_TOLERANCE: f64 = 1e-5;
const ARC_SAMPLES: u32 = 16;
/// How many lowered segments of an arc are checked at a time: camlib's most.
const LOWERED_BATCH: usize = 64;
const DEPTH_TOLERANCE: f64 = 0.01;
const RADIUS_TOLERANCE: RadiusTolerance = RadiusTolerance { absolute_mm: 0.00127, relative: 0.001 };
/// Metric, absolute, incremental arc centres, XY, feed per minute, fixed spindle speed, no cutter
/// compensation, no length offset, no cycle, no G92 offset, exact stop, spindle and coolant off.
const PREAMBLE: [&str; 13] = ["G21", "G90", "G91.1", "G17", "G94", "G97", "G40", "G49", "G80", "G92.1", "G61.1", "M5", "M9"];

fn default_tolerance() -> f64 {
    0.01
}
fn default_feed_tolerance() -> f64 {
    0.1
}
fn default_true() -> bool {
    true
}

/// The machine block of the tool library, as far as this post needs it.
#[derive(Clone, Debug, Deserialize)]
pub struct Machine {
    pub decimals: usize,
    #[serde(default = "default_units")]
    pub output_units: String,
    pub max_feed_mm_min: f64,
    pub max_spindle_rpm: f64,
    #[serde(default)]
    pub planes: Vec<String>,
    #[serde(default = "default_true")]
    pub arcs: bool,
    #[serde(default = "default_true")]
    pub helices: bool,
    pub max_sweep_rad: Option<f64>,
    pub max_tool_number: Option<u32>,
    pub max_length_register: Option<u32>,
    #[serde(default = "default_tolerance")]
    pub tolerance_mm: f64,
    #[serde(default = "default_feed_tolerance")]
    pub feed_tolerance_mm_min: f64,
    #[serde(default = "default_true")]
    pub tool_changes: bool,
    #[serde(default = "default_true")]
    pub flood: bool,
    #[serde(default = "default_true")]
    pub mist: bool,
}

impl Machine {
    /// What the checker holds the program to.
    fn capabilities(&self) -> Result<Capabilities, String> {
        let planes = if self.planes.is_empty() { vec!["xy".to_string(), "xz".into(), "yz".into()] } else { self.planes.clone() };
        Ok(Capabilities {
            id: "step2cam_linuxcnc".into(),
            planes: planes.iter().map(|p| serde_json::from_value(serde_json::json!(p)).map_err(|e| format!("machine plane {p}: {e}"))).collect::<Result<_, String>>()?,
            arcs: self.arcs,
            // A machine with no arcs has no helices, whatever the library says.
            helices: self.arcs && self.helices,
            max_sweep_rad: self.max_sweep_rad.unwrap_or(1e6),
            max_feed_mm_min: self.max_feed_mm_min,
            max_spindle_rpm: self.max_spindle_rpm,
            max_tool_number: self.max_tool_number.unwrap_or(u32::MAX),
            max_length_register: self.max_length_register.unwrap_or(u32::MAX),
            max_work_offset: 6,
            tool_changes: self.tool_changes,
            flood: self.flood,
            mist: self.mist,
        })
    }
}
fn default_units() -> String {
    "mm".into()
}

/// Why a job did not post, and the operation it stopped at if it had started one.
#[derive(Debug)]
pub struct Failure {
    pub operation: Option<String>,
    pub message: String,
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self { operation: None, message }
    }
}

#[derive(Serialize)]
pub struct OperationRange {
    pub id: String,
    pub start: u64,
    pub end: u64,
}

#[derive(Serialize)]
pub struct Program {
    pub nc: String,
    pub operations: Vec<OperationRange>,
    pub motions: u64,
    /// Motions camlib's stock-contact check could not clear.
    pub uncertain: u64,
    /// The stock after the last move against the part, where the job asks for it.
    pub material: Option<camlib::material::Report>,
}

/// Why writing an operation stopped.
enum Stop {
    /// camlib or the machine will not do this operation: the rest of the part can still be cut.
    Refused(String),
    /// What was written does not read back as the plan. That is a fault in this post, and nothing
    /// it wrote can be trusted, so no program is given out.
    Written(String),
}

impl From<String> for Stop {
    fn from(message: String) -> Self {
        Self::Refused(message)
    }
}

impl From<&str> for Stop {
    fn from(message: &str) -> Self {
        Self::Refused(message.into())
    }
}

/// A number as this post writes it: digits, a point and a sign, not `1e3` or `inf`.
fn plain_number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    digits.chars().any(|c| c.is_ascii_digit()) && digits.chars().all(|c| c.is_ascii_digit() || c == '.') && digits.parse::<f64>().is_ok()
}

/// A motion block has one motion word and coordinates, centres and a feed: nothing else. camlib's
/// decoder returns no move for any block it does not model, such as `G28`, or `M5`.
fn check_motion_words(text: &str) -> Result<(), String> {
    let mut words = text.split_whitespace();
    let code_ok = words.next().is_some_and(|code| ["G0", "G1", "G2", "G3"].contains(&code));
    let rest_ok = words.all(|word| word.starts_with(['X', 'Y', 'Z', 'I', 'J', 'K', 'F']) && plain_number(&word[1..]));
    if code_ok && rest_ok { Ok(()) } else { Err(format!("unreadable motion block {text:?}")) }
}

/// What a tool change's blocks mean on this machine: tool id and length register by number, and
/// where the tool tip is afterwards. From the job, not from the event being checked.
type ToolTable = HashMap<u32, (String, u32, Vec3)>;

/// Reads non-motion blocks back from their text, independently of what asked for them.
fn decode_control(block: &[String], tools: &ToolTable) -> Result<Vec<Event>, String> {
    let mut out = vec![];
    let mut selected: Option<u32> = None;
    let mut changed = false;
    for line in block {
        let words: Vec<&str> = line.split_whitespace().collect();
        let unreadable = || format!("unreadable block {line:?}");
        let value = |word: &str, letter: char| -> Result<f64, String> { word.strip_prefix(letter).filter(|v| plain_number(v)).and_then(|v| v.parse().ok()).ok_or_else(unreadable) };
        // Tool and register numbers are whole: `T3.9` is not T3.
        let whole = |word: &str, letter: char| -> Result<u32, String> { word.strip_prefix(letter).filter(|v| v.chars().all(|c| c.is_ascii_digit())).and_then(|v| v.parse().ok()).ok_or_else(unreadable) };
        match words.as_slice() {
            [t] if t.starts_with('T') => selected = Some(whole(t, 'T')?),
            ["M6"] => changed = selected.is_some(),
            ["G43", h] if h.starts_with('H') && changed => {
                let number = selected.take().ok_or("a length offset with no tool selected")?;
                changed = false;
                let register = whole(h, 'H')?;
                let (tool, expected, position_after_mm) = tools.get(&number).ok_or_else(|| format!("tool T{number} is not in the job"))?;
                if register != *expected {
                    return Err(format!("T{number} uses length register H{register}, the job says H{expected}"));
                }
                out.push(Event::ToolChange { tool: tool.clone(), number, length_register: register, position_after_mm: *position_after_mm });
            }
            ["M3", s] if s.starts_with('S') => out.push(Event::Spindle { rpm: value(s, 'S')?, direction: SpindleDirection::Cw }),
            ["M4", s] if s.starts_with('S') => out.push(Event::Spindle { rpm: value(s, 'S')?, direction: SpindleDirection::Ccw }),
            ["M5"] => out.push(Event::Spindle { rpm: 0., direction: SpindleDirection::Cw }),
            // Changing coolant is written as off, then on: one change.
            ["M9"] => out.push(Event::Coolant { mode: Coolant::Off }),
            [m] if *m == "M8" || *m == "M7" => {
                if matches!(out.last(), Some(Event::Coolant { mode: Coolant::Off })) {
                    out.pop();
                }
                out.push(Event::Coolant { mode: if *m == "M8" { Coolant::Flood } else { Coolant::Mist } });
            }
            // Only these: `G59.1` is a coordinate system of its own on LinuxCNC.
            [g] if ["G54", "G55", "G56", "G57", "G58", "G59"].contains(g) => out.push(Event::WorkOffset { number: g[1..].parse::<u8>().map_err(|e| e.to_string())? - 53 }),
            _ => return Err(format!("unreadable block {line:?}")),
        }
    }
    if selected.is_some() || changed {
        return Err("a tool change left unfinished".into());
    }
    Ok(out)
}

fn close(a: Vec3, b: Vec3, tolerance: f64) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() <= tolerance)
}

struct Post<'a> {
    machine: Machine,
    decoder: Decoder,
    lines: Vec<String>,
    position: Vec3,
    plane: Plane,
    spindle_on: bool,
    coolant: Coolant,
    checker: IntentChecker,
    tools: ToolTable,
    cancel: AtomicBool,
    tamper: Option<&'a dyn Fn(&str) -> String>,
}

impl<'a> Post<'a> {
    fn new(machine: Machine, manifest: &StreamManifest, tools: ToolTable, tamper: Option<&'a dyn Fn(&str) -> String>) -> Result<Self, String> {
        let position = manifest.initial_position_mm;
        if machine.output_units != "mm" {
            return Err("only mm output is wired up".into());
        }
        let commands = [
            ("rapid", "G0"), ("linear", "G1"), ("arc_cw", "G2"), ("arc_ccw", "G3"),
            ("plane_xy", "G17"), ("plane_xz", "G18"), ("plane_yz", "G19"),
        ]
        .into_iter()
        .map(|(effect, code)| Rule { effect: effect.into(), code: code.into() })
        .collect();
        let decoder = Decoder::new(Config {
            commands,
            axes: ["X".into(), "Y".into(), "Z".into()],
            centers: ["I".into(), "J".into(), "K".into()],
            feed_word: "F".into(),
            parentheses_comments: true,
            radius_tolerance_mm: RADIUS_TOLERANCE.absolute_mm,
            radius_relative_tolerance: RADIUS_TOLERANCE.relative,
        })?;
        let lines: Vec<String> = PREAMBLE.iter().map(|line| tamper.map_or((*line).to_string(), |tamper| tamper(line))).collect();
        if let Some(line) = lines.iter().zip(PREAMBLE).find(|(line, expected)| line != expected) {
            return Err(format!("the preamble says {:?} where it must say {:?}", line.0, line.1));
        }
        let header = Header {
            version: 1,
            id: manifest.run.id.clone(),
            revision: manifest.run.revision.clone(),
            units: "mm".into(),
            tolerance_mm: GEOMETRY_TOLERANCE,
            frame: WorkFrame::identity(),
            initial_position_mm: position,
            tools: manifest.tools.clone(),
            events: vec![],
        };
        let settings = Settings {
            capabilities: machine.capabilities()?,
            tolerance_mm: machine.tolerance_mm,
            feed_tolerance_mm_min: machine.feed_tolerance_mm_min,
            traverse_feed_mm_min: None,
            max_normal_accel_mm_s2: None,
            max_axis_feed_mm_min: None,
            work_to_machine: WorkFrame::identity(),
            tip_travel_mm: None,
            definition: serde_json::json!({"post": "step2cam linuxcnc", "decimals": machine.decimals}),
            run_id: manifest.run.id.clone(),
            decoded_initial_position_mm: position,
            lowering: Some(Lowering { native_arcs: true, ..Lowering::default() }),
            motion_scheduling: None,
        };
        let (checker, _) = IntentChecker::begin(&header, settings, &lines)?;
        Ok(Self { machine, decoder, lines, position, plane: Plane::Xy, spindle_on: false, coolant: Coolant::Off, checker, tools, cancel: AtomicBool::new(false), tamper })
    }

    /// A line as written, after any tampering.
    fn written_line(&self, line: &str) -> String {
        self.tamper.map_or_else(|| line.to_string(), |tamper| tamper(line))
    }

    /// Lines written since `mark`, as they stand after any tampering.
    fn block(&mut self, mark: usize) -> Vec<String> {
        if let Some(tamper) = self.tamper {
            for line in &mut self.lines[mark..] {
                *line = tamper(line);
            }
        }
        self.lines[mark..].to_vec()
    }

    /// Queues a source event and the piece it is written in: (fraction of it, the event for it).
    fn pieces(&mut self, event: &Event, whole: bool) -> Result<Vec<([f64; 2], Event)>, String> {
        self.checker.start_with(event.clone(), whole, &self.cancel)?;
        Ok(self.checker.preview(LOWERED_BATCH, &self.cancel)?.into_iter().map(|p| (p.source_fraction_interval, p.event)).collect())
    }

    fn check(&mut self, ranges: &[[f64; 2]], decoded: &[Vec<Event>], blocks: &[Vec<String>]) -> Result<(), String> {
        self.checker.check_lowered(ranges, decoded, blocks, &self.cancel).map(|_| ())
    }

    /// What a motion block says, as the checker wants it: the planned piece's identity, with the
    /// curve and feed read from the text.
    fn said(piece: &Event, written: Option<Decoded>) -> Result<Vec<Event>, String> {
        let Event::Motion { motion } = piece else { return Err("not a motion".into()) };
        Ok(written.map(|w| Event::Motion { motion: Motion { curve: w.curve, feed_mm_min: w.feed, ..motion.clone() } }).into_iter().collect())
    }

    fn words(&self, addresses: &str, values: &[f64]) -> Result<Vec<String>, String> {
        format_words(addresses, values, 1., self.machine.decimals)
    }

    /// Whether the machine can do this at all. Not being able to is not a fault in the post.
    fn allowed(&self, event: &Event) -> Result<(), String> {
        match event {
            Event::Motion { motion } if motion.feed_mm_min.is_some_and(|feed| feed > self.machine.max_feed_mm_min) => {
                Err(format!("feed {} is above the machine's limit", motion.feed_mm_min.unwrap_or(0.)))
            }
            Event::ToolChange { number, .. } if self.machine.max_tool_number.is_some_and(|m| *number > m) => Err(format!("tool number {number} is above the machine's limit")),
            Event::ToolChange { length_register, .. } if self.machine.max_length_register.is_some_and(|m| *length_register > m) => {
                Err(format!("length register {length_register} is above the machine's limit"))
            }
            Event::Spindle { rpm, .. } if *rpm > self.machine.max_spindle_rpm => Err(format!("spindle speed {rpm} is above the machine's limit")),
            _ => Ok(()),
        }
    }

    /// Writes an event and reads it back.
    fn event(&mut self, event: &Event) -> Result<(), Stop> {
        self.allowed(event).map_err(Stop::Refused)?;
        self.write_event(event).map_err(Stop::Written)
    }

    fn write_event(&mut self, event: &Event) -> Result<(), String> {
        if let Event::Motion { motion } = event {
            return self.motion(motion);
        }
        let mark = self.lines.len();
        match event {
            Event::Motion { .. } => unreachable!(),
            Event::ToolChange { number, length_register, position_after_mm, .. } => {
                self.lines.push(format!("T{number}"));
                self.lines.push("M6".into());
                self.lines.push(format!("G43 H{length_register}"));
                self.position = *position_after_mm;
            }
            Event::Spindle { rpm, direction } => {
                if *rpm == 0. {
                    self.lines.push("M5".into());
                } else {
                    let code = if *direction == SpindleDirection::Cw { "M3" } else { "M4" };
                    self.lines.push(format!("{code} {}", self.words("S", &[*rpm])?.join(" ")));
                }
                self.spindle_on = *rpm != 0.;
            }
            Event::Coolant { mode } => {
                if *mode == Coolant::Off || (self.coolant != Coolant::Off && self.coolant != *mode) {
                    self.lines.push("M9".into());
                }
                match mode {
                    Coolant::Flood => self.lines.push("M8".into()),
                    Coolant::Mist => self.lines.push("M7".into()),
                    Coolant::Off => {}
                }
                self.coolant = *mode;
            }
            Event::WorkOffset { number } => {
                if !(1..=6).contains(number) {
                    return Err(format!("work offset {number} is not G54 to G59"));
                }
                self.lines.push(format!("G{}", 53 + u32::from(*number)));
            }
            // The moves stay authoritative; this post writes them, not a canned cycle.
            Event::Cycle { .. } => {}
            other => return Err(format!("event not supported by this post: {other:?}")),
        }
        let block = self.block(mark);
        let decoded = decode_control(&block, &self.tools)?;
        let pieces = self.pieces(event, false)?;
        let ranges: Vec<[f64; 2]> = pieces.iter().map(|p| p.0).collect();
        self.check(&ranges, &[decoded], &[block])
    }

    fn motion(&mut self, motion: &Motion) -> Result<(), String> {
        match &motion.curve {
            Curve::Line { start, end } => self.line(motion, *start, *end),
            Curve::Arc { plane, sweep_rad, .. } | Curve::Helix { plane, sweep_rad, .. } => {
                let helix = matches!(motion.curve, Curve::Helix { .. });
                let name = match plane {
                    Plane::Xy => "xy",
                    Plane::Xz => "xz",
                    Plane::Yz => "yz",
                };
                let accepted = self.machine.arcs
                    && (!helix || self.machine.helices)
                    && (self.machine.planes.is_empty() || self.machine.planes.iter().any(|p| p == name))
                    && self.machine.max_sweep_rad.is_none_or(|m| sweep_rad.abs() <= m);
                let words = circular_words(&motion.curve, self.position, 1., self.machine.decimals, GEOMETRY_TOLERANCE, self.machine.tolerance_mm, RADIUS_TOLERANCE);
                match (accepted, words) {
                    (true, Ok((end, offsets))) => self.arc(motion, *plane, *sweep_rad, helix, end, offsets),
                    _ => self.lowered(motion),
                }
            }
        }
    }

    /// Writes one motion block, reads it back and returns the words' decoded curve.
    fn write(&mut self, code: &str, addresses: &str, values: &[f64], plane: Option<Plane>) -> Result<Option<Decoded>, String> {
        let text = format!("{code} {}", self.words(addresses, values)?.join(" "));
        let text = self.written_line(&text);
        check_motion_words(&text)?;
        let decoded = self.decoder.decode(&text, self.position, Some(1.), plane)?;
        self.lines.push(text);
        let Some(decoded) = decoded else { return Ok(None) };
        // A line that rounds to where the tool already is moves nothing.
        if matches!(decoded.curve, Curve::Line { start, end } if start == end) {
            return Ok(None);
        }
        self.position = decoded.curve.end();
        Ok(Some(decoded))
    }

    fn line(&mut self, motion: &Motion, start: Vec3, end: Vec3) -> Result<(), String> {
        let code = if matches!(motion.kind, MoveKind::Rapid) && motion.feed_mm_min.is_none() { "G0" } else { "G1" };
        // A straight vertical move keeps the controller's current XY (an arc's representable end
        // may differ by a word); writing the source XY would add a sideways step.
        let vertical = start[0] == end[0] && start[1] == end[1];
        let keep = vertical && (0..2).all(|k| (self.position[k] - end[k]).abs() <= self.machine.tolerance_mm / 2.);
        let target = if keep { [self.position[0], self.position[1], end[2]] } else { end };
        let source = Event::Motion { motion: motion.clone() };
        let pieces = self.pieces(&source, false)?;
        let [(range, piece)] = pieces.as_slice() else { return Err("a line planned as more than one piece".into()) };
        let mark = self.lines.len();
        let curve = self.write_line(code, target, motion.feed_mm_min, end)?;
        let block = self.lines[mark..].to_vec();
        let said = Self::said(piece, curve)?;
        self.check(&[*range], &[said], &[block])
    }

    fn write_line(&mut self, code: &str, target: Vec3, feed: Option<f64>, planned: Vec3) -> Result<Option<Decoded>, String> {
        let mut values = target.to_vec();
        let mut addresses = String::from("XYZ");
        if let Some(feed) = feed {
            addresses.push('F');
            values.push(feed);
        }
        let before = self.position;
        let plane = Some(self.plane);
        let written = self.write(code, &addresses, &values, plane)?;
        // A block that moves nothing at this precision decodes to no move, and is still planned
        // from where the tool is.
        let reached = written.as_ref().map_or(before, |w| w.curve.end());
        if !close(reached, planned, self.machine.tolerance_mm) {
            return Err(format!("posted block ends at {reached:?}, planned {planned:?}"));
        }
        Ok(written)
    }

    fn arc(&mut self, motion: &Motion, plane: Plane, sweep: f64, helix: bool, end: Vec3, offsets: Vec3) -> Result<(), String> {
        let (name, (u, v, axis)) = match plane {
            Plane::Xy => ("G17", (0, 1, 2)),
            Plane::Xz => ("G18", (2, 0, 1)),
            Plane::Yz => ("G19", (1, 2, 0)),
        };
        let source = Event::Motion { motion: motion.clone() };
        let pieces = self.pieces(&source, true)?;
        let [(range, piece)] = pieces.as_slice() else { return Err("a whole arc planned as more than one piece".into()) };
        let (range, piece) = (*range, piece.clone());
        let mark = self.lines.len();
        // The plane is modal, so it is written when it changes, and the arc is decoded in the plane
        // that written line selects.
        if self.plane != plane {
            let plane_line = self.written_line(name);
            self.plane = match plane_line.as_str() {
                "G17" => Plane::Xy,
                "G18" => Plane::Xz,
                "G19" => Plane::Yz,
                other => return Err(format!("unreadable plane block {other:?}")),
            };
            self.lines.push(plane_line);
        }
        let code = if sweep > 0. { "G3" } else { "G2" };
        let (mut addresses, mut values) = (String::new(), vec![]);
        for (i, letter) in ['X', 'Y', 'Z'].into_iter().enumerate() {
            if helix || i != axis {
                addresses.push(letter);
                values.push(end[i]);
            }
        }
        for k in [u, v] {
            addresses.push(['I', 'J', 'K'][k]);
            values.push(offsets[k]);
        }
        if let Some(feed) = motion.feed_mm_min {
            addresses.push('F');
            values.push(feed);
        }
        let written = self.write(code, &addresses, &values, Some(self.plane))?.ok_or("an arc block decoded to no move")?;
        let curve = &written.curve;
        // Rounding moves a small arc's centre and sweep further than its path, so compare paths.
        let tolerance = self.machine.tolerance_mm;
        let (planned, posted) = (motion.curve.prepare(GEOMETRY_TOLERANCE)?, curve.prepare(tolerance)?);
        for i in 0..=ARC_SAMPLES {
            let fraction = f64::from(i) / f64::from(ARC_SAMPLES);
            if !close(posted.position(fraction)?, planned.position(fraction)?, tolerance) {
                return Err(format!("posted arc strays from the plan on the way to {:?}", motion.curve.end()));
            }
        }
        let block = self.lines[mark..].to_vec();
        let said = Self::said(&piece, Some(written))?;
        self.check(&[range], &[said], &[block])
    }

    /// An arc as line segments within camlib's chord tolerance, in the pieces the checker plans.
    fn lowered(&mut self, motion: &Motion) -> Result<(), String> {
        let source = Event::Motion { motion: motion.clone() };
        let mut pieces = self.pieces(&source, false)?;
        while !pieces.is_empty() {
            let (mut ranges, mut decoded, mut blocks) = (vec![], vec![], vec![]);
            for (range, piece) in &pieces {
                let Event::Motion { motion: segment } = piece else { return Err("not a motion".into()) };
                let planned = segment.curve.end();
                let mark = self.lines.len();
                let curve = self.write_line("G1", planned, motion.feed_mm_min, planned)?;
                ranges.push(*range);
                decoded.push(Self::said(piece, curve)?);
                blocks.push(self.lines[mark..].to_vec());
            }
            self.check(&ranges, &decoded, &blocks)?;
            pieces = if self.checker.has_pending() {
                self.checker.preview(LOWERED_BATCH, &self.cancel)?.into_iter().map(|p| (p.source_fraction_interval, p.event)).collect()
            } else {
                vec![]
            };
        }
        Ok(())
    }

    fn finish(mut self) -> Result<String, String> {
        let spindle = self.spindle_on.then_some("M5");
        let coolant = (self.coolant != Coolant::Off).then_some("M9");
        let expected: Vec<&str> = spindle.into_iter().chain(coolant).chain(["G49", "M2"]).collect();
        let last: Vec<String> = expected.iter().map(|line| self.written_line(line)).collect();
        if let Some(line) = last.iter().zip(&expected).find(|(line, expected)| line != *expected) {
            return Err(format!("the end of the program says {:?} where it must say {:?}", line.0, line.1));
        }
        self.lines.extend_from_slice(&last);
        self.checker.finish(&last, &self.cancel)?;
        Ok(self.lines.join("\n") + "\n")
    }
}

type Modal = (u8, (String, u32, u32), (u64, SpindleDirection), Coolant);

/// A stop of spindle and coolant: held back in case the next operation carries straight on.
fn is_stop(events: &[Event]) -> bool {
    events.len() == 2
        && events.iter().any(|e| matches!(e, Event::Spindle { rpm, .. } if *rpm == 0.))
        && events.iter().any(|e| matches!(e, Event::Coolant { mode: Coolant::Off }))
}

fn begin_modal(events: &[Event]) -> Option<(Modal, Vec3)> {
    if events.len() != 4 {
        return None;
    }
    let (mut offset, mut tool, mut spindle, mut coolant) = (None, None, None, None);
    for e in events {
        match e {
            Event::WorkOffset { number } => offset = Some(*number),
            Event::ToolChange { tool: t, number, length_register, position_after_mm } => {
                tool = Some(((t.clone(), *number, *length_register), *position_after_mm));
            }
            Event::Spindle { rpm, direction } if *rpm != 0. => spindle = Some((rpm.to_bits(), *direction)),
            Event::Coolant { mode } => coolant = Some(*mode),
            _ => return None,
        }
    }
    let (assignment, pose) = tool?;
    Some(((offset?, assignment, spindle?, coolant?), pose))
}

/// The machine's tool table, from the job: each operation's tool number and length register, and
/// the tip's position after a change from the job's declared tool-change position.
fn tool_table(manifest: &StreamManifest, raw: &serde_json::Value) -> Result<ToolTable, String> {
    let change = manifest.tool_change.as_ref().ok_or("the job declares no tool-change position")?;
    let xy = change.xy_mm.unwrap_or([manifest.initial_position_mm[0], manifest.initial_position_mm[1]]);
    let mut table = ToolTable::new();
    for operation in raw["operations"].as_array().ok_or("the job has no operations")? {
        let id = operation["tool"].as_str().ok_or("operation has no tool")?;
        let tool = manifest.tools.iter().find(|t| t.id == id).ok_or("operation tool missing")?;
        let number = operation["tool_number"].as_u64().ok_or("operation has no tool number")? as u32;
        let register = operation["length_register"].as_u64().ok_or("operation has no length register")? as u32;
        table.insert(number, (tool.id.clone(), register, [xy[0], xy[1], change.gauge_z_mm - tool.gauge_length_mm]));
    }
    Ok(table)
}

fn gouge_message(report: &camlib::material::Report) -> String {
    let at = report.findings.iter().find(|f| f.gouge == Assessment::DefiniteExcess).map_or(String::new(), |f| {
        format!(", first near X{} Y{}, {} mm deep", f.xy_mm[0], f.xy_mm[1], f.gouge_height_mm[0])
    });
    format!("the finished stock cuts into the part: camlib's material check found {} cells below it{at}", report.gouge.definite_cells)
}

/// The most material a flat's cut can definitely meet: cutter centre and rim along the move. camlib
/// proves a flat clear of the part, not how much stock is in the way. A cell's lower bound says
/// what cannot have been cut, so what roughing only partly reached does not count.
fn deepest_cut(stock: &camlib::stock::Stock, motion: &Motion, radius: f64) -> f64 {
    let (start, end) = match &motion.curve {
        Curve::Line { start, end } => (*start, *end),
        curve => (curve.end(), curve.end()),
    };
    let ([nx, ny], [wx, wy]) = (stock.dimensions(), stock.cell_widths_mm());
    let (min, cells) = (stock.spec().min_mm, stock.cells());
    let top_at = |x: f64, y: f64| -> f64 {
        let (i, j) = (((x - min[0]) / wx).floor(), ((y - min[1]) / wy).floor());
        if i < 0. || j < 0. || i as usize >= nx || j as usize >= ny { f64::NEG_INFINITY } else { cells[j as usize * nx + i as usize].lower_mm }
    };
    let length = (end[0] - start[0]).hypot(end[1] - start[1]);
    let steps = (length / wx.min(wy)).ceil().max(1.) as usize;
    let mut deepest = f64::NEG_INFINITY;
    for step in 0..=steps {
        let t = step as f64 / steps as f64;
        let (x, y) = (start[0] + (end[0] - start[0]) * t, start[1] + (end[1] - start[1]) * t);
        deepest = deepest.max(top_at(x, y));
        for k in 0..8 {
            let angle = f64::from(k) * std::f64::consts::FRAC_PI_4;
            deepest = deepest.max(top_at(x + 0.9 * radius * angle.cos(), y + 0.9 * radius * angle.sin()));
        }
    }
    deepest - end[2]
}

/// The stock against the part, if the job asks for it at this point. A definite gouge refuses the
/// whole program, and so does a comparison that ran out of budget: it has not looked everywhere.
fn compare(producer: &mut StreamProducer, binding: &Binding, after: Option<&str>, cancel: &AtomicBool) -> Result<Option<camlib::material::Report>, Failure> {
    if producer.start_material_verification(after)?.is_none() {
        return Ok(None);
    }
    let bytes = producer.finish_material_verification(cancel)?;
    let Check::Material { report, .. } = binding.decode(&bytes, producer.completed(), None)?.check else {
        return Err(Failure::from("camlib's material check gave no report".to_string()));
    };
    if report.gouge.definite_cells > 0 {
        return Err(Failure::from(gouge_message(&report)));
    }
    if !report.complete {
        return Err(Failure::from(format!("camlib's comparison of the stock with the part ran out of budget with {} cells unexamined, so no program is given out", report.stats.unexamined_cells)));
    }
    Ok(Some(*report))
}

/// A program that does not read back as its plan: not tied to an operation, so nothing is retried.
fn written(message: String) -> Failure {
    Failure::from(format!("the written program does not match the plan, so none is given out: {message}"))
}

/// Plans, simulates, checks and posts one job. A definite stock-contact violation refuses the
/// operation, and a block that does not read back as planned the whole program; motions the check
/// could not clear are counted.
pub fn post(manifest_json: &str, machine: Machine) -> Result<Program, Failure> {
    post_tampered(manifest_json, machine, None)
}

/// [`post`], with every written line passed through `tamper` before it is read back: for testing
/// that the checks see what was written.
#[doc(hidden)]
pub fn post_tampered(manifest_json: &str, machine: Machine, tamper: Option<&dyn Fn(&str) -> String>) -> Result<Program, Failure> {
    let manifest: StreamManifest = serde_json::from_str(manifest_json).map_err(|e| e.to_string())?;
    let binding = Binding::new(&manifest)?;
    let mut producer = StreamProducer::new(manifest.clone())?;
    let cancel = AtomicBool::new(false);
    let raw: serde_json::Value = serde_json::from_str(manifest_json).map_err(|e| e.to_string())?;
    let tools = tool_table(&manifest, &raw)?;
    let mut post = Post::new(machine, &manifest, tools, tamper).map_err(written)?;
    let mut operations = vec![];
    let mut uncertain = 0;
    let mut held: Option<Vec<Event>> = None;
    let mut modal: Option<Modal> = None;
    let mut active_tool: Option<(String, u32, u32)> = None;
    let mut position = manifest.initial_position_mm;
    let mut active: Option<&str> = None;
    let mut material = None;

    for (index, operation) in manifest.operations.iter().enumerate() {
        let mut one = || -> Result<(), Stop> {
            if active != Some(operation.tool()) {
                producer.change_tool(operation.tool())?;
                active = Some(operation.tool());
            }
            let tool = manifest.tools.iter().find(|t| t.id == operation.tool()).ok_or("operation tool missing")?;
            let mut plan = OperationPlan::prepare_with_stock(
                operation.clone(),
                tool,
                producer.stock()?,
                producer.position(),
                producer.completed().checked_add(1).ok_or("motion ID overflow")?,
                &cancel,
            )?;
            let start = producer.completed();

            // An operation continuing with the same tool, spindle, coolant and offset drops the
            // previous stop and its own restart; otherwise every stop and start stays. A tool
            // assignment that repeats the current one without moving the tool is dropped.
            let begin = plan.prefix_events();
            let parsed = begin_modal(&begin);
            let previous = held.take();
            let continues = match (&previous, &parsed) {
                (Some(_), Some((m, pose))) => modal.as_ref() == Some(m) && active_tool.as_ref() == Some(&m.1) && *pose == position,
                _ => false,
            };
            if !continues {
                modal = parsed.map(|(m, _)| m);
                for event in previous.iter().flatten().chain(begin.iter()) {
                    if let Event::ToolChange { tool, number, length_register, position_after_mm } = event {
                        let assignment = (tool.clone(), *number, *length_register);
                        let repeat = active_tool.as_ref() == Some(&assignment) && *position_after_mm == position;
                        active_tool = Some(assignment);
                        position = *position_after_mm;
                        if repeat {
                            continue;
                        }
                    }
                    post.event(event)?;
                }
            }
            for event in plan.take_events() {
                post.event(&event)?;
            }
            // What a flat may take: what roughing leaves on a flat between its layers.
            let flat_limit = raw["operations"][index].get("kind").filter(|k| *k == "flat").and_then(|_| {
                let rough = raw["operations"].as_array()?.iter().find(|o| o["kind"] == "roughing" && o["tool"] == raw["operations"][index]["tool"])?;
                let radius = raw["tools"].as_array()?.iter().find(|t| t["id"] == rough["tool"])?["cutter"]["diameter_mm"].as_f64()? / 2.;
                Some((rough["stepdown_mm"].as_f64()? + rough["bottom_allowance_mm"].as_f64()?, radius))
            });
            let guard = camlib::limits::LinkGuard::new(operation.limits(), tool, manifest.sweep.tolerance_mm)?;
            while let Some(motion) = plan.next_motion_with_stock(producer.stock()?, &cancel)? {
                let motion = manifest.retract_policy(motion, producer.stock()?, operation.declares_retract_feed());
                if let Some((limit, radius)) = flat_limit.filter(|_| motion.feed_mm_min.is_some()) {
                    let depth = deepest_cut(producer.stock()?, &motion, radius);
                    if depth > limit + DEPTH_TOLERANCE {
                        let at = motion.curve.end();
                        return Err(Stop::Refused(format!("a flat would cut {depth:.1} mm deep near X{:.1} Y{:.1}, where roughing did not reach; its most is {limit:.1} mm", at[0], at[1])));
                    }
                }
                if let Some(g) = &guard {
                    g.check(&motion)?;
                }
                let (_, check) = producer.push_checked_motion(motion.clone(), DEFAULT_FRAME_LIMIT, &cancel)?;
                if let Some(raw) = check {
                    match binding.decode(&raw, producer.completed(), Some(&motion))?.check.motion_status() {
                        Some(Status::Violation) => {
                            return Err(Stop::Refused(format!("camlib's stock-contact check found a violation in {} at motion {}", operation.id(), motion.id)));
                        }
                        Some(Status::Uncertain) => uncertain += 1,
                        _ => {}
                    }
                }
                position = motion.curve.end();
                post.event(&Event::Motion { motion })?;
                for event in plan.take_events() {
                    post.event(&event)?;
                }
            }
            plan.finish_report(&cancel)?;
            operations.push(OperationRange { id: operation.id().to_string(), start, end: producer.completed() });
            let end = plan.suffix_events();
            if is_stop(&end) {
                held = Some(end);
            } else {
                for event in &end {
                    post.event(event)?;
                }
            }
            Ok(())
        };
        one().map_err(|stop| match stop {
            Stop::Refused(message) => Failure { operation: Some(operation.id().to_string()), message },
            Stop::Written(message) => written(message),
        })?;
        if material.is_none() {
            material = compare(&mut producer, &binding, Some(operation.id()), &cancel)?;
        }
    }
    for event in held.iter().flatten() {
        post.event(event).map_err(|stop| match stop {
            Stop::Refused(message) => Failure::from(message),
            Stop::Written(message) => written(message),
        })?;
    }
    if material.is_none() {
        material = compare(&mut producer, &binding, None, &cancel)?;
    }
    producer.finish(DEFAULT_FRAME_LIMIT)?;
    Ok(Program { motions: producer.completed(), nc: post.finish().map_err(written)?, operations, uncertain, material })
}
