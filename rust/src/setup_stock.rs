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

pub(crate) fn assumption(reason: &str) -> String {
    format!("Assumes a freed, part-sized block; stock carry unavailable: {reason}")
}

/// A manual instruction before the next setup, attached to the existing setup list.
pub(crate) fn breakout(previous: &Value) -> Option<Value> {
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
    let id = previous["id"].as_str()?;
    Some(json!({
        "kind": "break_out", "after_setup": id,
        "witness": {"setup": id, "bottom_z_mm": floor - thickness, "top_z_mm": floor, "thickness_mm": thickness},
        "says": format!("After {id}, support the part and break it out of the web before reclamping. Clean the web witness around the bottom perimeter, approximately {thickness} mm thick, between Z{} and Z{floor} in the {id} frame. Simulation retains the web and surrounding stock conservatively; manual separation is not simulated.", floor - thickness)
    }))
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
    fn front_starts_with_top_finished_stock_and_records_the_web_witness() {
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
        let program = output["programs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["setup"] == "front")
            .unwrap();
        assert!(program["job"]["stock"]["mesh"].is_object());
        assert!(
            front["stock_note"]
                .as_str()
                .unwrap()
                .contains("carried from top")
        );
        let step = &front["before_setup"];
        assert_eq!(step["kind"], "break_out");
        assert_eq!(step["after_setup"], "top");
        let rough = plan["setups"][0]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["kind"] == "roughing")
            .unwrap();
        assert_eq!(step["witness"]["thickness_mm"], rough["web_mm"]);
        assert_eq!(step["witness"]["top_z_mm"], rough["floor_z"]);
        let measured = step["witness"]["top_z_mm"].as_f64().unwrap()
            - step["witness"]["bottom_z_mm"].as_f64().unwrap();
        assert!((measured - rough["web_mm"].as_f64().unwrap()).abs() < 1e-9);
        assert!(step["says"].as_str().unwrap().contains("bottom perimeter"));
        let manifest: StreamManifest = serde_json::from_value(program["job"].clone()).unwrap();
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
        assert!(breakout(&json!({"flip_role":"sacrificial_grip"})).is_none());
        let web = breakout(&json!({"id":"top", "operations":[{"kind":"roughing", "tool":"em6", "web_mm":0.5, "floor_z":-18.5}]})).unwrap();
        assert_eq!(
            web["witness"],
            json!({"setup":"top", "bottom_z_mm":-19., "top_z_mm":-18.5, "thickness_mm":0.5})
        );
    }
}
