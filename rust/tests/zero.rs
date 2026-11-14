//! Datum pickup uses retained earlier finishing, never a merely recognised feature.
use serde_json::{Value, json};
fn read(p: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(p)).unwrap()).unwrap()
}
#[test]
fn stepped_plate_pickups_are_recorded_for_both_machines() {
    let a=read("tests/fixtures/synthetic/stepped-plate.analysis.json");
    let m=read("tests/fixtures/synthetic/stepped-plate.mesh.json");
    for lib in ["demo/tools.json","libraries/haas-vf2.json"] {
        let out=step2cam::process(&a,&m,&read(lib)).unwrap();
        assert!(out["error"].is_null(),"{lib}: {}",out["error"]);
        let setups=out["plan"]["setups"].as_array().unwrap();
        let top=&setups[0];
        assert_eq!(top["clamping"]["fixed_jaw"],"y.min");
        assert_eq!(top["stock"]["min"][0],0.);
        assert_eq!(top["stock"]["min"][1],0.);
        assert_eq!(top["stock"]["max"][2],0.);
        assert_eq!(top["zero"]["z"]["description"],"stock top");
        let bottom=setups.iter().find(|s|s["id"]=="bottom").unwrap();
        assert_eq!(bottom["zero"]["xy"]["features"],json!(["holes:36"]));
        assert_eq!(bottom["zero"]["xy"]["finished_in"],"top");
        // Top machining cannot finish the underside before the flip. It must
        // explicitly transfer Z from stock rather than invent an earlier finish.
        assert!(bottom["zero"]["z"]["finished_in"].is_null());
        assert!(bottom["zero"]["z"]["description"].as_str().unwrap().contains("touch off stock top at Z+"));
        for s in setups {assert!(s["zero"].is_object());}
        let validator=jsonschema::validator_for(&read("schemas/plan-1.schema.json")).unwrap();
        let errors:Vec<_>=validator.iter_errors(&out["plan"]).map(|e|format!("{e} at {}",e.instance_path())).collect();
        assert!(errors.is_empty(),"{}",errors.join("\n"));
        let typed:capp::model::Plan=serde_json::from_value(out["plan"].clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(),out["plan"]);
    }
}
#[test]
fn absent_finished_features_preserve_the_entire_later_frame() {
    let mut p=json!({"setups":[
        {"id":"top","rotation":[[1,0,0],[0,1,0],[0,0,1]],"origin_model":[0,0,10],"stock":{"min":[0,0,-10],"max":[20,20,0]},"operations":[]},
        {"id":"front","rotation":[[1,0,0],[0,0,1],[0,-1,0]],"origin_model":[1,2,3],"stock":{"min":[0,0,-20],"max":[20,10,0]},"operations":[]}
    ],"decisions":[],"warnings":[],"skipped":[]});
    let old=p["setups"][1].clone();
    let offsets=capp::zero::locate(&mut p,&json!({"features":[],"faces":[]}),&json!({}));
    assert_eq!(offsets[1],[0.;3]);
    assert_eq!(p["setups"][1]["origin_model"],old["origin_model"]);
    assert_eq!(p["setups"][1]["stock"],old["stock"]);
    assert_eq!(p["setups"][1]["zero"]["status"],"warning");
    assert!(p["warnings"][0].as_str().unwrap().contains("no finished feature to pick up X/Y"));
}
#[test]
fn fixed_jaw_side_is_optional_but_validated() {
    let mut lib=read("demo/tools.json");
    let v=jsonschema::validator_for(&read("schemas/resources-1.schema.json")).unwrap();
    for side in ["x.min","x.max","y.min","y.max"] {
        lib["machines"][0]["workholding"][0]["fixed_jaw"]=json!(side);
        assert!(v.is_valid(&lib));
        assert!(capp::resources::normalize(&lib).is_ok());
    }
    lib["machines"][0]["workholding"][0]["fixed_jaw"]=json!("front");
    assert!(!v.is_valid(&lib));
    assert!(capp::resources::normalize(&lib).is_err());
}
