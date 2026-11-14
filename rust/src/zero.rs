//! Change the coordinate description only after the checked workholding run is selected.
//! Keeping execution in its original frame avoids re-tessellation/grid changes from a
//! coordinate offset. Saved jobs, drawn motions and plan geometry use the same new frame.
use serde_json::{Value, json};

fn number(v: &mut Value, d: f64) {
    if let Some(n)=v.as_f64() { *v=json!(n-d); }
}
fn point(v: &mut Value, d: [f64;3], dimensions: usize) {
    if let Some(p)=v.as_array_mut() {
        for (i,x) in p.iter_mut().take(dimensions).enumerate() {number(x,d[i]);}
    }
}
fn points(v: &mut Value, d: [f64;3], dimensions: usize) {
    if let Some(ps)=v.as_array_mut() {for p in ps {point(p,d,dimensions);}}
}
// Only coordinate fields, never distances, vectors, cutter geometry or model coordinates.
fn geometry(v: &mut Value, d: [f64;3]) {
    match v {
        Value::Object(map) => {
            let surface=map.contains_key("surface_error_mm") || map.contains_key("indices");
            for (k,v) in map {
                match k.as_str() {
                    "before_setup" | "tools" | "technology" | "zero" | "stock_size" | "flip_seating" | "contacts" | "force_check" | "because" => (),
                    "min" | "max" | "min_mm" | "max_mm" | "initial_position_mm" => point(v,d,3),
                    "a" | "b" | "center" | "start_mm" | "end_mm" | "xy_mm" => point(v,d,2),
                    "centers" | "centers_mm" | "outer" => points(v,d,2),
                    "vertices" => points(v,d,if surface {3} else {2}),
                    "triangles" => {if let Some(ts)=v.as_array_mut() {for t in ts {points(t,d,3);}}},
                    "positions" => {if let Some(ps)=v.as_array_mut() {for (i,p) in ps.iter_mut().enumerate() {number(p,d[i%3]);}}},
                    "levels" => {if let Some(zs)=v.as_array_mut() {for z in zs {number(z,d[2]);}}},
                    "floor_z" | "top_z" | "bottom_z" | "bore_top_z" | "root_z" | "wall_top" | "jaw_top_z" | "contact_min_z" |
                    "floor_z_mm" | "top_z_mm" | "bottom_z_mm" | "depth_z_mm" | "clearance_z_mm" | "retract_z_mm" | "feed_z_mm" |
                    "base_z_mm" | "gauge_z_mm" | "z_min_mm" | "z_max_mm" => number(v,d[2]),
                    _ => geometry(v,d),
                }
            }
        }
        Value::Array(vs) => {for v in vs {geometry(v,d);}},
        _ => (),
    }
}

pub fn apply(out: &mut Value, analysis: &Value, mesh: &Value) {
    let offsets=capp::zero::locate(&mut out["plan"],analysis,mesh);
    for (i,d) in offsets.into_iter().enumerate() {
        let s=&mut out["plan"]["setups"][i];
        let id=s["id"].clone();
        let old: [f64;3]=std::array::from_fn(|j|s["origin_model"][j].as_f64().unwrap());
        let origin: [f64;3]=std::array::from_fn(|j|old[j]+(0..3).map(|k|s["rotation"][k][j].as_f64().unwrap()*d[k]).sum::<f64>());
        geometry(s,d);
        s["origin_model"]=json!(origin);
        // Along-contact coordinates are in the transverse setup axis; model-labelled
        // contacts and flip seating stay in model coordinates.
        let axis=s["clamping"]["fixed_jaw"].as_str().map(|j|usize::from(!j.starts_with('y')));
        if let Some(axis)=axis {for key in ["contact_along_min","contact_along_max"] {if let Some(v)=s.get_mut("clamping").and_then(|c|c.get_mut(key)) {number(v,d[axis]);}}}
        for p in out["programs"].as_array_mut().into_iter().flatten().filter(|p|p["setup"]==id) {
            geometry(&mut p["job"],d);
            for m in p["moves"].as_array_mut().into_iter().flatten() {points(&mut m["points"],d,3);}
            if let Some(findings)=p.get_mut("material").and_then(|m|m.get_mut("findings")).and_then(Value::as_array_mut) {
                for f in findings {if let Some(xy)=f.get_mut("xy_mm") {point(xy,d,2);}}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relocation_translates_every_motion_and_preserves_its_metadata() {
        let a=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.analysis.json")).unwrap();
        let m=serde_json::from_str(include_str!("../../tests/fixtures/synthetic/stepped-plate.mesh.json")).unwrap();
        let lib=serde_json::from_str(include_str!("../../libraries/haas-vf2.json")).unwrap();
        let original=crate::workholding::process(&a,&m,&lib).unwrap();
        let mut moved=original.clone();
        apply(&mut moved,&a,&m);
        assert_eq!(original["plan"]["skipped"],moved["plan"]["skipped"]);
        assert_eq!(original["error"],moved["error"]);
        assert_eq!(original["programs"].as_array().unwrap().len(),moved["programs"].as_array().unwrap().len());
        for (old,new) in original["programs"].as_array().unwrap().iter().zip(moved["programs"].as_array().unwrap()) {
            let setup=moved["plan"]["setups"].as_array().unwrap().iter().find(|s|s["id"]==new["setup"]).unwrap();
            let d:[f64;3]=std::array::from_fn(|i|setup["zero"]["offset_in_previous_frame_mm"][i].as_f64().unwrap());
            assert_eq!(old["operations"],new["operations"]);
            assert_eq!(old["moves"].as_array().unwrap().len(),new["moves"].as_array().unwrap().len());
            for (a,b) in old["moves"].as_array().unwrap().iter().zip(new["moves"].as_array().unwrap()) {
                let mut translated=a.clone();
                points(&mut translated["points"],d,3);
                assert_eq!(translated,*b,"every point shifts by the same vector, all other motion fields stay identical");
            }
            let manifest:camlib::stream::StreamManifest=serde_json::from_value(new["job"].clone()).unwrap();
            manifest.validate().unwrap();
        }
    }
}
