mod common;
use common::*;
use capp::{plan::{Options, plan}, resources::Library};
use serde_json::{Value, json};

fn ops(p: &Value, kind: &str) -> Vec<Value> {
    p["setups"].as_array().unwrap().iter().flat_map(|s| s["operations"].as_array().unwrap()).filter(|o| o["kind"] == kind).cloned().collect()
}
fn finish(a: &Value, lib: &Value) -> Value {
    let mut p = plan(a, None, &top_stock(0.)).unwrap();
    capp::finish(&mut p, lib, &[]).unwrap();
    p
}
fn spot(lib: &Value, diameter: f64, angle: f64, name: &str) -> Value {
    let mut t = lib["tools"].as_array().unwrap().iter().find(|t| t["type"] == "spot_drill").unwrap().clone();
    t["diameter"] = json!(diameter); t["point_angle"] = json!(angle); t["id"] = json!(name); t
}

#[test]
fn plate_spots_are_grouped_before_twist_drills_and_match_the_formula() {
    let lib = fixture("../../tests/fixtures/placeholder-tools.json");
    let mut p = plan(&fixture("plate.analysis.json"), Some(&fixture("plate.mesh.json")), &Options::default()).unwrap();
    capp::finish(&mut p, &lib, &[]).unwrap();
    let spots = ops(&p,"spot_drill");
    assert_eq!(spots.len(),1);
    assert_eq!(spots[0]["tool"],"spot8");
    let drilled: Vec<_> = ops(&p,"holes").into_iter().filter(|o| o["tool"].as_str().is_some_and(|id| id.starts_with("drill"))).collect();
    let expected: usize = drilled.iter().map(|o| o["features"].as_array().unwrap().len()).sum();
    assert_eq!(spots[0]["spots"].as_array().unwrap().len(),expected);
    for s in spots[0]["spots"].as_array().unwrap() {
        assert_close(number(&s["depth"]),number(&s["diameter"])/2./60f64.to_radians().tan());
    }
    let ordered = p["setups"][0]["operations"].as_array().unwrap();
    assert!(ordered.iter().position(|o| o["kind"]=="spot_drill").unwrap() < ordered.iter().position(|o| o["kind"]=="holes").unwrap());
    let (jobs,_) = step2cam::job::jobs(&p,&lib).unwrap();
    let cycles: Vec<_> = jobs[0].manifest["operations"].as_array().unwrap().iter().filter(|o| o["tool"]=="spot8").collect();
    for c in cycles { assert_eq!(c["depth_reference"],"tip"); assert!(c["peck_mm"].is_null()); assert_eq!(c["dwell_s"],0.2); }
}

#[test]
fn matching_countersink_is_cut_but_wrong_angle_or_diameter_keeps_the_skip() {
    let mut lib = example_tools();
    let right_angle = spot(&lib,8.,90.,"spot90");
    lib["tools"].as_array_mut().unwrap().push(right_angle);
    // The 90° case formerly encoded the erroneous twist-drill angle restriction.
    for (angle,major,cut) in [(120.,7.,true),(120.8,7.,true),(122.,7.,false),(90.,7.,true),(120.,25.,false)] {
        let h = with(down(0.,0.,4.2,8.,"flat"),"csink",json!({"major_diameter":major,"included_angle":angle}));
        let p = finish(&holes_only(&[h]),&lib);
        let spots = ops(&p,"spot_drill"); let s = &spots[0]["spots"][0];
        assert_eq!(s["countersink"],cut);
        assert_eq!(number(&s["diameter"]),if cut { major } else { 4.2 });
        assert_eq!(p["skipped"].as_array().unwrap().iter().any(|s| s["reason"].as_str().unwrap().contains("chamfer it by hand")),!cut);
        assert!(ops(&p,"edge_chamfer").is_empty());
    }
}

#[test]
fn missing_spot_tool_keeps_drilling_and_describes_the_needed_tool() {
    let mut lib=example_tools();
    lib["tools"].as_array_mut().unwrap().retain(|t| t["type"]!="spot_drill");
    let p=finish(&holes_only(&[down(0.,0.,4.2,8.,"flat")]),&lib);
    // The spot drilling no tool cuts is not an operation: its hole says why, and a warning says
    // the hole is drilled without spotting.
    assert!(ops(&p,"spot_drill").is_empty());
    let skipped=p["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(),1,"{skipped:?}");
    let reason=skipped[0]["reason"].as_str().unwrap();
    assert!(reason.starts_with("spot drilling left out: no usable spot drill: needs a spot drill"),"{reason}");
    let setup=p["setups"][0]["id"].as_str().unwrap();
    let need=reason.split_once(": needs ").unwrap().1;
    assert!(p["warnings"].as_array().unwrap().contains(&json!(format!("setup {setup}: 1 hole drilled without spotting: no usable spot drill: needs {need}"))),"{}",p["warnings"]);
    let warned=p["decisions"].as_array().unwrap().iter().find(|d| d["rule"]=="tool.none.unspotted@1" && d["outcome"]=="warning").unwrap();
    assert_eq!(warned["features"],json!([skipped[0]["feature"]]));
    assert_eq!(p["tools_needed"][0]["description"],need);
    assert_eq!(p["tools_needed"][0]["features"],skipped[0]["feature"].as_str().map(|f| json!([f])).unwrap());
    assert_eq!(ops(&p,"holes")[0]["tool"],"drill4.2");
    let (jobs,_) = step2cam::job::jobs(&p,&lib).unwrap();
    assert_eq!(jobs[0].manifest["operations"].as_array().unwrap().len(),1);
}

#[test]
fn bores_and_thread_mills_are_not_spotted_but_tap_drills_are() {
    let lib=example_tools();
    let p=finish(&holes_only(&[down(0.,0.,8.,8.,"flat")]),&lib);
    assert_eq!(ops(&p,"holes")[0]["tool"],"em6");
    assert!(ops(&p,"spot_drill").is_empty());
    let a=tapped(json!({"hole.function:holes:0":"tapped:M5"}),holes_only(&[down(0.,0.,4.2,8.,"through")]));
    let p=finish(&a,&lib);
    assert_eq!(ops(&p,"thread").len(),1);
    assert_eq!(ops(&p,"spot_drill")[0]["spots"].as_array().unwrap().len(),1);
}

#[test]
fn exclusion_indices_survive_pruning_and_refused_spots_stay_out() {
    let lib=example_tools();
    let a=holes_only(&[down(0.,0.,4.2,8.,"flat"),down(8.,0.,6.6,8.,"flat"),down(15.,0.,8.,8.,"flat")]);
    let planned=plan(&a,None,&top_stock(0.)).unwrap();
    let mut initial=planned.clone();capp::finish(&mut initial,&lib,&[]).unwrap();
    let setup=initial["setups"][0]["id"].as_str().unwrap().to_owned();
    let mut exclusions=vec![];
    // Repeatedly exclude the first live drill using the indices the engine actually sees.
    for _ in 0..3 {
        let n=initial["setups"][0]["operations"].as_array().unwrap().iter().position(|o| o["kind"]=="holes").unwrap();
        exclusions.push(json!({"operation":format!("{setup}-{}-holes",n+1),"message":"test refusal"}));
        initial=planned.clone();capp::finish(&mut initial,&lib,&exclusions).unwrap();
    }
    assert!(ops(&initial,"holes").is_empty()); assert!(ops(&initial,"spot_drill").is_empty());
    let mut p=planned.clone();
    capp::finish(&mut p,&lib,&[json!({"operation":format!("{setup}-1-spot_drill-1"),"message":"spot refused"})]).unwrap();
    assert!(ops(&p,"spot_drill").is_empty()); assert_eq!(ops(&p,"holes").len(),3);
}

#[test]
fn smallest_covering_tool_and_reach_are_selected_and_fields_are_required() {
    let mut lib=example_tools();
    let large=spot(&lib,10.,142.,"large");let small=spot(&lib,5.,120.,"small");
    lib["tools"].as_array_mut().unwrap().retain(|t| t["type"]!="spot_drill");
    lib["tools"].as_array_mut().unwrap().extend([large,small]);
    let a=holes_only(&[down(0.,0.,4.2,8.,"flat")]);
    assert_eq!(ops(&finish(&a,&lib),"spot_drill")[0]["tool"],"small");
    let index=lib["tools"].as_array().unwrap().len()-1;
    lib["tools"][index]["flute_length"]=json!(0.1);
    assert_eq!(ops(&finish(&a,&lib),"spot_drill")[0]["tool"],"large");
    for field in ["point_angle","flute_length","feed_mm_min"] {
        let mut missing=lib.clone();missing["tools"][index].as_object_mut().unwrap().remove(field);
        assert!(Library::new(&missing).err().unwrap().contains(field));
    }
}

#[test]
fn mixed_mouth_heights_and_depths_split_cycles_without_extra_tools() {
    let lib=example_tools();
    let a=holes_only(&[down(0.,0.,4.2,8.,"flat"),hole([0.,0.,-1.],[8.,0.,-2.],4.2,6.,"flat"),down(12.,0.,6.6,8.,"flat")]);
    let p=finish(&a,&lib);
    assert_eq!(ops(&p,"spot_drill").len(),1);
    let (jobs,_) = step2cam::job::jobs(&p,&lib).unwrap();
    let cycles: Vec<_>=jobs[0].manifest["operations"].as_array().unwrap().iter().filter(|o| o["tool"]=="spot8").collect();
    assert_eq!(cycles.len(),3);
    let specs=jobs[0].manifest["tools"].as_array().unwrap();
    assert_eq!(specs.iter().filter(|t| t["id"]=="spot8").count(),1);
    for s in ops(&p,"spot_drill")[0]["spots"].as_array().unwrap() {
        let z=number(&s["top_z"])-number(&s["depth"]);
        assert!(cycles.iter().any(|c| number(&c["depth_z_mm"])==z && c["centers_mm"].as_array().unwrap().contains(&s["center"])));
    }
}

#[test]
fn unusable_spot_tools_do_not_substitute_an_undersized_or_sharper_spot() {
    for (field,value) in [("diameter",3.),("point_angle",90.),("flute_length",0.1),("stickout",0.1)] {
        let mut lib=example_tools();
        let mut t=spot(&lib,8.,120.,"unusable");t[field]=json!(value);
        lib["tools"].as_array_mut().unwrap().retain(|t| t["type"]!="spot_drill");
        lib["tools"].as_array_mut().unwrap().push(t);
        let p=finish(&holes_only(&[down(0.,0.,4.2,8.,"flat")]),&lib);
        assert!(ops(&p,"spot_drill").is_empty(),"{field}");
        assert!(p["skipped"][0]["reason"].as_str().unwrap().starts_with("spot drilling left out: no usable spot drill"),"{field}");
        assert_eq!(ops(&p,"holes")[0]["tool"],"drill4.2");
    }
}

#[test]
fn refused_matching_spot_leaves_its_countersink_for_hand_finishing() {
    let lib=example_tools();
    let h=with(down(0.,0.,4.2,8.,"flat"),"csink",json!({"major_diameter":7.,"included_angle":120.}));
    let mut p=plan(&holes_only(&[h]),None,&top_stock(0.)).unwrap();
    let mut initial=p.clone();capp::finish(&mut initial,&lib,&[]).unwrap();
    let setup=initial["setups"][0]["id"].as_str().unwrap();
    capp::finish(&mut p,&lib,&[json!({"operation":format!("{setup}-1-spot_drill-1"),"message":"spot refused"})]).unwrap();
    assert!(ops(&p,"spot_drill").is_empty());
    assert!(p["skipped"].as_array().unwrap().iter().any(|s|s["reason"].as_str().unwrap().contains("chamfer it by hand")));
    assert_eq!(ops(&p,"holes")[0]["tool"],"drill4.2");
}

#[test]
fn pruning_updates_the_missing_tool_query_to_the_remaining_spots() {
    let mut lib=example_tools();
    lib["tools"].as_array_mut().unwrap().retain(|t| t["type"]!="spot_drill");
    let mut p=plan(&holes_only(&[down(0.,0.,4.2,8.,"flat"),down(8.,0.,6.6,8.,"flat")]),None,&top_stock(0.)).unwrap();
    let setup=p["setups"][0]["id"].as_str().unwrap().to_owned();
    // The finished plan the engine saw has no spot drilling: the second holes are its second operation.
    capp::finish(&mut p,&lib,&[json!({"operation":format!("{setup}-2-holes"),"message":"drill refused"})]).unwrap();
    assert!(ops(&p,"spot_drill").is_empty());
    assert_eq!(ops(&p,"holes").len(),1);
    assert_eq!(ops(&p,"holes")[0]["diameter"],4.2);
    let spot:Vec<_>=p["skipped"].as_array().unwrap().iter().filter(|s| s["reason"].as_str().unwrap().starts_with("spot drilling left out")).collect();
    assert_eq!(spot.len(),1);
    assert!(spot[0]["reason"].as_str().unwrap().contains("at least ⌀4.2"));
    assert!(p["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains(": 1 hole drilled without spotting: ")));
}

#[test]
fn right_angle_spot_cuts_an_enlarged_countersink_but_not_an_ordinary_spot() {
    let mut lib=example_tools();
    let right_angle=spot(&lib,8.,90.,"spot90");
    lib["tools"].as_array_mut().unwrap().retain(|t| t["type"]!="spot_drill");
    lib["tools"].as_array_mut().unwrap().push(right_angle);
    let h=down(0.,0.,4.2,8.,"flat");
    let ordinary=finish(&holes_only(std::slice::from_ref(&h)),&lib);
    assert!(ops(&ordinary,"spot_drill").is_empty());
    assert_eq!(ops(&ordinary,"holes")[0]["tool"],"drill4.2");

    let h=with(h,"csink",json!({"major_diameter":7.,"included_angle":90.}));
    let p=finish(&holes_only(&[h]),&lib);
    let group=ops(&p,"spot_drill").remove(0);
    assert_eq!(group["tool"],"spot90");
    let s=&group["spots"][0];
    assert_eq!(s["countersink"],true);
    assert_eq!(s["diameter"],7.);
    assert_close(number(&s["depth"]),3.5);
    assert_eq!(ops(&p,"holes")[0]["tool"],"drill4.2");
    assert!(!p["skipped"].as_array().unwrap().iter().any(|s|s["reason"].as_str().unwrap().contains("chamfer it by hand")));
    assert!(ops(&p,"edge_chamfer").is_empty());
    let (jobs,_)=step2cam::job::jobs(&p,&lib).unwrap();
    let cycle=jobs[0].manifest["operations"].as_array().unwrap().iter().find(|o|o["tool"]=="spot90").unwrap();
    assert_eq!(cycle["depth_reference"],"tip");
    assert_close(number(&cycle["depth_z_mm"]),-3.5);
}
