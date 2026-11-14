//! Published nominal sizes are data, never shop inventory.
mod common;
use common::*;
use serde_json::{Value, json};

fn planned(tools: &Value) -> Value {
    let a = fixture("synthetic/stepped-plate.analysis.json");
    let m = fixture("synthetic/stepped-plate.mesh.json");
    let mut p = capp::flip::plan(&a, &m, tools).unwrap();
    capp::finish(&mut p, tools, &[]).unwrap();
    p
}

#[test]
fn shipped_mills_use_cited_heights_and_flat_bars() {
    let sizes = capp::rules::standard_sizes();
    let heights = list(&sizes["parallels"]["heights_mm"]);
    assert_eq!(heights.len(), 10);
    for (i,h) in heights.iter().enumerate() {
        assert_close(number(h), (0.5+i as f64/8.)*25.4);
    }
    for path in ["../../demo/tools.json", "../../libraries/haas-vf2.json"] {
        let tools = fixture(path);
        assert!(tools["machines"][0]["workholding"][0].get("parallels_heights_mm").is_none());
        let p = planned(&tools);
        let first = &p["setups"][0];
        assert_eq!(first["stock_size"]["kind"], "standard");
        let bar = &first["stock_size"]["cross_section_mm"];
        assert!(list(&sizes["flat_bar"]["sizes_mm"]).iter().any(|b| b == bar || json!([b[1],b[0]]) == *bar));
        let spans: Vec<_> = (0..3).map(|i| number(&first["stock"]["max"][i])-number(&first["stock"]["min"][i])).collect();
        assert_close(spans[2], number(&bar[0]));
        assert!(spans[..2].contains(&number(&bar[1])));
        assert_eq!(number(&first["stock_size"]["sawn_length_mm"]).fract(), 0.);
        let c = &first["clamping"];
        assert_eq!(c["parallels_size"]["kind"], "standard");
        assert!(heights.contains(&c["parallels_size"]["height_mm"]));
        for s in list(&p["setups"]) {
            if let Some(c) = s.get("clamping") {
                assert!(heights.contains(&c["parallels_size"]["height_mm"]));
                for support in list(&c["parallels"]) {
                    assert_close(number(&support["max"][2])-number(&support["min"][2]), number(&c["parallels_size"]["height_mm"]));
                }
            }
        }
    }
}

#[test]
fn shop_inventory_and_fixed_height_win_and_empty_inventory_refuses() {
    let mut tools = fixture("../../demo/tools.json");
    tools["machines"][0]["workholding"][0]["parallels_heights_mm"] = json!([11., 13.]);
    let p = planned(&tools);
    assert_eq!(p["setups"][0]["clamping"]["parallels_size"]["kind"], "library");
    assert_eq!(p["setups"][0]["clamping"]["parallels_size"]["height_mm"], 13.);
    tools["machines"][0]["workholding"][0]["parallels_height_mm"] = json!(12.);
    assert_eq!(planned(&tools)["setups"][0]["clamping"]["parallels_size"]["height_mm"], 12.);
    tools["machines"][0]["workholding"][0].as_object_mut().unwrap().remove("parallels_height_mm");
    tools["machines"][0]["workholding"][0]["parallels_heights_mm"] = json!([]);
    let p = planned(&tools);
    assert!(list(&p["setups"][0]["operations"]).is_empty());
    assert!(p["warnings"].to_string().contains("no available shop parallels"));
}

#[test]
fn oversized_part_keeps_computed_blank_and_warns() {
    let a = synthetic(&[], ([0.,0.,0.],[1000.,1000.,1000.]), &[], &[]);
    let p = capp::plan::plan(&a, None, &Default::default()).unwrap();
    // Add a hole so a setup is emitted even without a mesh/roughing.
    assert!(p["warnings"].to_string().contains("stock is the computed size"));
    let a = synthetic(&[hole([0.,0.,-1.],[500.,500.,1000.],5.,10.,"flat")], ([0.,0.,0.],[1000.,1000.,1000.]), &[], &[]);
    let p = capp::plan::plan(&a, None, &Default::default()).unwrap();
    assert_eq!(p["setups"][0]["stock_size"]["kind"], "computed");
    assert_eq!(p["setups"][0]["stock"]["max"], json!([1004.,1004.,0.]));
    assert_eq!(p["setups"][0]["stock"]["min"], json!([0.,0.,-1001.]));
}

#[test]
fn selected_flip_searches_only_real_heights_and_preserves_sacrificial_grip() {
    for path in ["../../demo/tools.json", "../../libraries/haas-vf2.json"] {
        let mut tools = fixture(path);
        tools["overrides"]["workholding_option"] = json!("vise_flip_face");
        let p = planned(&tools);
        let top = &p["setups"][0];
        let bottom = list(&p["setups"]).iter().find(|s| s["id"] == "bottom").expect("flip seating");
        let heights = list(&capp::rules::standard_sizes()["parallels"]["heights_mm"]);
        for setup in [top,bottom] {
            assert!(heights.contains(&setup["clamping"]["parallels_size"]["height_mm"]));
        }
        let sacrificed = number(&bottom["stock"]["max"][2]);
        let grip = number(&top["clamping"]["grip_height_mm"]);
        assert!(sacrificed >= grip + 2. - 1e-4, "keep rule clearance plus facing allowance");
        assert_eq!(bottom["stock_size"]["kind"], "remaining");
    }
}

#[test]
fn explicit_setup_height_is_shop_provenance() {
    let mut tools = fixture("../../demo/tools.json");
    tools["overrides"]["clamping"]["top"] = json!({"axis":"y", "grip_height_mm":7., "parallels_height_mm":13.});
    let p = planned(&tools);
    let c = &p["setups"][0]["clamping"];
    assert_eq!(c["parallels_size"]["kind"], "library");
    assert_eq!(c["parallels_size"]["height_mm"], 13.);
    assert_eq!(c["grip_height_mm"], 7.);
}
