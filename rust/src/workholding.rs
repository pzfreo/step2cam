//! Full candidate runs, including camlib retries. Never infer one option's cost from another.
use super::Outcome;
use capp::model::{Cut, Plan};
use capp::resources::Settings;
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn rule_name(value: &Value) -> Option<&str> {
    value.as_str()?.split('@').next()
}
fn is_flip(plan: &Plan) -> bool {
    plan.setups.iter().any(|s| s.flip_seating.as_ref().is_some_and(Value::is_object))
}
fn stock_height(plan: &Plan) -> f64 {
    plan.setups.first().map_or(0., |s| s.stock.max[2] - s.stock.min[2])
}
fn unavailable(id: &str, label: &str, reason: &str) -> Value {
    json!({"id":id,"label":label,"available":false,"reason":reason})
}
fn consequences(
    id: &str,
    label: &str,
    out: &Outcome,
    ordinary_height: f64,
    settings: &Settings,
) -> Value {
    let p = &out.plan;
    let operations: Vec<_> = p.setups.iter().flat_map(|s| &s.operations).collect();
    let omitted: BTreeSet<_> = p.skipped.iter().map(|s| s.feature.as_str()).collect();
    let left: Vec<_>=omitted.iter().map(|id|json!({"feature":id,"reasons":p.skipped.iter().filter(|s|s.feature==**id).map(|s|s.reason.clone()).collect::<Vec<_>>()})).collect();
    let cut: BTreeSet<_> = operations
        .iter()
        .filter(|o| matches!(o.tool, Some(Some(_))))
        .flat_map(|o| o.cut.features().into_iter().flatten())
        .map(String::as_str)
        .collect();
    let remaining = operations
        .iter()
        .filter_map(|o| match &o.cut {
            Cut::Roughing(rough) => Some(rough.web_mm),
            _ => None,
        })
        .fold(0_f64, f64::max);
    let grip_omission = p.skipped.iter().any(|s| {
        p.decisions.iter().flatten().any(|d| {
            s.decision.as_ref().is_some_and(|id| d["id"] == *id)
                && matches!(
                    rule_name(&d["rule"]),
                    Some("machine.clamping" | "machine.travel")
                )
        })
    });
    let finished = out.error.is_none()
        && !grip_omission
        && remaining <= 1e-6
        && operations
            .iter()
            .any(|o| matches!(o.cut, Cut::Roughing(_)) && matches!(o.tool, Some(Some(_))));
    let mut seconds = 0.;
    let mut tools = BTreeSet::new();
    let mut previous = None;
    let mut changes = 0;
    let rapid = settings
        .rapid_mm_min
        .filter(|r| *r > 0.)
        .unwrap_or(5000.);
    for program in &out.programs {
        for m in program["moves"].as_array().into_iter().flatten() {
            if let Some(tool) = m["tool"].as_str() {
                tools.insert(tool);
                if previous.is_some_and(|p| p != tool) {
                    changes += 1;
                }
                previous = Some(tool);
            }
            let rate = if m["kind"] == "rapid" {
                rapid
            } else {
                m["feed"].as_f64().filter(|r| *r > 0.).unwrap_or(rapid)
            };
            for points in m["points"]
                .as_array()
                .into_iter()
                .flat_map(|p| p.windows(2))
            {
                let squared = (0..3)
                    .map(|i| {
                        (points[0][i].as_f64().unwrap_or(0.) - points[1][i].as_f64().unwrap_or(0.))
                            .powi(2)
                    })
                    .sum::<f64>();
                seconds += squared.sqrt() / rate * 60.;
            }
        }
    }
    let mut risks = vec![];
    let grips: Vec<_>=p.setups.iter().filter_map(|s| {
        let grip=s.clamping.as_ref()?["grip_height_mm"].as_f64()?;
        let height=s.stock.max[2] - s.stock.min[2];
        let wall=s.flip_seating.as_ref().is_some_and(Value::is_object);
        risks.push(format!("{}: grips {grip} mm ({:.1}% of setup stock height){}",s.id,100.*grip/height,if wall {" on finished wall; soft jaws advised"} else {""}));
        let mut entry = json!({"setup":s.id,"height_mm":grip,"stock_fraction":grip/height,"finished_wall":wall});
        if let Some(force) = s.clamping.as_ref()?.get("force_check") { entry["force_check"] = force.clone(); }
        Some(entry)
    }).collect();
    risks.extend(p.warnings.iter().cloned());
    risks.push("Cited vise force is screened against estimated mean tangential cutting force with a safety factor; facing is stepped down when needed. Missing clamping force warns not checked. Contact pressure, lift, overturning, peak loads and support flatness are not simulated. Without modeled parallels, through cuts are not checked against supports or the bed.".into());
    let reason = if finished {
        "finished".into()
    } else if remaining > 1e-6 {
        format!("bottom {remaining} mm of outline left attached")
    } else if !left.is_empty() {
        format!("{} features left out", left.len())
    } else {
        "finished outline not established".into()
    };
    json!({"id":id,"label":label,"available":true,"finished":finished,"reason":reason,
        "outline_left_mm":remaining,"features_cut":cut,"covered_by_roughing":p.covered_by_roughing,"features_left_out":left,
        "setups":p.setups.len(),"extra_stock_mm":(stock_height(p)-ordinary_height).max(0.),
        "stock_height_mm":stock_height(p),"estimated_seconds":seconds,"tools":tools,"tool_changes":changes,"tools_needed":p.tools_needed,
        "grips":grips,"risks":risks,"shop_needs":["dimensioned vise","parallels or equivalent supports"]})
}

/// Potential screw clearance holes only; this does not validate plate workholding.
fn suitable_fixture_holes(analysis: &Value, mesh: &Value, ordinary: Option<&Plan>) -> bool {
    let threaded: BTreeSet<_> = ordinary
        .into_iter()
        .flat_map(|p| &p.setups)
        .flat_map(|s| &s.operations)
        .filter_map(|o| match &o.cut {
            Cut::Thread(thread) => Some(&thread.features),
            _ => None,
        })
        .flatten()
        .map(String::as_str)
        .collect();
    let holes: Vec<[f64; 2]> = analysis["features"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["family"] == "holes" && !threaded.contains(f["id"].as_str().unwrap_or("")))
        .filter_map(|f| {
            let r = &f["record"];
            (r["bottom"] == "through"
                && (3.2..=9.).contains(&r["diameter"].as_f64()?)
                && r["axis"][2].as_f64()?.abs() > 0.999)
                .then_some([r["location"][0].as_f64()?, r["location"][1].as_f64()?])
        })
        .collect();
    let diagonal = (0..2)
        .map(|axis| {
            let side = mesh["bbox"][1][axis].as_f64()? - mesh["bbox"][0][axis].as_f64()?;
            Some(side * side)
        })
        .collect::<Option<Vec<_>>>()
        .map(|ss| ss.iter().sum::<f64>().sqrt());
    diagonal.is_some_and(|d| {
        d > 0.
            && holes.iter().enumerate().any(|(i, a)| {
                holes[i + 1..]
                    .iter()
                    .any(|b| (a[0] - b[0]).hypot(a[1] - b[1]) >= 0.25 * d)
            })
    })
}

pub fn process(analysis: &Value, mesh: &Value, library: &Value) -> Result<Value, String> {
    process_using(analysis, mesh, library, |a, m, t, p| {
        super::process_planned(a, m, t, p, vec![])
    })
}

fn process_using(
    analysis: &Value,
    mesh: &Value,
    library: &Value,
    mut execute: impl FnMut(&Value, &Value, &Value, Plan) -> Result<Outcome, String>,
) -> Result<Value, String> {
    let settings = Settings::new(library)?;
    let mut run_library = library.clone();
    // Check workholding choices only after all independent runs.
    if let Some(ds) = run_library
        .get_mut("decisions")
        .and_then(Value::as_array_mut)
    {
        ds.retain(|s| rule_name(&s["rule"]) != Some(capp::candidates::RULE));
    }
    run_library["overrides"]["workholding_option"] = json!("none");
    let ordinary = capp::flip::build(analysis, mesh, &run_library);
    let ordinary_height = ordinary.as_ref().map(stock_height).unwrap_or(0.);
    let fixture_holes = suitable_fixture_holes(analysis, mesh, ordinary.as_ref().ok());
    let ordinary_result = ordinary.and_then(|p| execute(analysis, mesh, &run_library, p));
    let ordinary_error = ordinary_result.as_ref().err().cloned();
    let mut ordinary_run = ordinary_result.ok();
    let mut runs = vec![];
    let mut candidates = vec![];
    let vise = capp::machines::dimensioned_vise(library).is_some();
    if vise {
        let c = match &ordinary_run {
            None => unavailable(
                "vise_ordinary",
                "Vise, ordinary grip",
                ordinary_error.as_deref().unwrap(),
            ),
            Some(Outcome { error: Some(error), .. }) => unavailable(
                "vise_ordinary",
                "Vise, ordinary grip",
                error,
            ),
            Some(out) if !out.plan.setups.iter().all(|s| s.clamping.as_ref().is_some_and(Value::is_object)) => unavailable(
                "vise_ordinary",
                "Vise, ordinary grip",
                "stock cannot be held by the described vise; see machine/clamping checks",
            ),
            Some(out) => consequences(
                "vise_ordinary",
                "Vise, ordinary grip",
                out,
                ordinary_height,
                &settings,
            ),
        };
        candidates.push(c);
        runs.push(ordinary_run.take());
        run_library["overrides"]["workholding_option"] = json!("vise_flip_face");
        let flip = capp::flip::build(analysis, mesh, &run_library).and_then(|p| {
            if !is_flip(&p) {
                let reason = p
                    .decisions
                    .iter()
                    .flatten()
                    .rev()
                    .find(|d| rule_name(&d["rule"]) == Some("setup.flip_face"))
                    .and_then(|d| d["chosen"]["unavailable_reason"].as_str())
                    .unwrap_or("flip unavailable");
                return Err(reason.to_string());
            }
            execute(analysis, mesh, &run_library, p)
        });
        let (c, out) = match flip {
            Err(reason) => (
                unavailable("vise_flip_face", "Vise, flip and face", &reason),
                None,
            ),
            Ok(out) => {
                let c = if let Some(e) = &out.error {
                    unavailable("vise_flip_face", "Vise, flip and face", e)
                } else if !is_flip(&out.plan) {
                    unavailable(
                        "vise_flip_face",
                        "Vise, flip and face",
                        "execution lost the prepared grip or seating; ordinary fallback required",
                    )
                } else {
                    consequences(
                        "vise_flip_face",
                        "Vise, flip and face",
                        &out,
                        ordinary_height,
                        &settings,
                    )
                };
                (c, Some(out))
            }
        };
        candidates.push(c);
        runs.push(out);
    }
    let mut fixture = unavailable(
        "fixture_plate",
        "Fixture plate, screwed through holes",
        if fixture_holes {
            "not available yet: screw-head/tool clearance, supported seating and residual stock transfer are not implemented; fixture-plate holding cannot be simulated soundly"
        } else {
            "no suitable through holes: requires two unthreaded Z-axis through holes, 3.2–9 mm diameter, spread at least 25% of the XY bounding-box diagonal"
        },
    );
    fixture["short_reason"] = json!(if fixture_holes {
        "not available yet"
    } else {
        "no suitable through holes"
    });
    candidates.push(fixture);
    runs.push(None);
    candidates.push(unavailable(
        "margins_tabs",
        "Margins and tabs",
        "not available yet (#87)",
    ));
    runs.push(None);
    let chosen = capp::candidates::choose(&mut json!({}), library, &candidates)?;
    if let Some(best) = candidates
        .iter()
        .filter(|c| c["available"] == true)
        .min_by(|a, b| capp::candidates::compare(a, b))
        .cloned()
    {
        for (i, c) in candidates
            .iter_mut()
            .enumerate()
            .filter(|(_, c)| c["available"] == true)
        {
            c["comparison"] = json!(if Some(i) == chosen {
                "chosen"
            } else if c["id"] == best["id"] {
                "default rule's preferred option; supplied choice takes precedence"
            } else {
                capp::candidates::loses(c, &best)
            });
        }
    }
    let out = if let Some(i) = chosen {
        runs[i].take().unwrap()
    } else {
        // No validated holding: reuse the ordinary run, even if its holding was unavailable.
        let legacy = if vise {
            runs[0].take()
        } else {
            ordinary_run.take()
        };
        legacy.ok_or_else(|| {
            ordinary_error.unwrap_or_else(|| "no executable workholding candidate".into())
        })?
    };
    // The output is JSON from here on.
    let mut out = out.json();
    capp::candidates::choose(&mut out["plan"], library, &candidates)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs() -> (Value, Value, Value) {
        (
            serde_json::from_str(include_str!(
                "../../tests/fixtures/synthetic/stepped-plate.analysis.json"
            ))
            .unwrap(),
            serde_json::from_str(include_str!(
                "../../tests/fixtures/synthetic/stepped-plate.mesh.json"
            ))
            .unwrap(),
            serde_json::from_str(include_str!("../../demo/tools.json")).unwrap(),
        )
    }
    #[test]
    fn a_candidate_execution_error_does_not_discard_the_other_run() {
        let (a, m, t) = inputs();
        for fail_flip in [false, true] {
            let mut calls = 0;
            let out = process_using(&a, &m, &t, |a, m, t, p| {
                calls += 1;
                if is_flip(&p) == fail_flip {
                    Err("forced candidate execution error".into())
                } else {
                    super::super::process_planned(a, m, t, p, vec![])
                }
            })
            .unwrap();
            assert_eq!(calls, 2);
            assert!(out["error"].is_null());
            assert!(!out["programs"].as_array().unwrap().is_empty());
            let (failed, survivor) = if fail_flip {
                ("vise_flip_face", "vise_ordinary")
            } else {
                ("vise_ordinary", "vise_flip_face")
            };
            assert_eq!(out["plan"]["workholding"]["chosen"], survivor);
            let failed = out["plan"]["workholding"]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == failed)
                .unwrap();
            assert_eq!(failed["available"], false);
            assert_eq!(failed["reason"], "forced candidate execution error");
        }
    }
    #[test]
    fn unavailable_vise_reuses_the_single_ordinary_run() {
        let (a, m, mut t) = inputs();
        t["machines"][0]["workholding"][0]["max_jaw_opening_mm"] = json!(1.);
        let mut calls = 0;
        let out = process_using(&a, &m, &t, |a, m, t, p| {
            calls += 1;
            super::super::process_planned(a, m, t, p, vec![])
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(out["plan"]["workholding"]["chosen"].is_null());
        assert!(out["error"].is_null());
    }
    #[test]
    fn grip_and_travel_omissions_ignore_rule_version() {
        for rule in ["machine.clamping@99", "machine.travel@7"] {
            let out = Outcome {error:None,programs:vec![],log:vec![],plan:serde_json::from_value(json!({
                "setups":[{"id":"top","stock":{"min":[0,0,-10],"max":[10,10,0]},"operations":[{"kind":"roughing","tool":"em","web_mm":0,"top_z":0,"floor_z":-10,"surface_error_mm":0}]}],
                "skipped":[{"feature":"f","reason":"not held","decision":"d-1"}],"decisions":[{"id":"d-1","rule":rule}],
                "covered_by_roughing":[],"tools_needed":[],"warnings":[]})).unwrap()};
            let c = consequences(
                "vise_ordinary",
                "Vise, ordinary grip",
                &out,
                10.,
                &Settings::new(&json!({})).unwrap(),
            );
            assert_eq!(c["finished"], false, "{rule}");
        }
    }
    #[test]
    fn fixture_hole_preconditions_require_clearance_spread_and_no_threads() {
        let hole = |id: &str, x: f64| json!({"id":id,"family":"holes","record":{"bottom":"through","diameter":4.4,"axis":[0,0,-1],"location":[x,0,10]}});
        let a = json!({"features":[hole("a",0.),hole("b",50.)]});
        let m = json!({"bbox":[[0,0,0],[100,100,10]]});
        assert!(suitable_fixture_holes(&a, &m, None));
        for (key, value) in [
            ("bottom", json!("blind")),
            ("diameter", json!(2.5)),
            ("axis", json!([1, 0, 0])),
            ("location", json!([1, 0, 10])),
        ] {
            let mut bad = a.clone();
            bad["features"][1]["record"][key] = value;
            assert!(!suitable_fixture_holes(&bad, &m, None), "{key}");
        }
        let threaded: Plan = serde_json::from_value(json!({"setups":[{"id":"top","stock":{"min":[0,0,0],"max":[100,100,10]},"operations":[{"kind":"thread","features":["a"],
            "spec":"M5","major":5,"pitch":0.8,"start_diameter":4.2,"top_z":10,"bottom_z":0,"through":true,"centers":[[0,0]]}]}],
            "covered_by_roughing":[],"skipped":[],"warnings":[]})).unwrap();
        assert!(!suitable_fixture_holes(&a, &m, Some(&threaded)));
    }
}
