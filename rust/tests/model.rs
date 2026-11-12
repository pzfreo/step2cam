//! The plan's types hold today's plans exactly: each plan decoded into `capp::model::Plan` and
//! encoded again is the same JSON, as values and as text (so an integer that turns into a float, a
//! key that appears or disappears, or a `null` that becomes absent all fail).
mod common;
use capp::kind::OpKind;
use capp::model::*;
use common::*;
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}
fn read(path: &PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// The first place two values differ, for the failure message.
fn difference(a: &Value, b: &Value, path: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => x.keys().chain(y.keys()).collect::<BTreeSet<_>>().into_iter().find_map(|k| match (x.get(k), y.get(k)) {
            (Some(p), Some(q)) => difference(p, q, &format!("{path}.{k}")),
            (p, q) => Some(format!("{path}.{k}: {p:?} against {q:?}")),
        }),
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x.iter().zip(y).enumerate().find_map(|(i, (p, q))| difference(p, q, &format!("{path}[{i}]"))),
        // A number's `==` already tells 1 from 1.0.
        _ => (a != b).then(|| format!("{path}: {a} against {b}")),
    }
}

/// What the round trips have seen, so the test can show it covered the cases the data must hold.
#[derive(Default)]
struct Seen {
    plans: usize,
    kinds: BTreeSet<&'static str>,
    cases: BTreeSet<&'static str>,
}

impl Seen {
    /// Decodes the plan, encodes it again and checks both are the same JSON text.
    #[track_caller]
    fn round_trip(&mut self, plan: &Value, label: &str) {
        let typed: Plan = serde_json::from_value(plan.clone()).unwrap_or_else(|e| panic!("{label}: does not decode: {e}"));
        let back = serde_json::to_value(&typed).unwrap();
        if let Some(d) = difference(plan, &back, "plan") {
            panic!("{label}: changed in the round trip at {d}");
        }
        assert_eq!(serde_json::to_string(&back).unwrap(), serde_json::to_string(plan).unwrap(), "{label}");
        self.plans += 1;
        self.note(plan, &typed);
    }

    fn note(&mut self, plan: &Value, typed: &Plan) {
        let cases = &mut self.cases;
        let mut case = |seen: bool, name: &'static str| if seen { cases.insert(name); };
        for key in ["workholding_surface_model", "contact_surface_model"] {
            case(plan[key].is_array(), if key == "contact_surface_model" { "contact mesh present" } else { "workholding mesh present" });
            case(plan.get(key).is_some_and(Value::is_null), if key == "contact_surface_model" { "contact mesh null" } else { "workholding mesh null" });
        }
        for setup in &typed.setups {
            case(setup.part_surface.is_some(), "setup with part_surface");
            for op in setup.operations.iter().chain(setup.part_surface.as_deref()) {
                self.kinds.insert(op.cut.kind().name());
                case(matches!(op.tool, Some(None)), "tool null");
                case(matches!(op.tool, Some(Some(_))), "tool named");
                case(op.tool.is_none(), "tool absent");
                match &op.cut {
                    Cut::Holes(h) => {
                        case(h.exit_clear_mm.is_none(), "holes without exit_clear_mm");
                        case(matches!(h.exit_clear_mm, Some(None)), "holes with exit_clear_mm null");
                        case(matches!(h.exit_clear_mm, Some(Some(_))), "holes with exit_clear_mm");
                    }
                    Cut::EdgeChamfer(c) => {
                        case(c.radius.is_some() && c.room.is_none() && c.maximum_diameter.is_none(), "edge round chamfer, no room or maximum_diameter");
                        case(c.radius.is_none() && c.room.is_some() && c.maximum_diameter.is_some(), "modelled chamfer, with room and maximum_diameter");
                        case(matches!(c.maximum_diameter, Some(Some(_))), "chamfer maximum_diameter number");
                        case(matches!(c.maximum_diameter, Some(None)), "chamfer maximum_diameter null");
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Every analysis under tests/fixtures, with its mesh where it has one.
fn parts() -> Vec<(String, Value, Option<Value>)> {
    let mut found = vec![];
    let mut dirs = vec![root().join("tests/fixtures")];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Some(name) = path.file_name().unwrap().to_str().unwrap().strip_suffix(".analysis.json") {
                let mesh = dir.join(format!("{name}.mesh.json"));
                found.push((name.to_string(), read(&path), mesh.exists().then(|| read(&mesh))));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Every tool library in the repository.
fn libraries() -> Vec<(&'static str, Value)> {
    ["demo/tools.json", "examples/tools.example.json", "libraries/haas-vf2.json", "libraries/linuxcnc-vmc.json",
        "tests/fixtures/placeholder-tools.json", "tools/benchmark/tools.json"]
        .into_iter().map(|path| (path, read(&root().join(path)))).collect()
}

/// Plans with counterbores and a spotface, which no fixture part has (tests/plan.rs).
fn counterbored() -> Vec<Value> {
    let cbore = json!({"diameter": 11, "depth": 3.5});
    let records = [
        vec![with(down(0., 0., 6.6, 6.5, "through"), "cbore", cbore.clone()), with(down(10., 0., 6.6, 6.5, "through"), "cbore", cbore)],
        vec![with(with(down(0., 0., 6.6, 3.5, "through"), "cbore", json!({"diameter": 11, "depth": 6})), "spotface", json!({"diameter": 20, "depth": 0.5}))],
        vec![with(with(down(0., 0., 5., 5., "through"), "cbore", json!({"diameter": 9, "depth": 3})), "outer_steps", json!([{"diameter": 16, "depth": 2}]))],
    ];
    records.iter().map(|holes| capp::plan::plan(&holes_only(holes), None, &top_stock(0.)).unwrap()).collect()
}

#[test]
fn plans_round_trip_through_the_types_exactly() {
    let mut seen = Seen::default();
    let libraries = libraries();
    let finish = |seen: &mut Seen, plan: &Value, library: &Value, label: &str| {
        let mut plan = plan.clone();
        if capp::finish(&mut plan, library, &[]).is_ok() {
            seen.round_trip(&plan, label);
        }
    };
    for (name, analysis, mesh) in parts() {
        let planned = capp::plan::plan(&analysis, mesh.as_ref(), &Default::default()).unwrap();
        seen.round_trip(&planned, &format!("{name}: planned"));
        for (path, library) in &libraries {
            finish(&mut seen, &planned, library, &format!("{name}: finished with {path}"));
            let Some(mesh) = &mesh else { continue };
            // As the adapter plans each workholding: ordinary, then flipped where the part allows.
            for option in ["none", "vise_flip_face"] {
                let mut library = library.clone();
                library["overrides"]["workholding_option"] = json!(option);
                let Ok(plan) = capp::flip::plan(&analysis, mesh, &library) else { continue };
                seen.round_trip(&plan, &format!("{name}: {option} with {path}"));
                finish(&mut seen, &plan, &library, &format!("{name}: {option}, finished with {path}"));
            }
        }
    }
    for plan in counterbored() {
        seen.round_trip(&plan, "counterbores: planned");
        for (path, library) in &libraries {
            finish(&mut seen, &plan, library, &format!("counterbores: finished with {path}"));
        }
    }
    // The frozen baseline plans, as text.
    for entry in fs::read_dir(root().join("tests/fixtures/rust-golden")).unwrap() {
        let path = entry.unwrap().path();
        if path.to_str().unwrap().ends_with(".plan.json") {
            seen.round_trip(&read(&path), path.to_str().unwrap());
        }
    }

    let all: BTreeSet<&str> = OpKind::ALL.iter().map(|k| k.name()).collect();
    assert_eq!(seen.kinds, all, "every kind of operation");
    for case in ["setup with part_surface", "holes without exit_clear_mm", "holes with exit_clear_mm null", "holes with exit_clear_mm",
        "edge round chamfer, no room or maximum_diameter", "modelled chamfer, with room and maximum_diameter",
        "chamfer maximum_diameter number", "chamfer maximum_diameter null", "workholding mesh present", "workholding mesh null",
        "contact mesh present", "contact mesh null", "tool null", "tool named", "tool absent"] {
        assert!(seen.cases.contains(case), "no plan had {case}");
    }
    println!("{} plans round tripped", seen.plans);
}

/// One cut of each kind, as the planner might write it.
fn samples() -> Vec<Cut> {
    let outline = vec![[0., 0., 0.], [1., 0., 0.], [1., 1., 0.]];
    let features = vec!["holes:0".to_string()];
    vec![
        Cut::Facing { floor_z: 0. },
        Cut::Roughing(Roughing { top_z: 1., floor_z: -9.5, web_mm: 0.5, surface_error_mm: 0.01, positions: None, indices: None }),
        Cut::Flats { floor_z: -9.5, levels: vec![-5.] },
        Cut::SpotDrill(SpotDrill { spot_tool: None, diameter: 4., angle: 118., reach: 0., features: features.clone(),
            spots: vec![Spot { center: [0., 0.], top_z: 0., diameter: 4., depth: None, features: features.clone(), countersink: false }] }),
        Cut::Holes(Holes { diameter: 4., top_z: 0., bore_top_z: 0., bottom_z: -10., bottom: "through".into(), centers: vec![[0., 0.]],
            features: features.clone(), exit_clear_mm: Some(None), countersinks: None }),
        Cut::Counterbore(Counterbore { diameter: 8., top_z: 0., floor_z: -4., centers: vec![[0., 0.]], features: features.clone() }),
        Cut::Thread(Thread { spec: "M5".into(), major: 5., pitch: 0.8, start_diameter: 4.2, top_z: 0., bottom_z: -8., through: false,
            centers: vec![[0., 0.]], features: features.clone(), hand: None, exit_clear_mm: None }),
        Cut::Slot(Slot { width: 6., length: 10., a: [0., 0.], b: [10., 0.], top_z: 0., floor_z: -3., bottom: "blind".into(), features: features.clone() }),
        Cut::Notch(Notch { width: 6., length: 10., open: false, outer: outline.clone(), corner_radius: Some(0.), inside_corners: Some(4),
            top_z: 0., floor_z: -3., wall_top: 0., bottom: "blind".into(), features: features.clone() }),
        Cut::VChannel(VChannel { angle: 90., width: 2., depth: 1., length: 10., a: [0., 0.], b: [10., 0.], top_z: 0., root_z: -1.,
            on_flat: false, room: None, features: features.clone() }),
        Cut::WallFinish(WallFinish { wall: "wall-1".into(), outer: outline.clone(), floor_z: -9.5, on_flat: false, room: None }),
        Cut::EdgeChamfer(EdgeChamfer { needs_wall: None, wall_features: vec![], needs_openings: vec![], radius: Some(1.), width: 0.6,
            top_z: 0., outer: outline, holes: vec![], boundaries: "outer".into(), features, maximum_diameter: None, room: None }),
    ]
}

#[test]
fn each_cut_writes_its_own_kind() {
    let samples = samples();
    let kinds: BTreeSet<&str> = samples.iter().map(|c| c.kind().name()).collect();
    assert_eq!(kinds, OpKind::ALL.iter().map(|k| k.name()).collect(), "one sample of every kind");
    for cut in samples {
        let op = Operation { cut, tool: None, tool_note: None, because: None, iso14649: None, technology: None };
        let encoded = serde_json::to_value(&op).unwrap();
        assert_eq!(encoded["kind"], op.cut.kind().name());
        assert_eq!(serde_json::from_value::<Operation>(encoded).unwrap(), op);
    }
}
