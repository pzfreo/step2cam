//! Plans and jobs against tests/fixtures/rust-golden: a frozen baseline, written by the Python
//! version this project started as.
use serde_json::Value;
use std::{fs, path::PathBuf};

/// The plan as it is executed: settled, and each operation's tool named.
fn finished(plan: &Value, library: &Value) -> Value {
    let mut plan = plan.clone();
    capp::finish(&mut plan, library, &[]).unwrap();
    plan
}

/// The plan without its decision trace (`decisions`, each operation's `because`, each skipped
/// feature's `decision`): what the frozen baseline has, which predates the trace.
fn untraced(plan: &Value) -> Value {
    let mut plan = plan.clone();
    if let Some(o) = plan.as_object_mut() {
        // Also newer than the baseline: warnings of drawing requirements not acted on.
        let unacted: Vec<Value> = o["decisions"].as_array().unwrap().iter().filter(|d| d["rule"] == "requirement.not_acted_on@1").map(|d| d["says"].clone()).collect();
        o["warnings"].as_array_mut().unwrap().retain(|w| !unacted.contains(w));
        o.remove("decisions");
        // Also newer than the baseline: the material Specify was told.
        o.remove("material");
        o.remove("rulebase");
        // Settlement's feature-depth metadata did not exist in the Python baseline.
        assert!(o["roughing_feature_depths"].is_object());
        o.remove("roughing_feature_depths");
        // New internal workholding meshes have no Python baseline. Compare
        // all original plan fields; dedicated contact tests exercise these.
        for key in ["contact_surface_model", "workholding_surface_model"] {
            let geometry = o.remove(key).expect("workholding geometry metadata");
            assert!(geometry.is_array() || geometry.is_null());
        }
    }
    for setup in plan["setups"].as_array_mut().into_iter().flatten() {
        for op in setup["operations"].as_array_mut().into_iter().flatten() {
            op.as_object_mut().map(|o| o.remove("because"));
        }
    }
    for skipped in plan["skipped"].as_array_mut().into_iter().flatten() {
        skipped.as_object_mut().map(|o| o.remove("decision"));
    }
    plan
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}
fn json(path: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(root().join(path)).unwrap()).unwrap()
}
/// Equal, with numbers compared by value (1 and 1.0) to within rounding of the last places.
fn same(a: &Value, b: &Value, path: &str) -> Result<(), String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if x == y || (x - y).abs() <= 1e-12 * x.abs().max(y.abs()) {
                Ok(())
            } else {
                Err(format!("{path}: {x} != {y}"))
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Err(format!("{path}: {} items != {}", x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().try_for_each(|(i, (p, q))| same(p, q, &format!("{path}[{i}]")))
        }
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<_> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter().try_for_each(|k| match (x.get(k), y.get(k)) {
                (Some(p), Some(q)) => same(p, q, &format!("{path}.{k}")),
                _ => Err(format!("{path}.{k}: only on one side")),
            })
        }
        _ if a == b => Ok(()),
        _ => Err(format!("{path}: {a} != {b}")),
    }
}

#[test]
fn the_plate_plans_as_in_python() {
    let analysis = json("tests/fixtures/plate.analysis.json");
    let mesh = json("tests/fixtures/plate.mesh.json");
    let plan = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    same(&untraced(&plan), &json("tests/fixtures/rust-golden/plate.plan.json"), "plan").unwrap();
}

#[test]
fn the_spindle_plans_without_a_mesh_as_in_python() {
    let analysis = json("tests/fixtures/spindle.analysis.json");
    let plan = capp::plan::plan(&analysis, None, &Default::default()).unwrap();
    same(&untraced(&plan), &json("tests/fixtures/rust-golden/spindle.plan.json"), "plan").unwrap();
}

#[test]
fn the_plates_job_is_python_s_job() {
    let plan = json("tests/fixtures/rust-golden/plate.plan.json");
    let library = json("examples/tools.example.json");
    let (jobs, _) = step2cam::job::jobs(&finished(&plan, &library), &library).unwrap();
    assert_eq!(jobs.len(), 1);
    let mut mine = jobs[0].manifest.clone();
    let mut python = json("tests/fixtures/rust-golden/plate.manifest.json");
    // The run's random id and the job's hash.
    mine["run"] = Value::Null;
    python["run"] = Value::Null;
    // Added since: the comparison of the finished stock with the part (rust/tests/material.rs).
    mine["verification"]["material"] = python["verification"]["material"].clone();
    // Deliberate change from Python: both plate drills are below 3D and now make
    // one plunge. Change only the expected pecks; compare every other manifest field.
    python["operations"][4]["peck_mm"] = Value::Null;
    python["operations"][5]["peck_mm"] = Value::Null;
    // Deliberate change: wall finishing takes its full 10.5 mm depth in one
    // flute-supported pass, independently of the 1.5 mm roughing stepdown.
    assert_eq!(python["operations"][7]["kind"], "contour");
    python["operations"][7]["stepdown_mm"] = serde_json::json!(10.5);
    same(&mine, &python, "manifest").unwrap();
}

#[test]
fn the_plates_job_runs_in_camlib() {
    let analysis = json("tests/fixtures/plate.analysis.json");
    let mesh = json("tests/fixtures/plate.mesh.json");
    let out = step2cam::process(&analysis, &mesh, &json("examples/tools.example.json")).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let ends: Vec<u64> = out["programs"][0]["operations"].as_array().unwrap().iter().map(|o| o["end"].as_u64().unwrap()).collect();
    // camlib #104 simplifies the roughing layer shadows: 1,565 moves instead of 1,922.
    // The shallow drills now make one plunge: 5 and 24 moves instead of 17 and 78.
    // Two spot cycles add 5 and 24 moves, sharing one tool.
    // Facing, roughing, twist drills and the bore retain their move counts.
    assert_eq!(&ends[..7], [53, 1618, 1623, 1647, 1652, 1676, 1714]);
    assert_eq!(ends.len(), 8); // two spots plus the wall finish
    assert_eq!(out["programs"][0]["moves"].as_array().unwrap().len() as u64, ends[7]);
    assert_eq!(out["programs"][0]["material"]["complete"], true);
    assert_eq!(out["programs"][0]["material"]["gouge"]["definite_cells"], 0);
}

/// The plate with its ⌀4.2 hole answered as tapped M5.
fn tapped_plate() -> Value {
    let mut analysis = json("tests/fixtures/plate.analysis.json");
    analysis["resume"] = serde_json::json!({"answers": {"hole.function:holes:12": "tapped:M5"}});
    analysis
}

#[test]
fn a_tapped_hole_plans_as_in_python() {
    let plan = capp::plan::plan(&tapped_plate(), Some(&json("tests/fixtures/plate.mesh.json")), &Default::default()).unwrap();
    same(&untraced(&plan), &json("tests/fixtures/rust-golden/plate-thread.plan.json"), "plan").unwrap();
}

#[test]
fn a_tapped_holes_job_is_python_s_job() {
    let plan = json("tests/fixtures/rust-golden/plate-thread.plan.json");
    let (jobs, _) = step2cam::job::jobs(&finished(&plan, &json("examples/tools.example.json")), &json("examples/tools.example.json")).unwrap();
    let mut mine = jobs[0].manifest.clone();
    let mut python = json("tests/fixtures/rust-golden/plate-thread.manifest.json");
    mine["run"] = Value::Null;
    python["run"] = Value::Null;
    // Added since: the comparison of the finished stock with the part (rust/tests/material.rs).
    mine["verification"]["material"] = python["verification"]["material"].clone();
    // Deliberate change from Python: both plate drills are below 3D and now make
    // one plunge. Change only the expected pecks; compare every other manifest field.
    python["operations"][4]["peck_mm"] = Value::Null;
    python["operations"][5]["peck_mm"] = Value::Null;
    // The same deliberate full-depth wall-finishing change as the plain plate.
    assert_eq!(python["operations"][7]["kind"], "contour");
    python["operations"][7]["stepdown_mm"] = serde_json::json!(10.5);
    same(&mine, &python, "manifest").unwrap();
}

/// The gauge line is fixed, while tools of different lengths start at different tip heights.
/// Check actual simulated starts, including a return to the milling tool after drilling.
#[test]
fn each_tool_starts_at_the_declared_machine_pose() {
    let mut library = json("examples/tools.example.json");
    library["tool_change_xy_mm"] = serde_json::json!([-7., -9.]);
    library["tool_change_gauge_z_mm"] = 130.into();
    let out = step2cam::process(&json("tests/fixtures/plate.analysis.json"), &json("tests/fixtures/plate.mesh.json"), &library).unwrap();
    assert!(out["error"].is_null(), "{}", out["error"]);
    let program = &out["programs"][0];
    let job = &program["job"];
    let mut active = None;
    let mut changes = 0;
    for range in program["operations"].as_array().unwrap() {
        let first = &program["moves"][range["start"].as_u64().unwrap() as usize];
        let tool = first["tool"].as_str().unwrap();
        if active != Some(tool) {
            let assembly = job["tools"].as_array().unwrap().iter().find(|t| t["id"] == tool).unwrap();
            let tip_z = 130. - assembly["gauge_length_mm"].as_f64().unwrap();
            assert_eq!(first["points"][0], serde_json::json!([-7., -9., tip_z]));
            active = Some(tool);
            changes += 1;
        }
    }
    assert_eq!(changes, 5); // mill, spot drill, two twist drills, mill again
    assert_eq!(program["material"]["complete"], true);
    assert_eq!(program["material"]["gouge"]["definite_cells"], 0);
}
