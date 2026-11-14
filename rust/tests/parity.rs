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

/// Apply only the new nominal blank and its frame translation to the frozen Python plan.
/// All original operation geometry, coverage and output fields remain compared.
fn standard_blank(p: &mut Value, spindle: bool) {
    let (bar, length, dx, dz) = if spindle { ([60.,30.],28.,1.,7.) } else { ([12.7,64.],104.,0.,1.7) };
    let top = &mut p["setups"][0];
    top["stock"]["min"][2] = serde_json::json!(-bar[0]);
    if spindle { top["stock"]["max"][0] = serde_json::json!(30.); }
    top["origin_model"][0] = serde_json::json!(top["origin_model"][0].as_f64().unwrap()-dx);
    top["origin_model"][2] = serde_json::json!(top["origin_model"][2].as_f64().unwrap()+dz);
    for op in top["operations"].as_array_mut().unwrap() {
        for key in ["floor_z", "top_z", "bore_top_z", "bottom_z"] {
            if let Some(v) = op.get_mut(key) { *v = serde_json::json!(v.as_f64().unwrap()-dz); }
        }
        // Roughing starts at the new stock top, Z0; its target follows the part.
        if op["kind"] == "roughing" { op["top_z"] = serde_json::json!(0.); }
        if let Some(points) = op.get_mut("positions").and_then(Value::as_array_mut) {
            for point in points.chunks_mut(3) {
                point[0] = serde_json::json!(point[0].as_f64().unwrap()+dx);
                point[2] = serde_json::json!(point[2].as_f64().unwrap()-dz);
            }
        }
        for key in ["centers", "outer"] {
            if let Some(points) = op.get_mut(key).and_then(Value::as_array_mut) {
                for point in points { point[0] = serde_json::json!(point[0].as_f64().unwrap()+dx); }
            }
        }
    }
    for (i, setup) in p["setups"].as_array_mut().unwrap().iter_mut().enumerate() {
        setup["stock_size"] = if i == 0 { serde_json::json!({"kind":"standard",
            "source":capp::rules::standard_sizes()["flat_bar"]["source"], "cross_section_mm":bar,
            "sawn_length_mm":length, "allocation":"extra thickness on top; width and sawn-length extra split equally between sides, keeping all margins as minimums"})
        } else { serde_json::json!({"kind":"remaining", "from_setup":"top"}) };
    }
}

#[test]
fn the_plate_plans_as_in_python() {
    let analysis = json("tests/fixtures/plate.analysis.json");
    let mesh = json("tests/fixtures/plate.mesh.json");
    let plan = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
    let mut expected = json("tests/fixtures/rust-golden/plate.plan.json");
    standard_blank(&mut expected, false);
    same(&untraced(&plan), &expected, "plan").unwrap();
}

#[test]
fn the_spindle_plans_without_a_mesh_as_in_python() {
    let analysis = json("tests/fixtures/spindle.analysis.json");
    let plan = capp::plan::plan(&analysis, None, &Default::default()).unwrap();
    let mut expected = json("tests/fixtures/rust-golden/spindle.plan.json");
    standard_blank(&mut expected, true);
    same(&untraced(&plan), &expected, "plan").unwrap();
}

/// Deliberate changes since Python in the plate jobs: roughing keeps the em6's centre within
/// a radius and 1 mm of the stock (`process.roughing.region`), and the convex plate's wall
/// finish leads on and off a radius along the tangent (`process.wall_finish.lead`).
fn adapter_changes(python: &mut Value) {
    let (lo, hi) = (python["stock"]["min_mm"].clone(), python["stock"]["max_mm"].clone());
    let x = |v: &Value, k: usize, grow: f64| v[k].as_f64().unwrap() + grow;
    let grow = 6. + 1.;
    let rough = &mut python["operations"][1];
    assert_eq!((&rough["kind"], &rough["tool"]), (&serde_json::json!("roughing"), &serde_json::json!("em6")));
    rough["limits"] = serde_json::json!({"contain": {
        "version": 1, "units": "mm", "name": format!("{}-stock", rough["id"].as_str().unwrap()), "boundary_error_mm": 0.0,
        "components": [{"outer": {"vertices": [[x(&lo, 0, -grow), x(&lo, 1, -grow), 0.], [x(&hi, 0, grow), x(&lo, 1, -grow), 0.], [x(&hi, 0, grow), x(&hi, 1, grow), 0.], [x(&lo, 0, -grow), x(&hi, 1, grow), 0.]]}, "holes": []}],
        "source": {"plan": capp::plan::PLAN_FORMAT},
    }});
    let wall = &mut python["operations"][7];
    assert_eq!((&wall["kind"], &wall["tool"]), (&serde_json::json!("contour"), &serde_json::json!("em6")));
    wall["lead_in_mm"] = serde_json::json!(3.0);
    wall["lead_out_mm"] = serde_json::json!(3.0);
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
    adapter_changes(&mut python);
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
    // Roughing kept within a radius and 1 mm of the stock (process.roughing.region) makes 1,334
    // moves instead of 1,565. Facing, twist drills and the bore retain their move counts.
    assert_eq!(&ends[..7], [105, 1438, 1443, 1467, 1472, 1496, 1538]);
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
    let mut expected = json("tests/fixtures/rust-golden/plate-thread.plan.json");
    standard_blank(&mut expected, false);
    same(&untraced(&plan), &expected, "plan").unwrap();
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
    adapter_changes(&mut python);
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
