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
                if c["parallels_size"]["kind"] == "computed" {
                    assert_eq!(s["id"], "front");
                    assert!(list(&c["parallels"]).is_empty());
                    assert_eq!(c["grip_height_mm"], 11.2);
                } else { assert!(heights.contains(&c["parallels_size"]["height_mm"])); }
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

#[test]
fn standard_miss_keeps_main_support_and_warns_once_even_after_refinishing() {
    let tools = fixture("../../demo/tools.json");
    let mut p = planned(&tools);
    for _ in 0..2 { capp::finish(&mut p, &tools, &[]).unwrap(); }
    let front = list(&p["setups"]).iter().find(|s|s["id"]=="front").unwrap();
    assert_eq!(front["clamping"]["grip_height_mm"], 11.2);
    assert_eq!(front["clamping"]["parallels_size"]["kind"], "computed");
    assert!(list(&front["clamping"]["parallels"]).is_empty());
    assert!(!list(&front["operations"]).is_empty());
    let warnings: Vec<_> = list(&p["decisions"]).iter().filter(|d| d["subject"]=="front" && d["outcome"]=="warning"
        && d["chosen"]["parallels_size"]["kind"]=="computed").collect();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0]["says"].as_str().unwrap().contains("not a standard size"));
    assert_eq!(list(&p["warnings"]).iter().filter(|w| **w==warnings[0]["says"]).count(), 1);
}

#[test]
fn flip_uses_deepest_available_grip_and_first_setup_keeps_least_grip() {
    let rule = capp::rules::workholding();
    let minimum = number(&rule["min_grip_mm"]);
    let fraction = number(&rule["min_grip_fraction"]);
    let maximum = number(&rule["max_grip_fraction"]);
    let clearance = number(&rule["clearance_mm"]);
    for path in ["../../demo/tools.json", "../../libraries/haas-vf2.json"] {
        for inventory in [false, true] {
            let mut tools = fixture(path);
            tools["overrides"]["workholding_option"] = json!("vise_flip_face");
            let vise = &mut tools["machines"][0]["workholding"][0];
            let jaw = number(&vise["jaw_height_mm"]);
            // Synthetic shop set: three eligible receiving grips plus one over the limit.
            if inventory { vise["parallels_heights_mm"] = json!([jaw-minimum, jaw-4.125, jaw-6., jaw-7.3]); }
            let heights = if inventory { list(&vise["parallels_heights_mm"]).clone() }
                else { list(&capp::rules::standard_sizes()["parallels"]["heights_mm"]).clone() };
            let p = planned(&tools);
            let top = &p["setups"][0];
            let bottom = list(&p["setups"]).iter().find(|s| s["id"]=="bottom").unwrap();
            assert!(list(&bottom["operations"]).iter().any(|o| o["kind"]=="facing"), "prepared grip must remain usable");
            let seat = &bottom["flip_seating"];
            let base = number(&p["part"]["min"][2]);
            let h = number(&seat["seat_z_model"])-base;
            let fits = |height: f64, stock: f64| {
                let g = jaw-height;
                g >= minimum.max(stock*fraction) && g <= stock*maximum && g <= jaw && stock > g+clearance
            };
            let expected = heights.iter().map(number).filter(|p| fits(*p,h)).map(|p| jaw-p).max_by(f64::total_cmp).unwrap();
            assert_close(number(&bottom["clamping"]["grip_height_mm"]), expected);
            assert_close(number(&bottom["clamping"]["parallels_size"]["height_mm"]), jaw-expected);
            let span = number(&seat["along_max_model"])-number(&seat["along_min_model"]);
            for contact in list(&bottom["clamping"]["contacts"]) {
                assert_close(number(&contact["area_mm2"]), span*expected);
                assert!(number(&contact["area_mm2"]) > span*minimum, "more contact than main's minimum grip");
            }
            let h = number(&top["stock"]["max"][2])-number(&top["stock"]["min"][2]);
            let expected_top = heights.iter().map(number).filter(|p| fits(*p,h)).map(|p| jaw-p).min_by(f64::total_cmp).unwrap();
            assert_close(number(&top["clamping"]["grip_height_mm"]), expected_top);
            assert!(list(&p["decisions"]).iter().any(|d| d["rule"]=="setup.flip_face@5"
                && d["says"].as_str().unwrap().contains("flip: deepest allowed grip, the sides being finished; first setup: least grip, leaving the sides exposed for cutting")));
        }
    }
}

#[test]
fn imperial_products_use_exact_conversions_and_keep_supplier_labels() {
    let sizes = capp::rules::standard_sizes();
    let products = list(&sizes["flat_bar"]["products"]);
    assert_eq!(products.len(), 201);
    let mut imperial = 0;
    for product in products {
        assert!(product["supplier_label"].as_str().unwrap().contains("Aluminium Flat Bar"));
        if let Some(inches) = product["size_inch"].as_array() {
            imperial += 1;
            for (i, inch) in inches.iter().enumerate() {
                assert_close(number(&product["size_mm"][i]), number(inch)*25.4);
            }
        }
    }
    assert_eq!(imperial, 124);
    let p = products.iter().find(|p| p["supplier_label"].as_str().unwrap().starts_with("64mm x 12.7mm ")).unwrap();
    assert_eq!(p["size_mm"], json!([12.7,63.5]));
}

#[test]
fn stock_keeps_the_top_margin_and_respects_the_vise_opening() {
    let a = fixture("synthetic/stepped-plate.analysis.json");
    let m = fixture("synthetic/stepped-plate.mesh.json");
    let mut tools = fixture("../../demo/tools.json");
    tools["overrides"]["workholding_option"] = json!("none");
    for opening in [60., 70., 150.] {
        tools["machines"][0]["workholding"][0]["max_jaw_opening_mm"] = json!(opening);
        let p = capp::flip::plan(&a, &m, &tools).unwrap();
        let top = &p["setups"][0];
        assert_close(number(&top["origin_model"][2]), number(&m["bbox"][1][2])+capp::plan::Options::default().top_stock);
        let width = number(&top["stock"]["max"][1])-number(&top["stock"]["min"][1]);
        assert!(width <= opening);
        if top["stock_size"]["kind"] == "standard" {
            assert!(number(&top["stock_size"]["cross_section_mm"][1]) <= opening);
        }
    }
    // No listed bar can supply a 64 mm opening at the plate's required thickness;
    // retain the computed 64 mm blank rather than selecting an oversize bar.
    let a = fixture("plate.analysis.json");
    let m = fixture("plate.mesh.json");
    tools["machines"][0]["workholding"][0]["max_jaw_opening_mm"] = json!(64.);
    let p = capp::flip::plan(&a, &m, &tools).unwrap();
    let top = &p["setups"][0];
    assert_eq!(top["stock_size"]["kind"], "computed");
    assert_eq!(top["stock"]["max"][1], 64.);
    assert!(p["warnings"].to_string().contains("stock is the computed size"));
}
