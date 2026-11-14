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
        assert!(out["plan"]["warnings"].as_array().unwrap().iter().any(|w|w.as_str().unwrap().contains("holes:36 is finished in setup top but carried stock caps")));
        assert_eq!(bottom["zero"]["xy"]["features"],json!(["faces:0","faces:5"]));
        assert_eq!(bottom["zero"]["xy"]["finished_in"],json!(["top","top"]));
        // With the cap removed, the same retained top bore is the preferred pickup.
        // This isolated selector case does not claim the shipped program removed it.
        let mut uncapped=out["plan"].clone();
        let vertices:Vec<[f64;3]>=m["positions"].as_array().unwrap().as_chunks::<3>().0.iter().map(|p|
            std::array::from_fn(|i|(0..3).map(|j|bottom["rotation"][i][j].as_f64().unwrap()*(p[j].as_f64().unwrap()-bottom["origin_model"][j].as_f64().unwrap())).sum())).collect();
        let indices:Vec<_>=m["indices"].as_array().unwrap().as_chunks::<3>().0.to_vec();
        let mut stocks=vec![Value::Null;setups.len()];
        let bi=setups.iter().position(|s|s["id"]=="bottom").unwrap();
        stocks[bi]=json!({"mesh":{"vertices":vertices,"indices":indices}});
        capp::zero::locate(&mut uncapped,&a,&m,&stocks);
        let b=uncapped["setups"].as_array().unwrap().iter().find(|s|s["id"]=="bottom").unwrap();
        assert_eq!(b["zero"]["xy"]["features"],json!(["holes:36"]));
        assert_eq!(b["zero"]["xy"]["finished_in"],"top");
        // Top machining cannot finish the underside before the flip. It must
        // explicitly transfer Z from stock rather than invent an earlier finish.
        assert_eq!(bottom["zero"]["z"]["features"],json!(["faces:4"]));
        assert!(bottom["zero"]["z"]["finished_in"].is_null());
        assert!(bottom["zero"]["z"]["description"].as_str().unwrap().contains("touch off stock top at Z+"));
        let front=setups.iter().find(|s|s["id"]=="front").unwrap();
        assert_eq!(front["zero"]["xy"]["features"],json!(["faces:0","faces:4"]));
        assert_eq!(front["zero"]["xy"]["finished_in"],json!(["top","bottom"]));
        assert_eq!(front["zero"]["z"]["features"],json!(["faces:11"]));
        assert_eq!(front["zero"]["z"]["finished_in"],"top");
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
    let offsets=capp::zero::locate(&mut p,&json!({"features":[],"faces":[]}),&json!({}),&[]);
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
