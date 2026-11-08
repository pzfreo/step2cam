//! step2cam: a STEP part's recognised features -> a plan, and the camlib job that carries it out,
//! simulated by camlib, with no Python and no server, so that it can run in a browser. step2cam
//! writes no G-code: camlib's own posts do, from the same job.
//!
//! specify-core's analysis and mesh -> [`capp`] (plan, settle, tools) -> [`job::jobs`] (the camlib
//! adapter) -> [`simulate::simulate`]. An operation camlib refuses goes back to capp as an exclusion.
pub mod job;
pub mod simulate;

use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

/// What camlib's comparison of the finished stock with the part found, for the log.
fn material_note(report: Option<&camlib::material::Report>, manifest: &Value) -> String {
    let Some(report) = report else {
        return "the finished stock is not compared with the part in this setup: it holds no mesh of the part that is fine enough".into();
    };
    // The mesh's error widens every cell, flat faces too, and only a cell the tool wholly covers
    // counts as cut: errors smaller than these pass, and they differ from part to part.
    let height = manifest["verification"]["material"][0]["target"]["surface"]["surface_error_mm"].as_f64().unwrap_or(0.);
    let cell = manifest["stock"]["cell_size_mm"].as_f64().unwrap_or(0.);
    format!(
        "finished stock against the part: no definite gouge. The cell comparison can miss an error under about {height:.2} mm in height or {cell:.2} mm sideways, or a misplaced cut narrower than about {:.1} mm. Up to {:.0} mm³ undecided; at least {:.0} mm³ stock remains, counting the web and anything a later setup cuts",
        4. * cell, report.gouge.volume_mm3[1], report.residual.volume_mm3[0],
    )
}

fn simulate_all(plan: &Value, library: &Value, programs: &mut Vec<Value>, log: &mut Vec<String>) -> Result<(), simulate::Failure> {
    let (jobs, notes) = job::jobs(plan, library)?;
    log.extend(notes);
    for job in jobs {
        log.extend(job.notes.iter().map(|n| format!("note: {n}")));
        let run = simulate::simulate(&job.manifest.to_string()).map_err(|f| simulate::Failure {
            message: format!("setup {}: {}", job.setup, f.message),
            ..f
        })?;
        if run.uncertain > 0 {
            log.push(format!("note: setup {}: camlib's stock-contact check could not clear {} motions", job.setup, run.uncertain));
        }
        log.push(format!("note: setup {}: {}", job.setup, material_note(run.material.as_ref(), &job.manifest)));
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
    let planned = capp::plan::plan(analysis, Some(mesh), &capp::plan::Options::default())?;
    let (mut programs, mut log, mut error) = (vec![], vec![], None);
    // What camlib refused, in order: capp plans again without each.
    let mut exclusions: Vec<Value> = vec![];
    let mut plan;
    loop {
        programs.clear();
        log.clear();
        plan = planned.clone();
        if let Err(message) = capp::finish(&mut plan, library, &exclusions) {
            error = Some(message);
            break;
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
        exclusions.push(json!({"operation": operation, "message": failure.message}));
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
