//! step2cam: a STEP part's recognised features -> a plan, and the camlib job that carries it out,
//! simulated by camlib, with no Python and no server, so that it can run in a browser. step2cam
//! writes no G-code: camlib's own posts do, from the same job.
//!
//! specify-core's analysis and mesh -> [`capp`] (plan, settle, tools) -> [`job::jobs`] (the camlib
//! adapter) -> [`simulate::simulate`]. An operation camlib refuses goes back to capp as an exclusion.
pub mod job;
pub mod simulate;
mod workholding;

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
    let remainder = if flip { "sacrificial stock and anything a later setup cuts" } else { "the web and anything a later setup cuts" };
    format!(
        "finished stock against the part: no definite gouge. The cell comparison can miss an error under about {height:.2} mm in height or {cell:.2} mm sideways, or a misplaced cut narrower than about {:.1} mm. Up to {:.0} mm³ undecided; at least {:.0} mm³ stock remains, counting {remainder}",
        4. * cell, report.gouge.volume_mm3[1], report.residual.volume_mm3[0],
    )
}

fn simulate_all(plan: &Value, library: &Value, programs: &mut Vec<Value>, log: &mut Vec<String>) -> Result<(), simulate::Failure> {
    let (jobs, notes) = job::jobs(plan, library)?;
    log.extend(notes);
    for job in jobs {
        log.extend(job.notes.iter().map(|n| format!("note: {n}")));
        let run = simulate::simulate_job(&job).map_err(|f| simulate::Failure {
            message: format!("setup {}: {}", job.setup, f.message),
            ..f
        })?;
        if run.uncertain > 0 {
            log.push(format!("note: setup {}: camlib's stock-contact check could not clear {} motions", job.setup, run.uncertain));
        }
        log.push(format!("note: setup {}: {}", job.setup, material_note(run.material.as_ref(), &job.manifest, plan["setups"].as_array().into_iter().flatten().any(|s| s["flip_role"]=="sacrificial_grip"))));
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
        programs.push(json!({"setup": job.setup, "job": job.manifest, "operations": run.operations, "moves": run.moves, "material": material}));
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
fn flip_ready(plan: &Value, bottom: &Value) -> bool {
    let evidence=plan["decisions"].as_array().into_iter().flatten()
        .find(|d|d["id"]==bottom["clamping"]["because"][0])
        .map(|d|&d["chosen"]["prior_setups"]);
    evidence.is_some_and(|e|e["prepared_contacts"]==true && e["established_seat"]==true && e["status"]=="no_overlap")
        && bottom["operations"].as_array().into_iter().flatten().any(|o|o["kind"]=="facing" && o["tool"].is_string())
}

fn process_planned(analysis: &Value, mesh: &Value, library: &Value, mut planned: Value, mut exclusions: Vec<Value>) -> Result<Value, String> {
    let (mut programs, mut log, mut error) = (vec![], vec![], None);
    // What camlib refused, in order: capp plans again without each.
    let mut plan;
    loop {
        programs.clear();
        log.clear();
        plan = planned.clone();
        if let Err(message) = capp::finish(&mut plan, library, &exclusions) {
            error = Some(message);
            break;
        }
        if let Some(bottom)=plan["setups"].as_array().into_iter().flatten().find(|s|s["flip_seating"].is_object())
            && !flip_ready(&plan,bottom) {
            let reason=plan["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|w|w.starts_with("setup bottom") || w.starts_with("bottom-") || w.contains("wall_finish")).collect::<Vec<_>>().join("; ");
            planned=capp::flip::fallback(analysis,mesh,&reason)?;
            // Retried indices refer to the flip plan. Re-run ordinary planning
            // and exclusions from scratch, once, with the flip now withheld.
            exclusions.clear();
            continue;
        }
        let Err(failure) = simulate_all(&plan, library, &mut programs, &mut log) else {
            break;
        };
        // An operation capp can take out is excluded and the plan finished again; anything else
        // stops: nothing is given out, not even a setup that ran before this one stopped.
        let excludable = failure.operation.as_deref().is_some_and(|id| capp::exclude(&mut plan.clone(), id, "").is_some());
        let (true, Some(operation)) = (excludable, failure.operation) else {
            error = Some(failure.message);
            programs.clear();
            break;
        };
        let mut parts = operation.splitn(3, '-');
        let setup_id = parts.next().unwrap_or("");
        let index = parts.next().and_then(|n| n.parse::<usize>().ok()).and_then(|n| n.checked_sub(1));
        let refused_op = plan["setups"].as_array().and_then(|ss| ss.iter().find(|s| s["id"] == setup_id))
            .and_then(|s| index.and_then(|i| s["operations"].get(i)));
        let tool = refused_op.map(|op| op["tool"].clone()).unwrap_or(Value::Null);
        let selection = plan["decisions"].as_array().and_then(|ds| ds.iter().find(|d| d["chosen"] == tool
            && refused_op.is_some_and(|op| op["because"].as_array().is_some_and(|ids| ids.contains(&d["id"]))))).cloned();
        exclusions.push(json!({"operation": operation, "message": failure.message, "tool": tool, "selection": selection}));
    }
    let needs: Vec<&str> = plan["tools_needed"].as_array().into_iter().flatten()
        .filter_map(|n| n["description"].as_str()).collect();
    if !needs.is_empty() {
        log.push(format!("tools needed: {}", needs.join("; ")));
    }
    // The page has the mesh already; the plan's copy of it is not needed.
    for setup in plan["setups"].as_array_mut().into_iter().flatten() {
        if let Some(s) = setup.as_object_mut() {
            s.remove("part_surface");
        }
        for op in setup["operations"].as_array_mut().into_iter().flatten() {
            if let Some(o) = op.as_object_mut() {
                o.remove("positions");
                o.remove("indices");
            }
        }
    }
    Ok(json!({"plan": plan, "programs": programs, "log": log, "error": error}))
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
    #[test]
    fn explicit_ordinary_demo_programs_and_notes_match_ordinary_planning() {
        let resources: Value=serde_json::from_str(include_str!("../../demo/tools.json")).unwrap();
        for (a,m) in [
            (include_str!("../../tests/fixtures/plate.analysis.json"),include_str!("../../tests/fixtures/plate.mesh.json")),
            (include_str!("../../tests/fixtures/mounting-plate.analysis.json"),include_str!("../../tests/fixtures/mounting-plate.mesh.json")),
            (include_str!("../../tests/fixtures/synthetic/stepped-plate.analysis.json"),include_str!("../../tests/fixtures/synthetic/stepped-plate.mesh.json")),
        ] {
            let a: Value=serde_json::from_str(a).unwrap();let m: Value=serde_json::from_str(m).unwrap();
            let ordinary=capp::plan::plan(&a,Some(&m),&Default::default()).unwrap();
            let ordinary=process_planned(&a,&m,&resources,ordinary,vec![]).unwrap();
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
    fn excluded_roughing_cannot_leave_a_finished_grip_and_process_replans() {
        let a: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.analysis.json")).unwrap();
        let m: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.mesh.json")).unwrap();
        let mut t: Value=serde_json::from_str(include_str!("../../demo/tools.json")).unwrap();
        t["overrides"]["workholding_option"]=json!("vise_flip_face");
        let planned=capp::flip::plan(&a,&m,&t).unwrap();
        let mut resolved=planned.clone();capp::finish(&mut resolved,&t,&[]).unwrap();
        let n=resolved["setups"][0]["operations"].as_array().unwrap().iter().position(|o|o["kind"]=="roughing").unwrap()+1;
        let exclusion=json!({"operation":format!("top-{n}-roughing"),"message":"test roughing refusal"});
        let mut finished=planned.clone();capp::finish(&mut finished,&t,std::slice::from_ref(&exclusion)).unwrap();
        let bottom=&finished["setups"][1];
        assert!(bottom["operations"].as_array().unwrap().is_empty());
        assert!(!flip_ready(&finished,bottom));
        let decision=finished["decisions"].as_array().unwrap().iter().find(|d|d["id"]==bottom["clamping"]["because"][0]).unwrap();
        assert_eq!(decision["chosen"]["prior_setups"]["prepared_contacts"],false);
        assert_eq!(decision["outcome"],"none");
        assert!(!decision["says"].as_str().unwrap().contains("outline already finished"));
        assert!(decision["says"].as_str().unwrap().contains("preparation not established"));
        let out=process_planned(&a,&m,&t,planned,vec![exclusion]).unwrap();
        assert!(out["error"].is_null(),"{}",out["error"]);
        assert!(out["plan"]["setups"].as_array().unwrap().iter().all(|s|s.get("flip_seating").is_none()));
        assert_eq!(out["plan"]["setups"][0]["stock"]["min"][2],-19.);
        assert!(out["plan"]["setups"][0]["operations"].as_array().unwrap().iter().any(|o|o["kind"]=="roughing" && o["tool"].is_string()));
        let ordinary=capp::plan::plan(&a,Some(&m),&Default::default()).unwrap();
        let ordinary=process_planned(&a,&m,&t,ordinary,vec![]).unwrap();
        let reasons=|p: &Value|p["skipped"].as_array().unwrap().iter().map(|s|(s["feature"].clone(),s["reason"].clone())).collect::<Vec<_>>();
        assert_eq!(reasons(&out["plan"]),reasons(&ordinary["plan"]));
        assert_eq!(out["plan"]["covered_by_roughing"],ordinary["plan"]["covered_by_roughing"]);
    }

    #[test]
    fn no_top_cuts_or_missing_seating_cannot_establish_a_flip_grip() {
        let a: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.analysis.json")).unwrap();
        let m: Value=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.mesh.json")).unwrap();
        let mut t: Value=serde_json::from_str(include_str!("../../demo/tools.json")).unwrap();
        t["overrides"]["workholding_option"]=json!("vise_flip_face");
        let mut empty=capp::flip::plan(&a,&m,&t).unwrap();
        empty["setups"][0]["operations"]=json!([]);
        capp::finish(&mut empty,&t,&[]).unwrap();
        assert!(empty["setups"][1]["operations"].as_array().unwrap().is_empty());
        assert!(!flip_ready(&empty,&empty["setups"][1]));
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
        let mut t: Value=serde_json::from_str(include_str!("../../demo/tools.json")).unwrap();
        t["overrides"]["workholding_option"]=json!("vise_flip_face");
        let mut p=capp::flip::plan(&a,&m,&t).unwrap();
        // Distort a lower corner arc while preserving the straight grip faces: camlib must refuse it, after
        // which the bottom grip loses its preparation and triggers fallback.
        let wall=p["setups"][0]["operations"].as_array_mut().unwrap().iter_mut().find(|o|o["kind"]=="wall_finish").unwrap();
        wall["outer"][0][2]=json!(2.);
        let mut checked=p.clone();capp::finish(&mut checked,&t,&[]).unwrap();
        assert!(checked["setups"][1]["operations"].as_array().unwrap().iter().any(|o|o["kind"]=="facing"));
        let out=process_planned(&a,&m,&t,p,vec![]).unwrap();
        assert!(out["error"].is_null(),"{}",out["error"]);
        let p=&out["plan"];
        assert!(p["setups"].as_array().unwrap().iter().all(|s|s.get("flip_seating").is_none()));
        assert_eq!(p["setups"][0]["stock"]["min"][2],-19.);
        assert!(p["decisions"].as_array().unwrap().iter().any(|d|d["chosen"]["available"]==false && d["says"].as_str().unwrap().contains("replanned without sacrificial bottom stock")));
        assert_eq!(out["programs"].as_array().unwrap().len(),2);
    }
}
