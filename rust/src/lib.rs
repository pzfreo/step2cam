//! step2cam: a STEP part's recognised features -> camlib toolpaths and G-code, with no Python and
//! no server, so that it can run in a browser.
//!
//! specify-core's analysis and mesh -> [`plan::plan`] -> [`job::jobs`] -> [`post::post`].
pub mod job;
pub mod plan;
pub mod post;

use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

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
        programs.push(json!({"setup": job.setup, "nc": program.nc, "operations": program.operations, "motions": program.motions}));
    }
    Ok(())
}

/// Takes an operation named as job::jobs names them (`top-2-roughing`) out of the plan.
fn remove_operation(plan: &mut Value, id: &str) -> bool {
    let mut parts = id.splitn(3, '-');
    let (Some(setup_id), Some(number)) = (parts.next(), parts.next().and_then(|n| n.parse::<usize>().ok())) else {
        return false;
    };
    let setups = plan["setups"].as_array_mut();
    let Some(setup) = setups.into_iter().flatten().find(|s| s["id"] == setup_id) else {
        return false;
    };
    match setup["operations"].as_array_mut() {
        Some(ops) if number >= 1 && number <= ops.len() => {
            ops.remove(number - 1);
            true
        }
        _ => false,
    }
}

/// Everything the page draws, from specify-core's analysis and mesh and a tool library. An
/// operation camlib refuses is left out with a warning, so the rest of the part can still be
/// seen; a part that plans but cannot post at all still comes back, with `post_error`.
pub fn process(analysis: &Value, mesh: &Value, library: &Value) -> Result<Value, String> {
    let mut plan = plan::plan(analysis, Some(mesh), &plan::Options::default())?;
    let machine: post::Machine = serde_json::from_value(library.get("machine").cloned().unwrap_or(Value::Null)).map_err(|e| format!("tool library machine: {e}"))?;
    let (mut programs, mut log, mut post_error) = (vec![], vec![], None);
    loop {
        programs.clear();
        log.clear();
        let Err(failure) = post_all(&plan, library, &machine, &mut programs, &mut log) else {
            break;
        };
        let Some(operation) = failure.operation.filter(|id| remove_operation(&mut plan, id)) else {
            post_error = Some(failure.message);
            break;
        };
        let brief: String = failure.message.chars().take(160).collect();
        if let Some(warnings) = plan["warnings"].as_array_mut() {
            warnings.push(json!(format!("{operation} left out: {brief}")));
        }
    }
    // The page has the mesh already; the plan's copy of it is not needed.
    for setup in plan["setups"].as_array_mut().into_iter().flatten() {
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
