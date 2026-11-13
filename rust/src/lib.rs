//! step2cam: a STEP part's recognised features -> a plan, and the camlib job that carries it out,
//! simulated by camlib, with no Python and no server, so that it can run in a browser. step2cam
//! writes no G-code: camlib's own posts do, from the same job.
//!
//! specify-core's analysis and mesh -> [`capp`] (plan, settle, tools) -> [`job::jobs`] (the camlib
//! adapter) -> [`simulate::simulate`]. An operation camlib refuses goes back to capp as an exclusion.
pub mod job;
pub mod post_machine;
pub mod simulate;
mod workholding;
mod setup_stock;

use capp::kind::OpKind;
use capp::model::{Cut, Exclusion, Plan, Setup};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

/// What camlib's comparison of the finished stock with the part found, for the log.
fn material_note(report: Option<&camlib::material::Report>, manifest: &Value, flip: bool) -> String {
    let Some(report) = report else {
        return "the finished stock is not compared with the part in this setup: it holds no mesh of the part that is fine enough".into();
    };
    // The mesh's error widens every cell, flat faces too, and only a cell the tool wholly covers
    // counts as cut: errors smaller than these pass, and they differ from part to part.
    let height = manifest["verification"]["material"][0]["target"]["surface"]["surface_error_mm"].as_f64().unwrap_or(0.);
    let cell = manifest["stock"]["cell_size_mm"].as_f64().unwrap_or(0.);
    let remainder = if flip { "sacrificial stock and anything a later setup cuts" } else { "uncut stock and anything a later setup cuts" };
    format!(
        "finished stock against the part: no definite gouge. The cell comparison can miss an error under about {height:.2} mm in height or {cell:.2} mm sideways, or a misplaced cut narrower than about {:.1} mm. Up to {:.0} mm³ undecided; at least {:.0} mm³ stock remains, counting {remainder}",
        4. * cell, report.gouge.volume_mm3[1], report.residual.volume_mm3[0],
    )
}

#[derive(Clone)]
struct CompletedSetup {
    input: Value,
    job: job::Job,
    run: simulate::Run,
    stock_note: Option<String>,
}

fn simulate_all(plan: &mut Plan, library: &Value, programs: &mut Vec<Value>, log: &mut Vec<String>, cache: &mut Vec<CompletedSetup>) -> Result<(), simulate::Failure> {
    // Keep the original coordinate/stock descriptions while reporting the actual premise.
    let frames = plan.setups.clone();
    let plan_part = plan.part.clone();
    for (index, setup) in plan.setups.iter_mut().enumerate().skip(1) {
        setup.stock_note = None;
        if let Some(step) = setup_stock::breakout(&frames[index - 1], plan_part.as_ref()) {
            setup.before_setup = Some(step);
        }
    }
    let (jobs, notes) = job::jobs_from(plan, library)?;
    log.extend(notes);
    let count = jobs.len();
    let mut previous: Option<(&Setup, Result<camlib::stock_mesh::FinishedStock, String>, bool, bool)> = None;
    for (index, mut job) in jobs.into_iter().enumerate() {
        job.return_stock = index + 1 < count;
        let box_manifest = job.manifest.clone();
        let box_bottom = job.stock_bottom_mm;
        let frame = frames.iter().find(|s| s.id == job.setup).ok_or("missing setup frame")?;
        let setup_index = frames.iter().position(|s| s.id == job.setup).unwrap();
        if setup_index > 0 {
            let carried = match &previous {
                Some((from, Ok(stock), _, _)) => setup_stock::carry(stock, from, frame, &job.manifest).and_then(|(manifest, bottom)| setup_stock::adapt(manifest, job.stock_bottom_mm.min(bottom), frame, plan.part.as_ref(), library)),
                Some((_, Err(reason), _, _)) => Err(reason.clone()),
                None => Err("the previous setup returned no finished stock".into()),
            };
            let note = match carried {
                Ok(mut adapted) => {
                    adapted.return_stock = job.return_stock;
                    job = adapted;
                    let from = previous.as_ref().unwrap();
                    setup_stock::carried_note(&from.0.id, from.2, from.3)
                }
                Err(reason) => setup_stock::assumption(&reason),
            };
            plan.setups[setup_index].stock_note = Some(note);
        }
        if setup_stock::released(frame, plan_part.as_ref()) {
            job.crop_stock_to = Some(setup_stock::part_bounds(plan_part.as_ref(), frame)?);
        }
        let input = if job.return_stock || cache.get(index).is_some() { job.manifest.clone() } else { Value::Null };
        log.extend(job.notes.iter().map(|n| format!("note: {n}")));
        let run = if let Some(saved) = cache.get(index).filter(|c| c.input == input && c.job.crop_stock_to == job.crop_stock_to && (!job.return_stock || c.job.return_stock)) {
            job = saved.job.clone();
            if setup_index > 0 { plan.setups[setup_index].stock_note = saved.stock_note.clone(); }
            saved.run.clone()
        } else {
            cache.truncate(index);
            let attempt = simulate::simulate_prepared_job(&mut job);
            let attempt = match attempt {
                Err(f) if f.operation.is_none() && f.cause == simulate::Cause::InitialStock && job.manifest["stock"]["mesh"].is_object() => {
                    plan.setups[setup_index].stock_note = Some(setup_stock::assumption(&f.message));
                    job.manifest = box_manifest;
                    job.stock_bottom_mm = box_bottom;
                    simulate::simulate_prepared_job(&mut job)
                }
                other => other,
            };
            let run = attempt.map_err(|f| simulate::Failure {
                message: format!("setup {}: {}", job.setup, f.message),
                ..f
            })?;
            if job.return_stock { cache.push(CompletedSetup { input, job: job.clone(), run: run.clone(), stock_note: plan.setups[setup_index].stock_note.clone() }); }
            run
        };
        if setup_index > 0 && job.manifest["stock"]["mesh"].is_object() {
            let corner = |key: &str| serde_json::from_value(job.manifest["stock"][key].clone()).map_err(|e| e.to_string());
            plan.setups[setup_index].stock = capp::model::Box3 { min: corner("min_mm")?, max: corner("max_mm")? };
        }
        previous = Some((frame, run.finished_stock.clone().unwrap_or_else(|| Err("finished-stock export was not requested".into())), job.crop_stock_to.is_some(), setup_index > 0 && job.manifest["stock"]["mesh"].is_null()));
        if run.uncertain > 0 {
            log.push(format!("note: setup {}: camlib's stock-contact check could not clear {} motions", job.setup, run.uncertain));
        }
        log.push(format!("note: setup {}: {}", job.setup, material_note(run.material.as_deref(), &job.manifest, plan.setups.iter().any(|s| s.flip_role.as_deref() == Some("sacrificial_grip")))));
        if let Some(proof) = &run.sweep_proof {
            log.push(format!(
                "note: setup {}: per-motion gouge proof: {} proven, {} unproven; {} possible gouge cells decided at {} mm tolerance{}",
                job.setup, proof.proven_moves, proof.unproven_moves, proof.decided_cells, proof.tolerance_mm,
                proof.unavailable.as_ref().map_or(String::new(), |reason| format!("; unavailable: {reason}")),
            ));
        }
        // The cell comparison's resolution, for the page to word.
        let check = &job.manifest["verification"]["material"][0];
        let limits = json!({
            "height_mm": check["target"]["surface"]["surface_error_mm"], "sideways_mm": job.manifest["stock"]["cell_size_mm"],
            "before_threads": !check["after_operation"].is_null(),
        });
        let material = run.material.map(|r| json!({"complete": r.complete, "residual": r.residual, "gouge": r.gouge, "limits": limits, "sweep_proof": run.sweep_proof}));
        programs.push(json!({"setup": job.setup, "job": job.manifest, "operations": run.operations, "moves": run.moves, "material": material, "uncertain_motions": run.uncertain}));
    }
    Ok(())
}

/// Everything the page draws, from specify-core's analysis and mesh and a tool library. An
/// operation camlib refuses is left out with a warning, so the rest of the part can still be
/// seen; a part that plans but cannot be simulated at all still comes back, with `error`.
pub fn process(analysis: &Value, mesh: &Value, library: &Value) -> Result<Value, String> {
    let normalized = capp::resources::normalize(library)?;
    let library = normalized.as_ref();
    workholding::process(analysis, mesh, library)
}

/// Both positive preparation evidence and an executable facing are required.
fn flip_ready(plan: &Plan, bottom: &Setup) -> bool {
    let because = bottom.clamping.as_ref().map_or(&Value::Null, |c| &c["because"][0]);
    let evidence=plan.decisions.iter().flatten()
        .find(|d|d["id"]==*because)
        .map(|d|&d["chosen"]["prior_setups"]);
    evidence.is_some_and(|e|e["prepared_contacts"]==true && e["established_seat"]==true && e["status"]=="no_overlap")
        && bottom.operations.iter().any(|o|o.cut.kind()==OpKind::Facing && matches!(o.tool, Some(Some(_))))
}

/// What [`process`] gives out, before it is encoded: the finished plan, the camlib programs, the
/// log, and the error that stopped it, if any.
struct Outcome {
    plan: Plan,
    programs: Vec<Value>,
    log: Vec<String>,
    error: Option<String>,
}

impl Outcome {
    /// The output as JSON, without the plan's copies of the mesh.
    fn json(mut self) -> Value {
        // The page has the mesh already; the plan's copy of it is not needed.
        self.plan.contact_surface_model = None;
        self.plan.workholding_surface_model = None;
        for setup in &mut self.plan.setups {
            setup.part_surface = None;
            setup.grip_surface_model = None;
            for op in &mut setup.operations {
                if let Cut::Roughing(rough) = &mut op.cut {
                    rough.positions = None;
                    rough.indices = None;
                }
            }
        }
        json!({"plan": self.plan, "programs": self.programs, "log": self.log, "error": self.error})
    }
}

/// Simulates the plan, and plans again without each operation camlib refuses. The loop ends: each
/// pass stops, falls back from the flip (at most once), or leaves out one more operation, and the
/// operations left out since the last start are at most the operations that start's plan had.
fn process_planned(analysis: &Value, mesh: &Value, library: &Value, mut planned: Plan, mut exclusions: Vec<Exclusion>) -> Result<Outcome, String> {
    let (mut programs, mut log, mut error) = (vec![], vec![], None);
    // What camlib refused, in order: capp plans again without each.
    let mut plan;
    let mut setup_cache = vec![];
    let mut fell_back = false;
    // Since the last start: the operations left out, and how many the first finished plan had.
    let mut refused: Vec<Option<capp::model::Operation>> = vec![];
    let mut planned_operations = None;
    loop {
        programs.clear();
        log.clear();
        plan = planned.clone();
        if let Err(message) = capp::finish_plan(&mut plan, library, &exclusions) {
            error = Some(message);
            break;
        }
        let operations = *planned_operations.get_or_insert_with(|| plan.setups.iter().map(|s| s.operations.len()).sum::<usize>());
        if let Some(bottom)=plan.setups.iter().find(|s|s.flip_seating.as_ref().is_some_and(Value::is_object))
            && !flip_ready(&plan,bottom) {
            if fell_back {
                error = Some("replanning stopped: the plan without the flip still asks for one".into());
                break;
            }
            let reason=plan.warnings.iter().map(String::as_str).filter(|w|w.starts_with("setup bottom") || w.starts_with("bottom-") || w.contains("wall_finish")).collect::<Vec<_>>().join("; ");
            planned=capp::flip::build_fallback(analysis,mesh,&reason)?;
            // Retried indices refer to the flip plan. Re-run ordinary planning
            // and exclusions from scratch, once, with the flip now withheld.
            exclusions.clear();
            (fell_back, refused, planned_operations) = (true, vec![], None);
            continue;
        }
        let Err(failure) = simulate_all(&mut plan, library, &mut programs, &mut log, &mut setup_cache) else {
            break;
        };
        // An operation capp can take out is excluded and the plan finished again; anything else
        // stops: nothing is given out, not even a setup that ran before this one stopped.
        let excludable = failure.operation.as_ref().is_some_and(|id| capp::excludable(&plan, id));
        let (true, Some(operation)) = (excludable, failure.operation) else {
            error = Some(failure.message);
            programs.clear();
            break;
        };
        let refused_op = plan.setups.iter().find(|s| s.id == operation.setup)
            .and_then(|s| operation.number.checked_sub(1).and_then(|i| s.operations.get(i)));
        // Leaving out what is already left out, or more operations than there were, would not end.
        // The same cut is the same operation, whichever decisions the pass numbered behind it.
        let cut = refused_op.map(|op| capp::model::Operation { because: None, ..op.clone() });
        let stop = if cut.is_some() && refused.contains(&cut) {
            Some(format!("replanning stopped: camlib refused {operation} again after it was left out ({})", failure.message))
        } else if refused.len() >= operations {
            Some(format!("replanning stopped after leaving out {} of {operations} operations: camlib still refused {operation} ({})", refused.len(), failure.message))
        } else {
            None
        };
        if let Some(message) = stop {
            error = Some(message);
            programs.clear();
            break;
        }
        refused.push(cut);
        let tool = refused_op.and_then(|op| op.tool.clone().flatten());
        let selection = plan.decisions.as_ref().and_then(|ds| ds.iter().find(|d| d["chosen"] == json!(tool)
            && refused_op.is_some_and(|op| op.because.as_ref().is_some_and(|ids| ids.iter().any(|id| d["id"] == *id))))).cloned();
        let reach = match failure.cause { simulate::Cause::Reach(reach) => Some(*reach), _ => None };
        exclusions.push(Exclusion { operation, message: failure.message, tool, selection, reach });
    }
    let needs: Vec<&str> = plan.tools_needed.iter().flatten().map(|n| n.description.as_str()).collect();
    if !needs.is_empty() {
        log.push(format!("tools needed: {}", needs.join("; ")));
    }
    Ok(Outcome { plan, programs, log, error })
}

thread_local! {
    /// The last panic's message: in the browser a panic ends in a trap that says nothing.
    static PANIC: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Keeps each panic's message for [`last_panic`], then reports it as before.
#[wasm_bindgen(start)]
pub fn keep_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info.payload().downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "no message".into());
        let place = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        PANIC.with(|p| *p.borrow_mut() = Some(format!("step2cam stopped on an internal error: {message}{place}")));
        previous(info);
    }));
}

/// The message of the panic that stopped the last call, once; for the page after a trap.
#[wasm_bindgen(js_name = lastPanic)]
pub fn last_panic() -> Option<String> {
    PANIC.with(|p| p.borrow_mut().take())
}

/// For the browser: JSON strings in, a JSON string out.
#[wasm_bindgen(js_name = process)]
pub fn process_json(analysis: &str, mesh: &str, library: &str) -> Result<String, JsError> {
    let parse = |text: &str| serde_json::from_str::<Value>(text).map_err(|e| JsError::new(&e.to_string()));
    let (analysis, mesh, library) = (parse(analysis)?, parse(mesh)?, parse(library)?);
    let out = process(&analysis, &mesh, &library).map_err(|e| JsError::new(&e))?;
    Ok(out.to_string())
}

#[cfg(test)]
mod flip_tests {
    use super::*;
    /// A JSON plan or setup as its type, for the functions that take one.
    fn typed<T: serde::de::DeserializeOwned>(value: &Value) -> T {
        serde_json::from_value(value.clone()).unwrap()
    }
    #[test]
    fn explicit_ordinary_demo_programs_and_notes_match_ordinary_planning() {
        let resources: Value=serde_json::from_str(include_str!("../../tests/fixtures/placeholder-tools.json")).unwrap();
        for (a,m) in [
            (include_str!("../../tests/fixtures/plate.analysis.json"),include_str!("../../tests/fixtures/plate.mesh.json")),
            (include_str!("../../tests/fixtures/mounting-plate.analysis.json"),include_str!("../../tests/fixtures/mounting-plate.mesh.json")),
            (include_str!("../../tests/fixtures/synthetic/stepped-plate.analysis.json"),include_str!("../../tests/fixtures/synthetic/stepped-plate.mesh.json")),
        ] {
            let a: Value=serde_json::from_str(a).unwrap();let m: Value=serde_json::from_str(m).unwrap();
            let ordinary=capp::plan::build(&a,Some(&m),&Default::default()).unwrap();
            let ordinary=process_planned(&a,&m,&resources,ordinary,vec![]).unwrap().json();
            let mut legacy=resources.clone();legacy["overrides"]["workholding_option"]=json!("none");
            let mut supplied=resources.clone();supplied["decisions"]=json!([{"rule":"setup.workholding","subject":"part","choice":"vise_ordinary","source":"user","reason":"ordinary parity check"}]);
            for t in [legacy,supplied] {
                let out=process(&a,&m,&t).unwrap();
                assert!(out["error"].is_null(),"{}",out["error"]);
                assert_eq!(out["programs"],ordinary["programs"]);
                assert_eq!(out["log"],ordinary["log"]);
                assert!(!out["plan"]["setups"].as_array().unwrap().iter().any(|s|s.get("flip_seating").is_some()));
            }
        }
    }

    #[test]
    fn each_program_counts_its_uncertain_motions_as_the_log_does() {
        let a: Value=serde_json::from_str(include_str!("../../tests/fixtures/plate.analysis.json")).unwrap();
        let m: Value=serde_json::from_str(include_str!("../../tests/fixtures/plate.mesh.json")).unwrap();
        let t: Value=serde_json::from_str(include_str!("../../tests/fixtures/placeholder-tools.json")).unwrap();
        let out=process(&a,&m,&t).unwrap();
        assert!(out["error"].is_null(),"{}",out["error"]);
        let programs=out["programs"].as_array().unwrap();
        assert!(!programs.is_empty());
        for p in programs {
            let count=p["uncertain_motions"].as_u64().unwrap();
            let line=format!("note: setup {}: camlib's stock-contact check could not clear {count} motions",p["setup"].as_str().unwrap());
            let logged=out["log"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|l|l.contains(&format!("setup {}: camlib's stock-contact check",p["setup"].as_str().unwrap()))).collect::<Vec<_>>();
            assert_eq!(logged,if count>0 {vec![line.as_str()]} else {vec![]});
        }
    }
    #[test]
    fn excluded_roughing_cannot_leave_a_finished_grip_and_process_replans() {
        let a: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.analysis.json")).unwrap();
        let m: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.mesh.json")).unwrap();
        let mut t: Value=serde_json::from_str(include_str!("../../tests/fixtures/placeholder-tools.json")).unwrap();
        t["overrides"]["workholding_option"]=json!("vise_flip_face");
        let planned=capp::flip::plan(&a,&m,&t).unwrap();
        let mut resolved=planned.clone();capp::finish(&mut resolved,&t,&[]).unwrap();
        let n=resolved["setups"][0]["operations"].as_array().unwrap().iter().position(|o|o["kind"]=="roughing").unwrap()+1;
        let exclusion=json!({"operation":format!("top-{n}-roughing"),"message":"test roughing refusal"});
        let mut finished=planned.clone();capp::finish(&mut finished,&t,std::slice::from_ref(&exclusion)).unwrap();
        let bottom=&finished["setups"][1];
        assert!(bottom["operations"].as_array().unwrap().is_empty());
        assert!(!flip_ready(&typed(&finished),&typed(bottom)));
        let decision=finished["decisions"].as_array().unwrap().iter().find(|d|d["id"]==bottom["clamping"]["because"][0]).unwrap();
        assert_eq!(decision["chosen"]["prior_setups"]["prepared_contacts"],false);
        assert_eq!(decision["outcome"],"none");
        assert!(!decision["says"].as_str().unwrap().contains("outline already finished"));
        assert!(decision["says"].as_str().unwrap().contains("preparation not established"));
        let out=process_planned(&a,&m,&t,typed(&planned),vec![typed(&exclusion)]).unwrap().json();
        assert!(out["error"].is_null(),"{}",out["error"]);
        assert!(out["plan"]["setups"].as_array().unwrap().iter().all(|s|s.get("flip_seating").is_none()));
        assert_eq!(out["plan"]["setups"][0]["stock"]["min"][2],-19.);
        assert!(out["plan"]["setups"][0]["operations"].as_array().unwrap().iter().any(|o|o["kind"]=="roughing" && o["tool"].is_string()));
        let ordinary=capp::plan::build(&a,Some(&m),&Default::default()).unwrap();
        let ordinary=process_planned(&a,&m,&t,ordinary,vec![]).unwrap().json();
        let reasons=|p: &Value|p["skipped"].as_array().unwrap().iter().map(|s|(s["feature"].clone(),s["reason"].clone())).collect::<Vec<_>>();
        assert_eq!(reasons(&out["plan"]),reasons(&ordinary["plan"]));
        assert_eq!(out["plan"]["covered_by_roughing"],ordinary["plan"]["covered_by_roughing"]);
    }

    #[test]
    fn no_top_cuts_or_missing_seating_cannot_establish_a_flip_grip() {
        let a: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.analysis.json")).unwrap();
        let m: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.mesh.json")).unwrap();
        let mut t: Value=serde_json::from_str(include_str!("../../tests/fixtures/placeholder-tools.json")).unwrap();
        t["overrides"]["workholding_option"]=json!("vise_flip_face");
        let mut empty=capp::flip::plan(&a,&m,&t).unwrap();
        empty["setups"][0]["operations"]=json!([]);
        capp::finish(&mut empty,&t,&[]).unwrap();
        assert!(empty["setups"][1]["operations"].as_array().unwrap().is_empty());
        assert!(!flip_ready(&typed(&empty),&typed(&empty["setups"][1])));
        let mut unseated=capp::flip::plan(&a,&m,&t).unwrap();
        // Keep finished sides and roughing, but remove the finishing of the
        // actual Z12 support plane; facing at the boss top cannot replace it.
        unseated["setups"][0]["operations"].as_array_mut().unwrap().retain(|o|o["kind"]!="flats");
        capp::finish(&mut unseated,&t,&[]).unwrap();
        let bottom=&unseated["setups"][1];
        let d=unseated["decisions"].as_array().unwrap().iter().find(|d|d["id"]==bottom["clamping"]["because"][0]).unwrap();
        assert_eq!(d["chosen"]["prior_setups"]["prepared_contacts"],true);
        assert_eq!(d["chosen"]["prior_setups"]["established_seat"],false);
        assert!(bottom["operations"].as_array().unwrap().is_empty());
    }

    #[test]
    fn refused_preparation_replans_without_the_flip_and_resets_exclusions() {
        let a: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.analysis.json")).unwrap();
        let m: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.mesh.json")).unwrap();
        let mut t: Value=serde_json::from_str(include_str!("../../tests/fixtures/placeholder-tools.json")).unwrap();
        t["overrides"]["workholding_option"]=json!("vise_flip_face");
        let mut p=capp::flip::plan(&a,&m,&t).unwrap();
        // Distort a lower corner arc while preserving the straight grip faces: camlib must refuse it, after
        // which the bottom grip loses its preparation and triggers fallback.
        let wall=p["setups"][0]["operations"].as_array_mut().unwrap().iter_mut().find(|o|o["kind"]=="wall_finish").unwrap();
        wall["outer"][0][2]=json!(2.);
        let mut checked=p.clone();capp::finish(&mut checked,&t,&[]).unwrap();
        assert!(checked["setups"][1]["operations"].as_array().unwrap().iter().any(|o|o["kind"]=="facing"));
        let out=process_planned(&a,&m,&t,typed(&p),vec![]).unwrap().json();
        assert!(out["error"].is_null(),"{}",out["error"]);
        let p=&out["plan"];
        assert!(p["setups"].as_array().unwrap().iter().all(|s|s.get("flip_seating").is_none()));
        assert_eq!(p["setups"][0]["stock"]["min"][2],-19.);
        assert!(p["decisions"].as_array().unwrap().iter().any(|d|d["chosen"]["available"]==false && d["says"].as_str().unwrap().contains("replanned without sacrificial bottom stock")));
        assert_eq!(out["programs"].as_array().unwrap().len(),2,"log: {}; warnings: {}",out["log"],out["plan"]["warnings"]);
    }

    /// The reason the flip was withheld, as the fallback plan's decision gives it.
    fn fallback_reason(out: &Value) -> &str {
        let says=out["plan"]["decisions"].as_array().unwrap().iter().filter_map(|d|d["says"].as_str())
            .find_map(|s|s.strip_prefix("vise flip option withheld; replanned without sacrificial bottom stock: "));
        says.expect("the plan fell back")
    }

    #[test]
    fn the_fallback_reason_is_the_bottom_setup_and_wall_finish_warnings() {
        let a: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.analysis.json")).unwrap();
        let m: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.mesh.json")).unwrap();
        let mut t: Value=serde_json::from_str(include_str!("../../tests/fixtures/placeholder-tools.json")).unwrap();
        t["overrides"]["workholding_option"]=json!("vise_flip_face");
        // A wall_finish camlib refuses: its warning, and the bottom setup's, which start "setup bottom".
        let mut p=capp::flip::plan(&a,&m,&t).unwrap();
        let wall=p["setups"][0]["operations"].as_array_mut().unwrap().iter_mut().find(|o|o["kind"]=="wall_finish").unwrap();
        wall["outer"][0][2]=json!(2.);
        let out=process_planned(&a,&m,&t,typed(&p),vec![]).unwrap().json();
        assert_eq!(fallback_reason(&out),"top-13-wall_finish left out: setup top: boundary vertices require finite coordinates within ±1e6 mm and bulge within [-1,1]; \
            setup bottom: earlier cuts may have removed material in the later grip band: [{\"faces\":[\"stock.x.min\",\"stock.x.max\"],\"operation\":\"top-2-roughing\"}]; \
            checks stock-face contact bands from stock bottom to jaw top against earlier tooled operations' conservative cutting envelopes, transformed through setup rotation and origin_model; \
            roughing/contour envelopes include preserved material and grouped holes include the space between them; not a stock simulation or proof of face retention, contact area or holding strength; \
            missing frames or geometry are unchecked; certain part-face contact per jaw: 158.7500 mm², 158.7500 mm²; carried upper-bound stock contributes no contact");
        // A bottom operation left out: its warning starts "bottom-".
        let p=capp::flip::plan(&a,&m,&t).unwrap();
        let exclusion=json!({"operation":"bottom-1-facing","message":"test facing refusal"});
        let out=process_planned(&a,&m,&t,typed(&p),vec![typed(&exclusion)]).unwrap().json();
        assert_eq!(fallback_reason(&out),"bottom-1-facing left out: test facing refusal");
    }
}

#[cfg(test)]
mod robust_tests {
    use super::*;
    fn fixture(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn a_face_range_past_the_triangles_is_an_error_naming_the_face() {
        let a=fixture(include_str!("../../tests/fixtures/plate.analysis.json"));
        let mut m=fixture(include_str!("../../tests/fixtures/plate.mesh.json"));
        let t=fixture(include_str!("../../tests/fixtures/placeholder-tools.json"));
        let triangles=m["indices"].as_array().unwrap().len()/3;
        m["faces"][3]=json!([1_000_000,758]);
        let message=match process(&a,&m,&t) { Err(e)=>e, Ok(out)=>out["error"].as_str().unwrap_or_default().to_string() };
        assert_eq!(message,format!("mesh: face 3's triangles 1000000..1000758 run past the {triangles} in the mesh"));
    }

    #[test]
    fn camlib_refusing_an_operation_already_left_out_stops_replanning() {
        let a=fixture(include_str!("../../tests/fixtures/synthetic/stepped-plate.analysis.json"));
        let m=fixture(include_str!("../../tests/fixtures/synthetic/stepped-plate.mesh.json"));
        let t=fixture(include_str!("../../tests/fixtures/placeholder-tools.json"));
        // Two copies of a wall finish camlib refuses: the second is the same cut, refused again.
        let mut p=serde_json::to_value(capp::plan::build(&a,Some(&m),&Default::default()).unwrap()).unwrap();
        let ops=p["setups"][0]["operations"].as_array_mut().unwrap();
        let n=ops.iter().position(|o|o["kind"]=="wall_finish").unwrap();
        ops[n]["outer"][0][2]=json!(2.);
        let copy=ops[n].clone();
        ops.insert(n,copy);
        let out=process_planned(&a,&m,&t,serde_json::from_value(p).unwrap(),vec![]).unwrap().json();
        let error=out["error"].as_str().unwrap();
        assert!(error.starts_with("replanning stopped: camlib refused top-") && error.contains("-wall_finish again after it was left out"),"{error}");
        assert_eq!(out["programs"],json!([]));
    }

    #[test]
    fn a_panic_leaves_its_message_for_the_page() {
        keep_panics();
        assert!(std::panic::catch_unwind(|| panic!("test panic {}", 7)).is_err());
        let message=last_panic().unwrap();
        assert!(message.starts_with("step2cam stopped on an internal error: test panic 7 at src/lib.rs:"),"{message}");
        assert_eq!(last_panic(),None);
    }
}
