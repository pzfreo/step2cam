//! A camlib job -> camlib's moves, simulated on the stock and checked, in-process.
//!
//! camlib plans each operation's motions and simulates them against the stock, with its
//! stock-contact checks, and compares the finished stock with the part. Nothing here writes
//! G-code: that is camlib's, from the same job, through its own posts.
use camlib::{
    buffers::HeightRead,
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

#[derive(Clone, Serialize)]
pub struct OperationRange {
    pub id: String,
    pub start: u64,
    pub end: u64,
}

/// One of camlib's moves, for drawing: its path as points from its start to its end, an arc or
/// helix in straight pieces.
#[derive(Clone, Serialize)]
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

#[derive(Clone, Serialize)]
pub struct Run {
    #[serde(skip)]
    pub finished_stock: Option<Result<camlib::stock_mesh::FinishedStock, String>>,
    pub moves: Vec<Move>,
    pub operations: Vec<OperationRange>,
    /// Motions camlib's stock-contact check could not clear.
    pub uncertain: u64,
    /// The stock after the last move against the part, where the job asks for it.
    #[serde(serialize_with = "serialize_material")]
    pub material: Option<std::sync::Arc<camlib::material::Report>>,
    /// Per-motion proof, separate from the independent stock-cell comparison.
    pub sweep_proof: Option<camlib::material::SweepProofReport>,
}

fn serialize_material<S: serde::Serializer>(value: &Option<std::sync::Arc<camlib::material::Report>>, serializer: S) -> Result<S::Ok, S::Error> {
    value.as_deref().serialize(serializer)
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

/// Restrict comparison to the real stock box. The deeper tap backing is only a
/// simulation aid, not material the target requires us to preserve. Clamping both
/// interval endpoints intersects each remaining column with the real stock box.
struct RealStock<'a> {
    stock: &'a camlib::stock::Stock,
    bottom: f64,
}
impl HeightRead for RealStock<'_> {
    fn scalar_len(&self) -> usize { self.stock.scalar_len() }
    fn read(&self, index: usize) -> f64 { self.stock.read(index).max(self.bottom) }
}

fn compare_real_stock(check: &camlib::stream_verification::MaterialCheck, stock: &camlib::stock::Stock, bottom: f64, cancel: &AtomicBool) -> Result<camlib::material::Report, String> {
    let mut spec = stock.spec().clone();
    spec.min_mm[2] = bottom;
    let context = camlib::material::Context { stock: spec, generation: stock.generation(), fraction: 0., run: None };
    check.target.prepare(check.options)?.check(&RealStock { stock, bottom }, &context, cancel)
}

fn compare(producer: &mut StreamProducer, binding: &Binding, after: Option<&str>, cancel: &AtomicBool, physical: Option<(&StreamManifest, f64)>) -> Result<Option<MaterialComparison>, Failure> {
    if producer.start_material_verification(after)?.is_none() {
        return Ok(None);
    }
    let bytes = producer.finish_material_verification(cancel)?;
    let Check::Material { report, sweep_proof, .. } = binding.decode(&bytes, producer.completed(), None)?.check else {
        return Err(Failure::from("camlib's material check gave no report"));
    };
    let (report, sweep_proof) = if let Some((manifest, bottom)) = physical {
        let check = manifest.verification.as_ref().and_then(|v| v.material.iter().find(|c| c.after_operation.as_deref() == after)).ok_or("missing real-stock comparison target")?;
        // Complete/decode the stream check above to retain its verification cursor.
        // This separate report assesses the real stock subset against the SAME target.
        // The stream's per-move proof covers extended backing, so it is not attached
        // to the report for the smaller box.
        (Box::new(compare_real_stock(check, producer.stock()?, bottom, cancel)?), None)
    } else { (report, sweep_proof) };
    if report.gouge.definite_cells > 0 {
        return Err(Failure::from(gouge_message(&report)));
    }
    if !report.complete {
        return Err(Failure::from(format!("camlib's comparison of the stock with the part ran out of budget with {} cells unexamined, so the job is not accepted", report.stats.unexamined_cells)));
    }
    Ok(Some(MaterialComparison { report: *report, sweep_proof }))
}

/// Describe camlib's evidence; this does not decide whether any assembly part collides.
fn stock_contact_reason(kind: &str, report: &camlib::stock_contact::Report, tool: &camlib::tool::ToolAssembly) -> Option<String> {
    use camlib::verification::ToolPart;
    let contacts: Vec<String> = report.findings.iter().filter(|c| c.finding.status == Status::Violation).map(|c| {
        let f = &c.finding;
        let part = match f.part { Some(ToolPart::Cutter) => "flutes", Some(ToolPart::Neck) => "neck", Some(ToolPart::Shank) => "shank", Some(ToolPart::Holder) => "holder", None => "tool assembly" };
        let section = f.holder_section.map(|n| format!(" section {n}")).unwrap_or_default();
        let point = c.contact_point_mm.or_else(|| f.witness.as_ref().map(|w| w.tip_mm));
        let at = point.map(|p| format!(" at X{:.1} Y{:.1} Z{}", p[0], p[1], capp::plan::display_position(p[2]))).unwrap_or_default();
        let mut text = format!("{}'s {part}{section} hits the remaining stock beside the {}{at}", tool.id, capp::plan::operation_noun(kind));
        let tool_kind = match tool.cutter.shape {
            camlib::tool::CutterShape::Drill { .. } => "a drill",
            camlib::tool::CutterShape::Chamfer { .. } => "a chamfer mill",
            _ if !tool.cutter.center_cutting && tool.cutter.neck.is_some() => "a thread mill",
            _ => "an end mill",
        };
        if matches!(f.part, Some(ToolPart::Holder | ToolPart::Shank | ToolPart::Neck))
            && let (Some(top), Some(tip)) = (c.obstruction_top_mm, c.lowest_tip_z_mm) {
                let query = format!("{tool_kind} of ⌀{}", tool.cutter.diameter_mm);
                text.push_str(&format!(": needs {}", capp::plan::assembly_requirement(&query, part, top - tip)));
                if let Some(radius) = c.max_clear_radius_mm {
                    // Round down: camlib's radius is already a conservative clearance bound.
                    let diameter = (2. * radius * 100.).floor() / 100.;
                    text.push_str(&format!(", or {part} diameter at most {diameter:.2} mm"));
                }
        }
        text
    }).collect();
    (!contacts.is_empty()).then(|| format!("camlib refused the {}: {}", capp::plan::operation_noun(kind), contacts.join("; ")))
}

/// Plans, simulates and checks one job. A definite stock-contact violation refuses the operation;
/// motions the check could not clear are counted.
pub fn simulate(manifest_json: &str) -> Result<Run, Failure> {
    simulate_with_stock_bottom(manifest_json, None, false, None)
}

/// Run an adapter job while excluding artificial through-tap backing from the
/// material comparison. The unmodified camlib manifest still drives every motion.
pub fn simulate_job(job: &crate::job::Job) -> Result<Run, Failure> {
    simulate_prepared_job(&mut job.clone())
}

/// Retain any camlib-admitted local pocket tops in the manifest shown with the moves.
pub(crate) fn simulate_prepared_job(job: &mut crate::job::Job) -> Result<Run, Failure> {
    let mut original_refusals = std::collections::HashMap::new();
    loop {
        let result = simulate_with_stock_bottom(&job.manifest.to_string(), Some(job.stock_bottom_mm), job.return_stock, job.crop_stock_to);
        let Err(failure) = result else { return result; };
        let reach = camlib::reach::ReachRefusal::parse(&failure.message);
        let operation = failure.operation.as_deref().unwrap_or("");
        if reach.as_ref().is_some_and(|r| r.limit == camlib::reach::ReachLimit::CuttingLength)
            && let Some(top) = job.local_pocket_tops.remove(operation)
            && let Some(op) = job.manifest["operations"].as_array_mut().and_then(|ops|
                ops.iter_mut().find(|op| op["id"] == operation))
            && op["top_z_mm"].as_f64().is_some_and(|old| top < old) {
                // Retry only a typed reach refusal. The whole job is replayed from its
                // initial stock, with the same checks. Keep the actual admitted manifest.
                op["top_z_mm"] = serde_json::json!(top);
                original_refusals.insert(operation.to_string(), failure);
                continue;
        }
        // A lower top that camlib cannot prove exposed does not replace the original
        // reach verdict with a stock-preparation complaint.
        if reach.is_none() && let Some(original) = original_refusals.remove(operation) {
            return Err(original);
        }
        return Err(failure);
    }
}

fn simulate_with_stock_bottom(manifest_json: &str, physical_bottom: Option<f64>, return_stock: bool, crop: Option<[[f64; 3]; 2]>) -> Result<Run, Failure> {
    let manifest: StreamManifest = serde_json::from_str(manifest_json).map_err(|e| e.to_string())?;
    let physical = physical_bottom.filter(|z| manifest.stock.height().is_ok_and(|s| *z > s.min_mm[2]));
    manifest.validate()?;
    let binding = Binding::new(&manifest)?;
    let mut producer = StreamProducer::new(manifest.clone()).map_err(|message| {
        // Only a demonstrated mesh conversion failure permits the box fallback.
        // Validation, identity and verifier errors keep their original meaning.
        if matches!(manifest.stock, camlib::stream::RunStockSpec::Mesh(_))
            && let Err(reason) = manifest.stock.initial_stock(&|| false)
            && reason == message {
            Failure::from(format!("initial stock: {reason}"))
        } else { Failure::from(message) }
    })?;
    let cancel = AtomicBool::new(false);
    let raw = serde_json::json!({"operations": manifest.operations, "tools": manifest.tools});
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
                Some((rough["stepup_mm"].as_f64().unwrap_or(rough["stepdown_mm"].as_f64()?) + rough["bottom_allowance_mm"].as_f64()?, radius))
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
                            // Adapter IDs retain the planner kind (notch/slot/etc.) even when
                            // camlib runs them as pocket jobs. Only the noun enters the reason.
                            let kind = operation.id().split('-').nth(2).unwrap_or("");
                            if let Check::Motion { stock_contact: Some(report), .. } = &checked.check
                                && let Some(reason) = stock_contact_reason(kind, report, tool) {
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
            material = compare(&mut producer, &binding, Some(operation.id()), &cancel, physical.map(|bottom| (&manifest, bottom)))?;
        }
    }
    if material.is_none() {
        material = compare(&mut producer, &binding, None, &cancel, physical.map(|bottom| (&manifest, bottom)))?;
    }
    producer.finish(DEFAULT_FRAME_LIMIT)?;
    let (material, sweep_proof) = material.map_or((None, None), |comparison| (Some(comparison.report), comparison.sweep_proof));
    let finished_stock = return_stock.then(|| {
        let stock = producer.stock()?;
        let mut snapshot = stock.snapshot();
        if let Some(bottom) = physical {
            snapshot.spec.min_mm[2] = bottom;
            for cell in &mut snapshot.cells {
                cell.lower_mm = cell.lower_mm.max(bottom);
                cell.upper_mm = cell.upper_mm.max(bottom);
            }
        }
        if let Some(bounds) = crop {
            crate::setup_stock::crop_cells(&mut snapshot, bounds)?;
        }
        Ok(camlib::stock_mesh::FinishedStock {
            mesh: camlib::stock_mesh::height_field_mesh(&snapshot.spec, &snapshot.cells, &cancel)?,
            approximation: Some(camlib::stock_mesh::HEIGHT_FIELD_APPROXIMATION),
        })
    });
    Ok(Run { moves, operations, uncertain, material: material.map(std::sync::Arc::new), sweep_proof, finished_stock })
}

#[cfg(test)]
mod contact_wording_tests {
    use super::*;
    use camlib::{stock_contact::{ContactFinding, Report}, verification::{Finding, FindingKind, ToolPart, Witness}};
    use serde_json::json;

    fn contact_fixture() -> (camlib::tool::ToolAssembly, Report) {
        let tool = serde_json::from_value(json!({"id":"em6", "cutter":{"id":"em6", "shape":{"kind":"flat"}, "diameter_mm":6., "cutting_length_mm":80., "overall_length_mm":100., "shank_diameter_mm":6., "flutes":2, "center_cutting":true}, "stickout_mm":90., "gauge_length_mm":120., "holders":[{"z_min_mm":90., "z_max_mm":120., "radius_min_mm":20., "radius_max_mm":20.}]})).unwrap();
        let report = Report {
            algorithm: "continuous_stock_contact_v1".into(),
            stock: camlib::stock::StockSpec { version:1, units:"mm".into(), min_mm:[0.,0.,-70.], max_mm:[60.,40.,0.], cell_size_mm:1., max_cells:10000 },
            pre_generation:0, options_sha256:String::new(), tool_sha256:String::new(), motion_sha256:String::new(), motion_id:1,
            operation:"top-3-notch".into(), tool:"em6".into(), scope:String::new(), status:Status::Violation, fully_examined:true,
            findings:vec![ContactFinding {
                finding:Finding {status:Status::Violation, kind:FindingKind::StockContact, object_id:"stock".into(), part:Some(ToolPart::Holder), holder_section:Some(0), fraction_interval:[0.,1.],
                    witness:Some(Witness {fraction:0.5, tip_mm:[1.,2.,-64.2], section_height_mm:Some(90.), margin_lower_mm:0.1}), detail:"camlib evidence".into()},
                cell_range:[0,0,1,1], solid_lower_mm:0., possible_upper_mm:0., contact_point_mm:Some([3.,4.,-1.]), obstruction_top_mm:Some(-10.), lowest_tip_z_mm:Some(-64.2), max_clear_radius_mm:Some(4.125)
            }], stats:Default::default(), numerical_policy:String::new()
        };
        (tool, report)
    }

    #[test]
    fn relays_assembly_part_section_and_camlib_position() {
        let (tool, mut report) = contact_fixture();
        let text = stock_contact_reason("notch", &report, &tool).unwrap();
        assert!(text.starts_with("camlib refused the notch: em6's holder section 0 hits the remaining stock beside the notch at X3.0 Y4.0 Z-1: needs "), "{text}");
        assert!(!text.contains(&report.operation), "{text}");
        assert!(text.contains(": needs an end mill of ⌀6 with its holder at least 55 mm above the tip, or holder diameter at most 8.25 mm"), "{text}");
        for (part, name) in [(ToolPart::Shank,"shank"), (ToolPart::Neck,"neck"), (ToolPart::Cutter,"flutes")] {
            report.findings[0].finding.part = Some(part);
            report.findings[0].finding.holder_section = None;
            report.findings[0].contact_point_mm = None;
            let text = stock_contact_reason("notch", &report, &tool).unwrap();
            assert!(text.contains(&format!("em6's {name} hits the remaining stock beside the notch at X1.0 Y2.0 Z-64.2")), "{text}");
            assert_eq!(text.contains(": needs "), matches!(part, ToolPart::Shank | ToolPart::Neck));
        }
        assert!(stock_contact_reason("unknown", &report, &tool).unwrap().starts_with("camlib refused the operation: "));
        report.findings[0].finding.status = Status::Uncertain;
        assert!(stock_contact_reason("notch", &report, &tool).is_none());
    }

    #[test]
    fn non_end_mill_contacts_request_their_own_tool_kind() {
        use camlib::tool::{CutterShape, Neck};
        let mut library: serde_json::Value = serde_json::from_str(include_str!("../../tests/fixtures/placeholder-tools.json")).unwrap();
        library.as_object_mut().unwrap().remove("machines");
        for (name, shape, neck, center_cutting) in [
            ("drill6", CutterShape::Drill { point_angle_rad: 118f64.to_radians() }, None, true),
            ("cm6", CutterShape::Chamfer { included_angle_rad: 90f64.to_radians(), tip_diameter_mm: 0. }, None, true),
            ("tm6", CutterShape::Flat, Some(Neck { diameter_mm: 4., length_mm: 50. }), false),
        ] {
            for (part, label) in [(ToolPart::Shank, "shank"), (ToolPart::Holder, "holder section 0")] {
                let (mut tool, mut report) = contact_fixture();
                tool.id = name.into();
                tool.cutter.id = name.into();
                report.tool = name.into();
                report.operation = "top-1-holes".into();
                tool.cutter.shape = shape.clone();
                tool.cutter.neck = neck.clone();
                tool.cutter.center_cutting = center_cutting;
                report.findings[0].finding.part = Some(part);
                report.findings[0].finding.holder_section = (part == ToolPart::Holder).then_some(0);
                let reason = stock_contact_reason("holes", &report, &tool).unwrap();
                let kind = match name { "drill6" => "a drill", "cm6" => "a chamfer mill", _ => "a thread mill" };
                assert!(reason.starts_with(&format!("camlib refused the holes: {name}'s {label} hits the remaining stock beside the holes at X3.0 Y4.0 Z-1: needs {kind} of ⌀6")), "{reason}");
                assert!(!reason.contains("end mill"), "{reason}");
                assert!(reason.contains("at least 55 mm above the tip"), "{reason}");
                let mut plan = json!({"setups":[{"id":"top", "stock":{"min":[0,0,-70],"max":[60,40,0]}, "operations":[
                    {"kind":"holes", "diameter":6., "top_z":0., "bottom_z":-64., "features":["holes:0"]}
                ]}]});
                capp::exclude(&mut plan, "top-1-holes", &reason).unwrap();
                capp::finish(&mut plan, &library, &[]).unwrap();
                assert_eq!(plan["skipped"][0]["reason"], format!("left out: {reason}"));
                assert!(plan["tools_needed"][0]["description"].as_str().unwrap().starts_with(kind), "{plan}");
            }
        }
    }

}

#[cfg(test)]
mod real_stock_tests {
    use super::*;
    use camlib::stock::{HeightBounds, Stock, StockSnapshot, StockSpec};
    use serde_json::json;

    #[test]
    fn comparison_excludes_only_the_backing_and_still_finds_real_gouges() {
        let check = |floor| serde_json::from_value(json!({
            "after_operation":null,
            "target":{"version":1,"units":"mm","id":"physical-stock","revision":"1",
                "machining_region":{"version":1,"units":"mm","name":"test","boundary_error_mm":0,
                    "components":[{"outer":{"vertices":[[0,0,0],[2,0,0],[2,2,0],[0,2,0]]},"holes":[]}]},
                "floor_z_mm":floor,"floor_error_mm":0}
        })).unwrap();
        let stock = |height| Stock::from_snapshot(StockSnapshot {
            spec: StockSpec {version:1, units:"mm".into(), min_mm:[0.,0.,-15.], max_mm:[2.,2.,0.], cell_size_mm:0.5, max_cells:16},
            generation:1, cells:vec![HeightBounds {lower_mm:height,upper_mm:height};16],
        }).unwrap();
        let cancel = AtomicBool::new(false);
        // Removal at -12 is wholly below the real stock bottom (-10).
        let report = compare_real_stock(&check(-10.), &stock(-12.), -10., &cancel).unwrap();
        assert!(report.complete);
        assert_eq!(report.context.stock.min_mm[2], -10.);
        assert_eq!(report.gouge.definite_cells, 0);
        // Removal at -6 gouges a -5 floor within the real stock: clipping must not hide it.
        let report = compare_real_stock(&check(-5.), &stock(-6.), -10., &cancel).unwrap();
        assert!(report.complete);
        assert!(report.gouge.definite_cells > 0);
    }
}
