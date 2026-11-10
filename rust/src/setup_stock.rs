//! Sequential stock handoff. Geometry and acceptance belong to camlib.
use camlib::stock_mesh::FinishedStock;
use serde_json::{Value, json};

/// Move the previous job's finished stock using the plan's recorded frames.
pub(crate) fn carry(
    stock: &FinishedStock,
    from: &Value,
    to: &Value,
    manifest: &Value,
) -> Result<(Value, f64), String> {
    let (rotation, translation) = capp::plan::between_setups(from, to)?;
    let vertices = stock
        .mesh
        .vertices()
        .iter()
        .map(|p| {
            std::array::from_fn(|i| {
                translation[i] + (0..3).map(|j| rotation[i][j] * p[j]).sum::<f64>()
            })
        })
        .collect();
    let mesh = camlib::stock_mesh::StockMesh::new(vertices, stock.mesh.indices().to_vec())?;
    let [lo, hi] = mesh.bounds();
    let mut out = manifest.clone();
    // Keep the receiving job's resolution and any declared through-tap backing.
    // The box encloses both the real mesh and the operation limits; it is not material.
    for k in 0..3 {
        let old_lo = out["stock"]["min_mm"][k]
            .as_f64()
            .ok_or("missing stock minimum")?;
        let old_hi = out["stock"]["max_mm"][k]
            .as_f64()
            .ok_or("missing stock maximum")?;
        out["stock"]["min_mm"][k] = json!(old_lo.min(lo[k]));
        out["stock"]["max_mm"][k] = json!(old_hi.max(hi[k]));
    }
    out["stock"]["mesh"] = serde_json::to_value(mesh).map_err(|e| e.to_string())?;
    // Mesh and declared whole-cell premises are mutually exclusive in camlib.
    out.as_object_mut().unwrap().remove("initial_stock");
    Ok((out, lo[2]))
}

/// Reuse the adapter's existing plane, tool-change and grid calculations on
/// the carried envelope. Cutting coordinates and the recorded frame stay fixed.
pub(crate) fn adapt(
    mut carried: Value,
    bottom: f64,
    frame: &Value,
    part: &Value,
    library: &Value,
) -> Result<crate::job::Job, String> {
    let mut receiving = frame.clone();
    receiving["stock"] =
        json!({"min": carried["stock"]["min_mm"], "max": carried["stock"]["max_mm"]});
    receiving["stock"]["min"][2] = json!(bottom);
    if receiving["flip_seating"].is_object() {
        // The carried box already contains the residual margin. The old box
        // needed this extra overlap because it covered only the part.
        receiving["flip_seating"]["residual_margin_mm"] = json!(0.);
    }
    let plan = json!({"format": capp::plan::PLAN_FORMAT, "part": part, "setups": [receiving]});
    let (mut jobs, _) = crate::job::jobs(&plan, library)?;
    let mut job = jobs
        .pop()
        .ok_or("no executable job for the carried stock")?;
    job.manifest["stock"]["mesh"] = carried["stock"]
        .as_object_mut()
        .unwrap()
        .remove("mesh")
        .ok_or("missing carried mesh")?;
    Ok(job)
}

pub(crate) fn carried_note(id: &str, cropped: bool, box_run: bool) -> String {
    format!(
        "Starting stock carried from {id}{}{}. It is camlib's approximate height-field stock: undercuts and voids are filled, so it may hold more material than is really there, never less.",
        if box_run { "’s box run" } else { "" },
        if cropped {
            "; the part is free, so stock outside its footprint is dropped (keeping one cell around it)"
        } else {
            ""
        },
    )
}

pub(crate) fn assumption(reason: &str) -> String {
    format!("Assumes a freed, part-sized block; stock carry unavailable: {reason}")
}

/// A manual instruction before the next setup, attached to the existing setup list.
pub(crate) fn part_bounds(part: &Value, frame: &Value) -> Result<[[f64; 3]; 2], String> {
    let model = json!({"rotation":[[1,0,0],[0,1,0],[0,0,1]], "origin_model":[0,0,0]});
    let (rotation, translation) = capp::plan::between_setups(&model, frame)?;
    let coordinate = |side: &str, i: usize| {
        part[side][i]
            .as_f64()
            .ok_or_else(|| format!("missing part {side} coordinate {i}"))
    };
    let lo = [
        coordinate("min", 0)?,
        coordinate("min", 1)?,
        coordinate("min", 2)?,
    ];
    let hi = [
        coordinate("max", 0)?,
        coordinate("max", 1)?,
        coordinate("max", 2)?,
    ];
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for bits in 0..8 {
        let p: [f64; 3] = std::array::from_fn(|i| if bits & (1 << i) == 0 { lo[i] } else { hi[i] });
        for i in 0..3 {
            let v = translation[i] + (0..3).map(|j| rotation[i][j] * p[j]).sum::<f64>();
            bounds[0][i] = bounds[0][i].min(v);
            bounds[1][i] = bounds[1][i].max(v);
        }
    }
    Ok(bounds)
}

/// Declared manual separation removes the frame, not material inside the part.
/// Retain only whole source cells within the footprint expanded by one cell.
/// Camlib supplies the cells and rebuilds their closed surface.
pub(crate) fn crop_cells(
    snapshot: &mut camlib::stock::StockSnapshot,
    bounds: [[f64; 3]; 2],
) -> Result<(), String> {
    let grid = camlib::stock::Stock::from_snapshot(snapshot.clone())?;
    let [nx, ny] = grid.dimensions();
    let step = grid.cell_widths_mm();
    drop(grid);
    for j in 0..ny {
        for i in 0..nx {
            let outside = [i, j].into_iter().enumerate().any(|(axis, index)| {
                let lo = snapshot.spec.min_mm[axis] + index as f64 * step[axis];
                let hi = if index + 1 == [nx, ny][axis] {
                    snapshot.spec.max_mm[axis]
                } else {
                    lo + step[axis]
                };
                lo < bounds[0][axis] - step[axis] || hi > bounds[1][axis] + step[axis]
            });
            if outside {
                snapshot.cells[j * nx + i] = camlib::stock::HeightBounds {
                    lower_mm: snapshot.spec.min_mm[2],
                    upper_mm: snapshot.spec.min_mm[2],
                };
            }
        }
    }
    Ok(())
}

pub(crate) fn breakout(previous: &Value, part: &Value) -> Option<Value> {
    if previous.get("flip_role").is_some() {
        return None;
    }
    let rough = previous["operations"]
        .as_array()?
        .iter()
        .find(|o| o["kind"] == "roughing" && o["tool"].is_string())?;
    let thickness = rough["web_mm"].as_f64()?;
    if thickness <= 0. {
        return None;
    }
    let floor = rough["floor_z"].as_f64()?;
    let bottom = part_bounds(part, previous).ok()?[0][2];
    let id = previous["id"].as_str()?;
    if floor > bottom + 1e-6 {
        Some(json!({"kind":"cut_free", "after_setup":id,
            "remaining_band":{"setup":id, "bottom_z_mm":bottom, "top_z_mm":floor, "thickness_mm":capp::plan::r(floor-bottom)},
            "says":format!("After {id}, the part is not free: its walls are uncut below Z{floor} in the {id} frame. Support it and cut it free (for example, saw it free) before reclamping. Finish the remaining band from Z{bottom} to Z{floor}, about {} mm, in a later setup.", capp::plan::r(floor-bottom))}))
    } else {
        Some(json!({"kind":"break_out", "after_setup":id,
            "witness":{"setup":id,"bottom_z_mm":floor-thickness,"top_z_mm":floor,"thickness_mm":thickness},
            "says":format!("After {id}, support the part and break the part out of the web before reclamping; clean the witness, about {thickness} mm, around the bottom perimeter between Z{} and Z{floor} in the {id} frame.",floor-thickness)}))
    }
}

pub(crate) fn released(frame: &Value, part: &Value) -> bool {
    (frame["flip_seating"].is_object()
        && frame["operations"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|o| o["kind"] == "facing" && o["tool"].is_string()))
        || breakout(frame, part).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use camlib::{
        stock::{Stock, StockSpec},
        stream::StreamManifest,
    };

    fn fixture(path: &str) -> Value {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        serde_json::from_str(&std::fs::read_to_string(root.join(path)).unwrap()).unwrap()
    }

    #[test]
    fn front_starts_with_released_top_stock_and_records_the_uncut_band() {
        let analysis = fixture("tests/fixtures/synthetic/stepped-plate.analysis.json");
        let mesh = fixture("tests/fixtures/synthetic/stepped-plate.mesh.json");
        let mut library = fixture("tests/fixtures/placeholder-tools.json");
        // Coarse stock is sufficient for a structural handoff test, and avoids
        // duplicating the full-resolution fixture/benchmark checks.
        library["cell_size_mm"] = json!(1.);
        library["decisions"] = json!([{"rule":"setup.workholding", "subject":"part", "choice":"vise_ordinary", "source":"user", "reason":"test the web route"}]);
        let output = crate::process(&analysis, &mesh, &library).unwrap();
        assert!(output["error"].is_null(), "{}", output["error"]);
        let plan = &output["plan"];
        let front = plan["setups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "front")
            .unwrap();
        let top = output["programs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["setup"] == "top")
            .unwrap();
        // Replay the accepted top manifest once in this test to obtain camlib's
        // returned stock, and compare the actual receiving mesh structurally.
        let mut job = crate::job::Job {
            setup: "top".into(),
            return_stock: true,
            crop_stock_to: Some(part_bounds(&plan["part"], &plan["setups"][0]).unwrap()),
            manifest: top["job"].clone(),
            stock_bottom_mm: plan["setups"][0]["stock"]["min"][2].as_f64().unwrap(),
            notes: vec![],
            local_pocket_tops: Default::default(),
        };
        let finished = crate::simulate::simulate_prepared_job(&mut job)
            .unwrap()
            .finished_stock
            .unwrap()
            .unwrap();
        let program = output["programs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["setup"] == "front")
            .expect("spot drilling should run above the carried stock");
        assert!(
            front["stock_note"]
                .as_str()
                .unwrap()
                .contains("carried from top")
        );
        let (expected, _) = carry(&finished, &plan["setups"][0], front, &program["job"]).unwrap();
        let manifest = program["job"].clone();
        assert!(manifest["stock"]["mesh"].is_object());
        assert_eq!(manifest["stock"]["mesh"], expected["stock"]["mesh"]);
        let top = manifest["stock"]["max_mm"][2].as_f64().unwrap();
        for op in manifest["operations"].as_array().unwrap() {
            assert!(op["clearance_z_mm"].as_f64().unwrap() > top);
            if let Some(retract) = op["retract_z_mm"].as_f64() {
                assert!(retract > top);
            }
        }

        let original = capp::plan::plan(&analysis, Some(&mesh), &Default::default()).unwrap();
        let stock_decision = |p: &Value| {
            p["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| {
                    d["subject"] == "front"
                        && d["rule"]
                            .as_str()
                            .unwrap()
                            .starts_with("setup.stock.part_box_with_margins@")
                })
                .unwrap()
                .clone()
        };
        assert_eq!(
            stock_decision(plan)["says"],
            stock_decision(&original)["says"]
        );
        assert_eq!(
            stock_decision(plan)["chosen"],
            stock_decision(&original)["chosen"]
        );
        let step = &front["before_setup"];
        assert_eq!(step["kind"], "cut_free");
        assert_eq!(step["after_setup"], "top");
        let rough = plan["setups"][0]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["kind"] == "roughing")
            .unwrap();
        assert_eq!(step["remaining_band"]["thickness_mm"], rough["web_mm"]);
        assert_eq!(step["remaining_band"]["top_z_mm"], rough["floor_z"]);
        let measured = step["remaining_band"]["top_z_mm"].as_f64().unwrap()
            - step["remaining_band"]["bottom_z_mm"].as_f64().unwrap();
        assert!((measured - rough["web_mm"].as_f64().unwrap()).abs() < 1e-9);
        assert!(
            step["says"]
                .as_str()
                .unwrap()
                .contains("walls are uncut below Z")
        );
        assert!(!step["says"].as_str().unwrap().contains("web"));
        let manifest: StreamManifest = serde_json::from_value(manifest).unwrap();
        let carried = manifest.stock.initial_stock(&|| false).unwrap();
        let carried = carried.height().unwrap();
        // Material removed in top must remain absent in front. Ask camlib for
        // the stock bounds, rather than implementing a point-in-mesh test.
        let spec = carried.spec();
        assert!(
            carried
                .cells()
                .iter()
                .any(|c| c.upper_mm < spec.max_mm[2] - 1.)
        );
        assert!(
            jsonschema::validator_for(&fixture("schemas/plan-1.schema.json"))
                .unwrap()
                .is_valid(plan)
        );
    }

    #[test]
    fn carry_uses_recorded_frame_and_fallback_says_the_assumption() {
        let stock = Stock::new(StockSpec {
            version: 1,
            units: "mm".into(),
            min_mm: [0., 0., -3.],
            max_mm: [4., 2., 0.],
            cell_size_mm: 1.,
            max_cells: 100,
        })
        .unwrap();
        let finished = stock.finished_mesh(&|| false).unwrap();
        let from = json!({"rotation":[[1,0,0],[0,1,0],[0,0,1]], "origin_model":[-2,-2,5]});
        let to = json!({"rotation":[[1,0,0],[0,0,1],[0,-1,0]], "origin_model":[0,0,0]});
        let (r, t) = capp::plan::between_setups(&from, &to).unwrap();
        assert_eq!(r, [[1., 0., 0.], [0., 0., 1.], [0., -1., 0.]]);
        assert_eq!(t, [-2., 5., 2.]);
        assert!(!finished.mesh.vertices().is_empty());
        let text = assumption("camlib refused an open mesh");
        assert!(text.contains("freed, part-sized block"));
        assert!(text.contains("camlib refused an open mesh"));
        assert!(breakout(&json!({"flip_role":"sacrificial_grip"}), &Value::Null).is_none());
        let frame = json!({"id":"top", "rotation":[[1,0,0],[0,1,0],[0,0,1]],"origin_model":[0,0,19], "operations":[{"kind":"roughing", "tool":"em6", "web_mm":0.5, "floor_z":-19.} ]});
        let web = breakout(&frame, &json!({"min":[0,0,0],"max":[4,2,18]})).unwrap();
        assert_eq!(web["kind"], "break_out");
        assert!(
            web["says"]
                .as_str()
                .unwrap()
                .contains("clean the witness, about 0.5 mm")
        );
        assert_eq!(
            web["witness"],
            json!({"setup":"top", "bottom_z_mm":-19.5, "top_z_mm":-19., "thickness_mm":0.5})
        );
    }
    #[test]
    fn release_crops_source_cells_and_camlib_rebuilds_the_closed_mesh() {
        let stock = Stock::new(StockSpec {
            version: 1,
            units: "mm".into(),
            min_mm: [0., 0., -2.],
            max_mm: [6., 6., 0.],
            cell_size_mm: 1.,
            max_cells: 100,
        })
        .unwrap();
        let mut snapshot = stock.snapshot();
        crop_cells(&mut snapshot, [[2., 2., -2.], [4., 4., 0.]]).unwrap();
        for j in 0..6 {
            for i in 0..6 {
                let expected = if (1..5).contains(&i) && (1..5).contains(&j) {
                    0.
                } else {
                    -2.
                };
                assert_eq!(snapshot.cells[j * 6 + i].upper_mm, expected);
                assert_eq!(snapshot.cells[j * 6 + i].lower_mm, expected);
            }
        }
        let mesh =
            camlib::stock_mesh::height_field_mesh(&snapshot.spec, &snapshot.cells, &|| false)
                .unwrap();
        assert_eq!(mesh.bounds(), [[1., 1., -2.], [5., 5., 0.]]);
        assert_eq!(mesh.volume_mm3(), 32.);
        let reread = camlib::stock_mesh::MeshStockSpec::new(snapshot.spec, mesh)
            .unwrap()
            .initial_stock(&|| false)
            .unwrap();
        assert_eq!(reread.cells(), snapshot.cells);
        assert!(!released(
            &json!({"flip_role":"sacrificial_grip"}),
            &Value::Null
        ));
        assert!(carried_note("bottom", true, true).contains("bottom’s box run; the part is free"));
    }

    #[test]
    fn unrelated_validation_is_not_initial_stock_and_unrun_setups_have_no_assumption() {
        let a = fixture("tests/fixtures/synthetic/stepped-plate.analysis.json");
        let m = fixture("tests/fixtures/synthetic/stepped-plate.mesh.json");
        let mut library = fixture("tests/fixtures/placeholder-tools.json");
        let mut plan = capp::plan::plan(&a, Some(&m), &Default::default()).unwrap();
        capp::finish(&mut plan, &library, &[]).unwrap();
        let (jobs, _) = crate::job::jobs(&plan, &library).unwrap();
        let mut invalid = jobs[0].manifest.clone();
        invalid["version"] = json!(999);
        let failure = crate::simulate::simulate(&invalid.to_string())
            .err()
            .unwrap();
        assert!(
            !failure.message.starts_with("initial stock:"),
            "{}",
            failure.message
        );
        plan["setups"][1]["stock_note"] = json!("stale assumption");
        library["cell_size_mm"] = json!(-1.);
        let mut programs = vec![];
        let mut log = vec![];
        let mut cache = vec![];
        assert!(
            crate::simulate_all(&mut plan, &library, &mut programs, &mut log, &mut cache).is_err()
        );
        assert!(
            plan["setups"]
                .as_array()
                .unwrap()
                .iter()
                .skip(1)
                .all(|s| s["stock_note"].is_null())
        );
    }
    #[test]
    fn successor_names_the_box_run_when_a_previous_handoff_failed() {
        let library = fixture("tests/fixtures/placeholder-tools.json");
        let setup = |id: &str, rotation: Value, width: f64| json!({"id":id,"zero":"part", "rotation":rotation,"origin_model":[0,0,0],"stock":{"min":[0,0,-2],"max":[width,width,0]},"operations":[{"kind":"facing","tool":"em6","floor_z":-0.5}]});
        let mut plan = json!({"format":capp::plan::PLAN_FORMAT,"part":{"min":[0,0,-2],"max":[10,10,0]},"decisions":[],"setups":[
            setup("top",json!([[1,0,0],[0,1,0],[0,0,1]]),10.),
            setup("front",json!([[1,0,0],[0,0,1],[0,-1,0]]),4.),
            setup("back",json!([[1,0,0],[0,0,-1],[0,1,0]]),4.)]});
        let mut bounded = library.clone();
        bounded["cell_size_mm"] = json!(1.);
        let mut programs = vec![];
        let mut log = vec![];
        let mut cache = vec![];
        // The large first export exceeds camlib's budget; the smaller box run
        // exports successfully and supplies the third setup's real input.
        camlib::resources::with_memory_budget(50_000, || {
            crate::simulate_all(&mut plan, &bounded, &mut programs, &mut log, &mut cache)
        })
        .unwrap();
        assert!(
            plan["setups"][1]["stock_note"]
                .as_str()
                .unwrap()
                .starts_with("Assumes a freed, part-sized block")
        );
        assert!(programs[1]["job"]["stock"]["mesh"].is_null());
        assert!(programs[2]["job"]["stock"]["mesh"].is_object());
        assert!(
            plan["setups"][2]["stock_note"]
                .as_str()
                .unwrap()
                .contains("carried from front’s box run")
        );
    }
}
