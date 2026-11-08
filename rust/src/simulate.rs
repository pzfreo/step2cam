//! A camlib job -> camlib's moves, simulated on the stock and checked, in-process.
//!
//! camlib plans each operation's motions and simulates them against the stock, with its
//! stock-contact checks, and compares the finished stock with the part. Nothing here writes
//! G-code: that is camlib's, from the same job, through its own posts.
use camlib::{
    material::Assessment,
    motion::{Curve, Motion, MoveKind},
    operation::OperationPlan,
    stream::{DEFAULT_FRAME_LIMIT, StreamManifest, StreamProducer},
    stream_verification::{Binding, Check},
    verification::Status,
};
use serde::Serialize;
use std::sync::atomic::AtomicBool;

const DEPTH_TOLERANCE: f64 = 0.01;
/// Arcs and helices are drawn as straight pieces of at most this angle.
const ARC_STEP_RAD: f64 = std::f64::consts::PI / 18.;

/// Why a job did not simulate, and the operation it stopped at if it had started one.
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

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

#[derive(Serialize)]
pub struct OperationRange {
    pub id: String,
    pub start: u64,
    pub end: u64,
}

/// One of camlib's moves, for drawing: its path as points from its start to its end, an arc or
/// helix in straight pieces.
#[derive(Serialize)]
pub struct Move {
    pub op: String,
    pub tool: String,
    pub kind: MoveKind,
    /// mm/min; none for a rapid.
    pub feed: Option<f64>,
    /// "line", "arc" or "helix".
    pub curve: &'static str,
    pub points: Vec<[f64; 3]>,
}

#[derive(Serialize)]
pub struct Run {
    pub moves: Vec<Move>,
    pub operations: Vec<OperationRange>,
    /// Motions camlib's stock-contact check could not clear.
    pub uncertain: u64,
    /// The stock after the last move against the part, where the job asks for it.
    pub material: Option<camlib::material::Report>,
    /// Per-motion proof, separate from the independent stock-cell comparison.
    pub sweep_proof: Option<camlib::material::SweepProofReport>,
}

fn round(p: [f64; 3]) -> [f64; 3] {
    p.map(|v| (v * 1e4).round() / 1e4)
}

fn drawn(motion: &Motion, tolerance: f64) -> Result<Move, String> {
    let (curve, sweep) = match &motion.curve {
        Curve::Line { .. } => ("line", 0.),
        Curve::Arc { sweep_rad, .. } => ("arc", *sweep_rad),
        Curve::Helix { sweep_rad, .. } => ("helix", *sweep_rad),
    };
    let points = if curve == "line" {
        vec![round(motion.curve.start()), round(motion.curve.end())]
    } else {
        let prepared = motion.curve.prepare(tolerance)?;
        let steps = (sweep.abs() / ARC_STEP_RAD).ceil().max(4.) as u32;
        (0..=steps).map(|i| prepared.position(f64::from(i) / f64::from(steps)).map(round)).collect::<Result<_, _>>()?
    };
    Ok(Move { op: motion.operation.clone(), tool: motion.tool.clone(), kind: motion.kind, feed: motion.feed_mm_min, curve, points })
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
/// whole job, and so does a comparison that ran out of budget: it has not looked everywhere.
struct MaterialComparison {
    report: camlib::material::Report,
    sweep_proof: Option<camlib::material::SweepProofReport>,
}

fn compare(producer: &mut StreamProducer, binding: &Binding, after: Option<&str>, cancel: &AtomicBool) -> Result<Option<MaterialComparison>, Failure> {
    if producer.start_material_verification(after)?.is_none() {
        return Ok(None);
    }
    let bytes = producer.finish_material_verification(cancel)?;
    let Check::Material { report, sweep_proof, .. } = binding.decode(&bytes, producer.completed(), None)?.check else {
        return Err(Failure::from("camlib's material check gave no report"));
    };
    if report.gouge.definite_cells > 0 {
        return Err(Failure::from(gouge_message(&report)));
    }
    if !report.complete {
        return Err(Failure::from(format!("camlib's comparison of the stock with the part ran out of budget with {} cells unexamined, so the job is not accepted", report.stats.unexamined_cells)));
    }
    Ok(Some(MaterialComparison { report: *report, sweep_proof }))
}

/// Describe camlib's evidence; this does not decide whether any assembly part collides.
fn stock_contact_reason(report: &camlib::stock_contact::Report, tool: &camlib::tool::ToolAssembly) -> Option<String> {
    use camlib::verification::ToolPart;
    let contacts: Vec<String> = report.findings.iter().filter(|c| c.finding.status == Status::Violation).map(|c| {
        let f = &c.finding;
        let part = match f.part { Some(ToolPart::Cutter) => "flutes", Some(ToolPart::Neck) => "neck", Some(ToolPart::Shank) => "shank", Some(ToolPart::Holder) => "holder", None => "tool assembly" };
        let section = f.holder_section.map(|n| format!(" section {n}")).unwrap_or_default();
        let point = c.contact_point_mm.or_else(|| f.witness.as_ref().map(|w| w.tip_mm));
        let at = point.map(|p| format!(" at X{:.1} Y{:.1} Z{:.1}", p[0], p[1], p[2])).unwrap_or_default();
        let mut text = format!("{}'s {part}{section} hits the stock{at} in {}", tool.id, report.operation);
        if matches!(f.part, Some(ToolPart::Holder | ToolPart::Shank))
            && let Some(w) = &f.witness {
                let reach = (report.stock.max_mm[2] - w.tip_mm[2] + 1.).ceil();
                text.push_str(&format!(": needs an end mill of ⌀{} with stickout of at least {reach:.0} mm", tool.cutter.diameter_mm));
                if f.part == Some(ToolPart::Holder) {
                    let holder = f.holder_section.and_then(|n| tool.holders.get(n));
                    if let Some(h) = holder {
                        text.push_str(&format!(", or a narrower holder than the current ⌀{} mm", 2. * h.radius_min_mm.max(h.radius_max_mm)));
                    }
                }
        }
        text
    }).collect();
    (!contacts.is_empty()).then(|| format!("camlib refused {}: {}", report.operation, contacts.join("; ")))
}

/// Plans, simulates and checks one job. A definite stock-contact violation refuses the operation;
/// motions the check could not clear are counted.
pub fn simulate(manifest_json: &str) -> Result<Run, Failure> {
    let manifest: StreamManifest = serde_json::from_str(manifest_json).map_err(|e| e.to_string())?;
    let binding = Binding::new(&manifest)?;
    let mut producer = StreamProducer::new(manifest.clone())?;
    let cancel = AtomicBool::new(false);
    let raw: serde_json::Value = serde_json::from_str(manifest_json).map_err(|e| e.to_string())?;
    let (mut moves, mut operations, mut uncertain) = (vec![], vec![], 0);
    let mut active: Option<&str> = None;
    let mut material = None;

    for (index, operation) in manifest.operations.iter().enumerate() {
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
                        return Err(format!("a flat would cut {depth:.1} mm deep near X{:.1} Y{:.1}, where roughing did not reach; its most is {limit:.1} mm", at[0], at[1]));
                    }
                }
                if let Some(g) = &guard {
                    g.check(&motion)?;
                }
                let (_, check) = producer.push_checked_motion(motion.clone(), DEFAULT_FRAME_LIMIT, &cancel)?;
                if let Some(raw) = check {
                    let checked = binding.decode(&raw, producer.completed(), Some(&motion))?;
                    match checked.check.motion_status() {
                        Some(Status::Violation) => {
                            if let Check::Motion { stock_contact: Some(report), .. } = &checked.check
                                && let Some(reason) = stock_contact_reason(report, tool) {
                                return Err(reason);
                            }
                            if let Check::Motion { report: Some(report), .. } = &checked.check {
                                let contacts: Vec<String> = report.findings.iter().filter(|f| f.status == Status::Violation)
                                    .map(|f| format!("{} ({:?}, {:?})", f.object_id, f.kind, f.part)).collect();
                                if !contacts.is_empty() {
                                    return Err(format!("camlib's fixture/scene check found a violation in {} at motion {}: {}", operation.id(), motion.id, contacts.join(", ")));
                                }
                            }
                            return Err(format!("camlib's stock-contact check found a violation in {} at motion {}", operation.id(), motion.id));
                        }
                        Some(Status::Uncertain) => uncertain += 1,
                        _ => {}
                    }
                }
                moves.push(drawn(&motion, manifest.sweep.tolerance_mm)?);
                // Events between motions (canned cycles) are for a post: the motions say it all.
                plan.take_events();
            }
            plan.finish_report(&cancel)?;
            operations.push(OperationRange { id: operation.id().to_string(), start, end: producer.completed() });
            Ok(())
        };
        one().map_err(|message| Failure { operation: Some(operation.id().to_string()), message })?;
        if material.is_none() {
            material = compare(&mut producer, &binding, Some(operation.id()), &cancel)?;
        }
    }
    if material.is_none() {
        material = compare(&mut producer, &binding, None, &cancel)?;
    }
    producer.finish(DEFAULT_FRAME_LIMIT)?;
    let (material, sweep_proof) = material.map_or((None, None), |comparison| (Some(comparison.report), comparison.sweep_proof));
    Ok(Run { moves, operations, uncertain, material, sweep_proof })
}

#[cfg(test)]
mod contact_wording_tests {
    use super::*;
    use camlib::{stock_contact::{ContactFinding, Report}, verification::{Finding, FindingKind, ToolPart, Witness}};
    use serde_json::json;

    #[test]
    fn relays_assembly_part_section_and_camlib_position() {
        let tool = serde_json::from_value(json!({"id":"em6", "cutter":{"id":"em6", "shape":{"kind":"flat"}, "diameter_mm":6., "cutting_length_mm":80., "overall_length_mm":100., "shank_diameter_mm":6., "flutes":2, "center_cutting":true}, "stickout_mm":90., "gauge_length_mm":120., "holders":[{"z_min_mm":90., "z_max_mm":120., "radius_min_mm":20., "radius_max_mm":20.}]})).unwrap();
        let mut report = Report {
            algorithm: "continuous_stock_contact_v1".into(),
            stock: camlib::stock::StockSpec { version:1, units:"mm".into(), min_mm:[0.,0.,-70.], max_mm:[60.,40.,0.], cell_size_mm:1., max_cells:10000 },
            pre_generation:0, options_sha256:String::new(), tool_sha256:String::new(), motion_sha256:String::new(), motion_id:1,
            operation:"top-3-notch".into(), tool:"em6".into(), scope:String::new(), status:Status::Violation, fully_examined:true,
            findings:vec![ContactFinding {
                finding:Finding {status:Status::Violation, kind:FindingKind::StockContact, object_id:"stock".into(), part:Some(ToolPart::Holder), holder_section:Some(0), fraction_interval:[0.,1.],
                    witness:Some(Witness {fraction:0.5, tip_mm:[1.,2.,-64.2], section_height_mm:Some(90.), margin_lower_mm:0.1}), detail:"camlib evidence".into()},
                cell_range:[0,0,1,1], solid_lower_mm:0., possible_upper_mm:0., contact_point_mm:Some([3.,4.,-1.])
            }], stats:Default::default(), numerical_policy:String::new()
        };
        let text = stock_contact_reason(&report, &tool).unwrap();
        assert!(text.contains("em6's holder section 0 hits the stock at X3.0 Y4.0 Z-1.0 in top-3-notch"), "{text}");
        assert!(text.contains(": needs an end mill of ⌀6 with stickout of at least 66 mm, or a narrower holder than the current ⌀40 mm"), "{text}");
        for (part, name) in [(ToolPart::Shank,"shank"), (ToolPart::Neck,"neck"), (ToolPart::Cutter,"flutes")] {
            report.findings[0].finding.part = Some(part);
            report.findings[0].finding.holder_section = None;
            report.findings[0].contact_point_mm = None;
            let text = stock_contact_reason(&report, &tool).unwrap();
            assert!(text.contains(&format!("em6's {name} hits the stock at X1.0 Y2.0 Z-64.2")), "{text}");
            assert_eq!(text.contains(": needs "), part == ToolPart::Shank);
        }
        report.findings[0].finding.status = Status::Uncertain;
        assert!(stock_contact_reason(&report, &tool).is_none());
    }
}
