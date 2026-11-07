//! step2cam: a STEP part's recognised features -> camlib toolpaths and G-code, with no Python and
//! no server, so that it can run in a browser.
//!
//! specify-core's analysis and mesh -> [`capp`] (plan, settle, tools) -> [`job::jobs`] (the camlib
//! adapter) -> [`post::post`]. An operation camlib refuses goes back to capp as an exclusion.
pub mod job;
pub mod post;

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
        "finished stock against the part: no definite gouge. It cannot see an error under about {height:.2} mm in height or {cell:.2} mm sideways, or a misplaced cut narrower than about {:.1} mm. Up to {:.0} mm³ undecided; at least {:.0} mm³ stock remains, counting the web and anything a later setup cuts",
        4. * cell, report.gouge.volume_mm3[1], report.residual.volume_mm3[0],
    )
}

fn post_all(plan: &Value, library: &Value, machine: &post::Machine, programs: &mut Vec<Value>, log: &mut Vec<String>) -> Result<(), post::Failure> {
    let (jobs, notes) = job::jobs(plan, library)?;
    log.extend(notes);
    for job in jobs {
        log.extend(job.notes.iter().map(|n| format!("note: {n}")));
        let program = post::post(&job.manifest.to_string(), machine.clone()).map_err(|f| post::Failure {
            message: format!("setup {}: {}", job.setup, f.message),
            ..f
        })?;
        if program.uncertain > 0 {
            log.push(format!("note: setup {}: camlib's stock-contact check could not clear {} motions", job.setup, program.uncertain));
        }
        log.push(format!("note: setup {}: {}", job.setup, material_note(program.material.as_ref(), &job.manifest)));
        // What the check cannot see, as the note says it, for the page to word.
        let check = &job.manifest["verification"]["material"][0];
        let limits = json!({
            "height_mm": check["target"]["surface"]["surface_error_mm"], "sideways_mm": job.manifest["stock"]["cell_size_mm"],
            "before_threads": !check["after_operation"].is_null(),
        });
        let material = program.material.map(|r| json!({"complete": r.complete, "residual": r.residual, "gouge": r.gouge, "limits": limits}));
        programs.push(json!({"setup": job.setup, "nc": program.nc, "operations": program.operations, "motions": program.motions, "material": material}));
    }
    Ok(())
}

/// Everything the page draws, from specify-core's analysis and mesh and a tool library. An
/// operation camlib refuses is left out with a warning, so the rest of the part can still be
/// seen; a part that plans but cannot post at all still comes back, with `post_error`.
pub fn process(analysis: &Value, mesh: &Value, library: &Value) -> Result<Value, String> {
    let planned = capp::plan::plan(analysis, Some(mesh), &capp::plan::Options::default())?;
    let machine: post::Machine = serde_json::from_value(library.get("machine").cloned().unwrap_or(Value::Null)).map_err(|e| format!("tool library machine: {e}"))?;
    let (mut programs, mut log, mut post_error) = (vec![], vec![], None);
    // What camlib refused, in order: capp plans again without each.
    let mut exclusions: Vec<Value> = vec![];
    let mut plan;
    loop {
        programs.clear();
        log.clear();
        plan = planned.clone();
        if let Err(message) = capp::finish(&mut plan, library, &exclusions) {
            post_error = Some(message);
            break;
        }
        let Err(failure) = post_all(&plan, library, &machine, &mut programs, &mut log) else {
            break;
        };
        // An operation capp can take out is excluded and the plan finished again; anything else
        // stops: nothing is given out, not even a setup that posted before this one stopped.
        let excludable = failure.operation.as_deref().is_some_and(|id| capp::exclude(&mut plan.clone(), id, "").is_some());
        let (true, Some(operation)) = (excludable, failure.operation) else {
            post_error = Some(failure.message);
            programs.clear();
            break;
        };
        exclusions.push(json!({"operation": operation, "message": failure.message}));
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
    Ok(json!({"plan": plan, "programs": programs, "log": log, "post_error": post_error}))
}

/// For the browser: JSON strings in, a JSON string out.
#[wasm_bindgen(js_name = process)]
pub fn process_json(analysis: &str, mesh: &str, library: &str) -> Result<String, JsError> {
    let parse = |text: &str| serde_json::from_str::<Value>(text).map_err(|e| JsError::new(&e.to_string()));
    let (analysis, mesh, library) = (parse(analysis)?, parse(mesh)?, parse(library)?);
    let out = process(&analysis, &mesh, &library).map_err(|e| JsError::new(&e))?;
    Ok(out.to_string())
}
