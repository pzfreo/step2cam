//! A camlib job -> checked LinuxCNC G-code, in-process.
//!
//! camlib plans and simulates each motion against the stock, with its stock-contact checks. Each
//! motion block written is read back with camlib's own decoder and compared with the planned
//! curve. An arc that cannot be written exactly, or that the machine does not accept, becomes
//! line segments within the chord tolerance. camlib's whole-program intent checker is not run
//! here: its decoding of non-motion blocks is in camlib's Python.
use camlib::{
    motion::{Coolant, Curve, Event, Motion, MoveKind, Plane, SpindleDirection, Vec3},
    operation::OperationPlan,
    post_decode::{Config, Decoder, Rule, format_words},
    post_rounding::{RadiusTolerance, circular_words},
    stream::{DEFAULT_FRAME_LIMIT, StreamManifest, StreamProducer},
    stream_verification::Binding,
    verification::Status,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

const GEOMETRY_TOLERANCE: f64 = 1e-5;
const CHORD_TOLERANCE: f64 = 0.0025;
const ARC_SAMPLES: u32 = 16;
const RADIUS_TOLERANCE: RadiusTolerance = RadiusTolerance { absolute_mm: 0.00127, relative: 0.001 };

fn default_tolerance() -> f64 {
    0.01
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
}

fn close(a: Vec3, b: Vec3, tolerance: f64) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() <= tolerance)
}

struct Post {
    machine: Machine,
    decoder: Decoder,
    lines: Vec<String>,
    position: Vec3,
    plane: Plane,
    spindle_on: bool,
    coolant: Coolant,
}

impl Post {
    fn new(machine: Machine, position: Vec3) -> Result<Self, String> {
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
        let lines = ["G21", "G90", "G91.1", "G17", "G94", "G97", "G40", "G49", "G80", "G92.1", "G61.1", "M5", "M9"]
            .map(String::from)
            .to_vec();
        Ok(Self { machine, decoder, lines, position, plane: Plane::Xy, spindle_on: false, coolant: Coolant::Off })
    }

    fn words(&self, addresses: &str, values: &[f64]) -> Result<Vec<String>, String> {
        format_words(addresses, values, 1., self.machine.decimals)
    }

    fn event(&mut self, event: &Event) -> Result<(), String> {
        match event {
            Event::Motion { motion } => self.motion(motion)?,
            Event::ToolChange { number, length_register, position_after_mm, .. } => {
                if self.machine.max_tool_number.is_some_and(|m| *number > m) {
                    return Err(format!("tool number {number} is above the machine's limit"));
                }
                if self.machine.max_length_register.is_some_and(|m| *length_register > m) {
                    return Err(format!("length register {length_register} is above the machine's limit"));
                }
                self.lines.push(format!("T{number}"));
                self.lines.push("M6".into());
                self.lines.push(format!("G43 H{length_register}"));
                self.position = *position_after_mm;
            }
            Event::Spindle { rpm, direction } => {
                if *rpm > self.machine.max_spindle_rpm {
                    return Err(format!("spindle speed {rpm} is above the machine's limit"));
                }
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
        Ok(())
    }

    fn motion(&mut self, motion: &Motion) -> Result<(), String> {
        if motion.feed_mm_min.is_some_and(|feed| feed > self.machine.max_feed_mm_min) {
            return Err(format!("feed {} is above the machine's limit", motion.feed_mm_min.unwrap_or(0.)));
        }
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
    fn write(&mut self, code: &str, addresses: &str, values: &[f64], plane: Option<Plane>) -> Result<Option<Curve>, String> {
        let text = format!("{code} {}", self.words(addresses, values)?.join(" "));
        let decoded = self.decoder.decode(&text, self.position, Some(1.), plane)?;
        self.lines.push(text);
        let Some(decoded) = decoded else { return Ok(None) };
        self.position = decoded.curve.end();
        Ok(Some(decoded.curve))
    }

    fn line(&mut self, motion: &Motion, start: Vec3, end: Vec3) -> Result<(), String> {
        let code = if matches!(motion.kind, MoveKind::Rapid) && motion.feed_mm_min.is_none() { "G0" } else { "G1" };
        // A straight vertical move keeps the controller's current XY (an arc's representable end
        // may differ by a word); writing the source XY would add a sideways step.
        let vertical = start[0] == end[0] && start[1] == end[1];
        let keep = vertical && (0..2).all(|k| (self.position[k] - end[k]).abs() <= self.machine.tolerance_mm / 2.);
        let target = if keep { [self.position[0], self.position[1], end[2]] } else { end };
        self.write_line(code, target, motion.feed_mm_min, end)
    }

    fn write_line(&mut self, code: &str, target: Vec3, feed: Option<f64>, planned: Vec3) -> Result<(), String> {
        let mut values = target.to_vec();
        let mut addresses = String::from("XYZ");
        if let Some(feed) = feed {
            addresses.push('F');
            values.push(feed);
        }
        let before = self.position;
        let plane = Some(self.plane);
        let curve = self.write(code, &addresses, &values, plane)?;
        // A block that moves nothing at this precision decodes to no move, and is still planned
        // from where the tool is.
        let reached = curve.map_or(before, |c| c.end());
        if !close(reached, planned, self.machine.tolerance_mm) {
            return Err(format!("posted block ends at {reached:?}, planned {planned:?}"));
        }
        Ok(())
    }

    fn arc(&mut self, motion: &Motion, plane: Plane, sweep: f64, helix: bool, end: Vec3, offsets: Vec3) -> Result<(), String> {
        let (name, (u, v, axis)) = match plane {
            Plane::Xy => ("G17", (0, 1, 2)),
            Plane::Xz => ("G18", (2, 0, 1)),
            Plane::Yz => ("G19", (1, 2, 0)),
        };
        self.lines.push(name.into());
        self.plane = plane;
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
        let curve = self.write(code, &addresses, &values, Some(plane))?.ok_or("an arc block decoded to no move")?;
        // Rounding moves a small arc's centre and sweep further than its path, so compare paths.
        let tolerance = self.machine.tolerance_mm;
        let (planned, posted) = (motion.curve.prepare(GEOMETRY_TOLERANCE)?, curve.prepare(tolerance)?);
        for i in 0..=ARC_SAMPLES {
            let fraction = f64::from(i) / f64::from(ARC_SAMPLES);
            if !close(posted.position(fraction)?, planned.position(fraction)?, tolerance) {
                return Err(format!("posted arc strays from the plan on the way to {:?}", motion.curve.end()));
            }
        }
        Ok(())
    }

    /// An arc as line segments within the chord tolerance.
    fn lowered(&mut self, motion: &Motion) -> Result<(), String> {
        let prepared = motion.curve.prepare(GEOMETRY_TOLERANCE)?;
        let (center, sweep, plane) = match &motion.curve {
            Curve::Arc { center, sweep_rad, plane, .. } | Curve::Helix { center, sweep_rad, plane, .. } => (*center, *sweep_rad, *plane),
            Curve::Line { .. } => return Err("not an arc".into()),
        };
        let (u, v) = match plane {
            Plane::Xy => (0, 1),
            Plane::Xz => (2, 0),
            Plane::Yz => (1, 2),
        };
        let start = motion.curve.start();
        let radius = (start[u] - center[u]).hypot(start[v] - center[v]);
        let step = 2. * (1. - (CHORD_TOLERANCE / radius).min(1.)).acos();
        let count = (sweep.abs() / step.max(1e-3)).ceil().max(1.) as usize;
        for i in 1..=count {
            let planned = prepared.position(i as f64 / count as f64)?;
            self.write_line("G1", planned, motion.feed_mm_min, planned)?;
        }
        Ok(())
    }

    fn finish(mut self) -> String {
        if self.spindle_on {
            self.lines.push("M5".into());
        }
        if self.coolant != Coolant::Off {
            self.lines.push("M9".into());
        }
        self.lines.push("G49".into());
        self.lines.push("M2".into());
        self.lines.join("\n") + "\n"
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

/// Plans, simulates, checks and posts one job. A definite stock-contact violation refuses the
/// program; motions the check could not clear are counted.
pub fn post(manifest_json: &str, machine: Machine) -> Result<Program, Failure> {
    let manifest: StreamManifest = serde_json::from_str(manifest_json).map_err(|e| e.to_string())?;
    let binding = Binding::new(&manifest)?;
    let mut producer = StreamProducer::new(manifest.clone())?;
    let cancel = AtomicBool::new(false);
    let mut post = Post::new(machine, manifest.initial_position_mm)?;
    let mut operations = vec![];
    let mut uncertain = 0;
    let mut held: Option<Vec<Event>> = None;
    let mut modal: Option<Modal> = None;
    let mut active_tool: Option<(String, u32, u32)> = None;
    let mut position = manifest.initial_position_mm;
    let mut active: Option<&str> = None;

    for operation in &manifest.operations {
        let mut one = || -> Result<(), String> {
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
            let guard = camlib::limits::LinkGuard::new(operation.limits(), tool, manifest.sweep.tolerance_mm)?;
            while let Some(motion) = plan.next_motion_with_stock(producer.stock()?, &cancel)? {
                let motion = manifest.retract_policy(motion, producer.stock()?, operation.declares_retract_feed());
                if let Some(g) = &guard {
                    g.check(&motion)?;
                }
                let (_, check) = producer.push_checked_motion(motion.clone(), DEFAULT_FRAME_LIMIT, &cancel)?;
                if let Some(raw) = check {
                    match binding.decode(&raw, producer.completed(), Some(&motion))?.check.motion_status() {
                        Some(Status::Violation) => {
                            return Err(format!("camlib's stock-contact check found a violation in {} at motion {}", operation.id(), motion.id));
                        }
                        Some(Status::Uncertain) => uncertain += 1,
                        _ => {}
                    }
                }
                position = motion.curve.end();
                post.motion(&motion)?;
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
        one().map_err(|message| Failure { operation: Some(operation.id().to_string()), message })?;
    }
    for event in held.iter().flatten() {
        post.event(event).map_err(Failure::from)?;
    }
    producer.finish(DEFAULT_FRAME_LIMIT)?;
    Ok(Program { motions: producer.completed(), nc: post.finish(), operations, uncertain })
}
