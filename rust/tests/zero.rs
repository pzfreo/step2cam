//! Datum pickup uses retained earlier finishing, never a merely recognised feature.
use serde_json::{Value, json};
fn read(p: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(p)).unwrap()).unwrap()
}
#[test]
fn stepped_plate_pickups_are_recorded_for_every_library() {
    let a=read("tests/fixtures/synthetic/stepped-plate.analysis.json");
    let m=read("tests/fixtures/synthetic/stepped-plate.mesh.json");
    for lib in ["demo/tools.json","libraries/haas-vf2.json","libraries/linuxcnc-vmc.json","tests/fixtures/placeholder-tools.json"] {
        let out=step2cam::process(&a,&m,&read(lib)).unwrap();
        assert!(out["error"].is_null(),"{lib}: {}",out["error"]);
        let setups=out["plan"]["setups"].as_array().unwrap();
        let top=&setups[0];
        assert_eq!(top["clamping"]["fixed_jaw"],"y.min");
        assert_eq!(top["stock"]["min"][0],0.);
        assert_eq!(top["stock"]["min"][1],0.);
        assert_eq!(top["stock"]["max"][2],0.);
        assert_eq!(top["zero"]["z"]["description"],"stock top");
        assert_eq!(top["zero"]["xy"]["description"],"the stock corner at the fixed jaw (−Y), −X end");
        let bottom=setups.iter().find(|s|s["id"]=="bottom").unwrap();
        let cap_warnings:Vec<_>=out["plan"]["decisions"].as_array().unwrap().iter().filter(|d|d["subject"]=="bottom" && d["outcome"]=="warning" && d["says"].as_str().unwrap().contains("covers finished bore pickup mouths")).collect();
        assert_eq!(cap_warnings.len(),1);
        assert!(cap_warnings[0]["features"].as_array().unwrap().contains(&json!("holes:36")));
        assert!(!cap_warnings[0]["says"].as_str().unwrap().contains("exposed finished faces"));
        assert_eq!(bottom["zero"]["xy"]["features"],json!(["faces:0","faces:5"]));
        assert_eq!(bottom["zero"]["xy"]["finished_in"],json!(["top","top"]));
        assert_eq!(bottom["zero"]["xy"]["method"],"fixture_contacts");
        let x=&bottom["zero"]["xy"]["axes"]["x"];let y=&bottom["zero"]["xy"]["axes"]["y"];
        assert_eq!(x["method"],"fixed_jaw_contact");assert_eq!(x["features"],json!(["faces:0"]));
        assert_eq!(y["method"],"part_stop_contact");assert_eq!(y["features"],json!(["faces:5"]));
        assert_eq!(x["finished_in"],"top");assert_eq!(y["finished_in"],"top");
        assert_eq!(x["coordinate_mm"],0.);assert_eq!(y["coordinate_mm"],0.);
        assert_eq!(bottom["clamping"]["part_stop"]["axis"],"y");
        assert_eq!(bottom["clamping"]["part_stop"]["coordinate_mm"],0.);
        assert_eq!(bottom["clamping"]["part_stop"]["feature"],"faces:5");
        let stop=&bottom["clamping"]["part_stop"];
        let height=stop["maximum_z_mm"].as_f64().unwrap();
        assert!(height>stop["contact_z_mm"].as_f64().unwrap());
        assert!(height<=stop["contact_z_mm"].as_f64().unwrap()+0.5);
        assert!(height<=stop["overhang_z_mm"].as_f64().unwrap()-0.5);
        let text=y["description"].as_str().unwrap();
        assert!(text.contains("pick up the stop face as Y0 before loading"));
        assert!(text.contains("stop below Z−"));
        let decision=out["plan"]["decisions"].as_array().unwrap().iter().find(|d|d["id"]==bottom["zero"]["because"][0]).unwrap();
        for id in ["faces:0","faces:5"] {assert!(decision["rejected_pickups"].as_array().unwrap().iter().any(|r|r["feature"]==id));}
        let z=&bottom["zero"]["z"];
        assert_eq!(z["method"],"parallel_seat");assert_eq!(z["reference_z_mm"],-12.);
        assert_eq!(z["offset_mm"],12.);assert_eq!(z["target_features"],json!(["faces:4"]));
        assert!(z["features"].as_array().unwrap().contains(&json!("faces:1")));
        assert!(z["seats"].as_array().unwrap().iter().all(|seat|seat["finished_in"]=="top"));
        assert!(z["description"].as_str().unwrap().contains("touch off a parallel before loading"));
        assert!(!z["description"].as_str().unwrap().contains("stock top"));
        for bar in bottom["clamping"]["parallels"].as_array().unwrap() {assert_eq!(bar["max"][2],z["reference_z_mm"]);}
        // With an explicit uncapped part mesh, drawing datums still outrank a
        // non-datum bore. Without drawing datums, the largest finished bore wins.
        let mut uncapped=out["plan"].clone();
        let vertices:Vec<[f64;3]>=m["positions"].as_array().unwrap().as_chunks::<3>().0.iter().map(|p|
            std::array::from_fn(|i|(0..3).map(|j|bottom["rotation"][i][j].as_f64().unwrap()*(p[j].as_f64().unwrap()-bottom["origin_model"][j].as_f64().unwrap())).sum())).collect();
        let indices:Vec<_>=m["indices"].as_array().unwrap().as_chunks::<3>().0.to_vec();
        let mut stocks=vec![Value::Null;setups.len()];
        let bi=setups.iter().position(|s|s["id"]=="bottom").unwrap();
        stocks[bi]=json!({"mesh":{"vertices":vertices,"indices":indices}});
        capp::zero::locate(&mut uncapped,&a,&m,&stocks);
        assert_eq!(uncapped["setups"][bi]["zero"]["xy"]["features"],json!(["faces:0","faces:5"]));
        let mut no_datums=a.clone();no_datums["characteristics"]=json!([]);
        capp::zero::locate(&mut uncapped,&no_datums,&m,&stocks);
        assert_eq!(uncapped["setups"][bi]["zero"]["xy"]["features"],json!(["holes:36"]));
        assert_eq!(uncapped["setups"][bi]["zero"]["xy"]["finished_in"],"top");
        // Missing parallels or unfinished seat faces must never imply a finished
        // thickness reference. The raw-bar fallback now carries its limitation.
        for remove_seat in [false,true] {
            let mut fallback=out["plan"].clone();
            if remove_seat {fallback["setups"][0]["operations"].as_array_mut().unwrap().retain(|o|o["kind"]!="flats");}
            else {fallback["setups"][bi]["clamping"]["parallels"]=json!([]);}
            capp::zero::locate(&mut fallback,&a,&m,&[]);
            let z=&fallback["setups"][bi]["zero"]["z"];
            assert_eq!(z["method"],"stock_touch_off");assert!(z["finished_in"].is_null());
            assert!(z["description"].as_str().unwrap().contains("measure the part after facing; bar thickness varies"));
            assert!(fallback["warnings"].as_array().unwrap().iter().any(|w|w.as_str().unwrap().contains("setup bottom: no finished seat")));
        }
        let front=setups.iter().find(|s|s["id"]=="front").unwrap();
        assert_eq!(front["zero"]["xy"]["features"],json!(["faces:0","faces:4"]));
        assert_eq!(front["zero"]["xy"]["finished_in"],json!(["top","bottom"]));
        assert_eq!(front["zero"]["xy"]["axes"]["x"]["method"],"part_stop_contact");
        assert_eq!(front["zero"]["xy"]["axes"]["y"]["method"],"probe_face");
        assert_eq!(front["zero"]["z"]["method"],"probe_face","{lib}");
        assert_eq!(front["zero"]["z"]["features"],json!(["faces:11"]),"{lib}");
        assert_eq!(front["zero"]["z"]["finished_in"],"top","{lib}");
        assert!(!out["plan"]["warnings"].as_array().unwrap().iter().any(|w|w.as_str().unwrap().starts_with("setup front: no finished seat")));
        let chamfered=step2cam::process(&read("tests/fixtures/synthetic/stepped-plate-chamfered.analysis.json"),&read("tests/fixtures/synthetic/stepped-plate-chamfered.mesh.json"),&read(lib)).unwrap();
        assert!(chamfered["error"].is_null(),"{lib}: {}",chamfered["error"]);
        let front=chamfered["plan"]["setups"].as_array().unwrap().iter().find(|s|s["id"]=="front").unwrap();
        assert_eq!(front["zero"]["z"]["method"],"probe_face","{lib}");
        assert_eq!(front["zero"]["z"]["features"],json!(["faces:6"]),"{lib}");
        assert_eq!(front["zero"]["z"]["finished_in"],"top","{lib}");
        assert!(!front["zero"]["z"]["description"].as_str().unwrap().contains("face 11"));
        for s in setups {assert!(s["zero"].is_object());}
        let validator=jsonschema::validator_for(&read("schemas/plan-1.schema.json")).unwrap();
        let errors:Vec<_>=validator.iter_errors(&out["plan"]).map(|e|format!("{e} at {}",e.instance_path())).collect();
        assert!(errors.is_empty(),"{}",errors.join("\n"));
        let mut bad=out["plan"].clone();
        let d=bad["decisions"].as_array_mut().unwrap().iter_mut().find(|d|d["subject"]=="bottom" && d["rejected_pickups"].as_array().is_some_and(|r|!r.is_empty())).unwrap();
        d["rejected_pickups"][0]["unrecognised"]=json!(true);
        assert!(!validator.is_valid(&bad),"rejection evidence remains strictly validated");
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
    assert!(p["warnings"][0].as_str().unwrap().contains("no usable finished feature to locate X/Y"));
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
