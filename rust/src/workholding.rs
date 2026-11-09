//! Full candidate runs, including camlib retries. Never infer one option's cost from another.
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn rule_name(value: &Value) -> Option<&str> {
    value.as_str()?.split('@').next()
}
fn setups(plan: &Value) -> impl Iterator<Item = &Value> {
    plan["setups"].as_array().into_iter().flatten()
}
fn is_flip(plan: &Value) -> bool {
    setups(plan).any(|s| s["flip_seating"].is_object())
}
fn stock_height(plan: &Value) -> f64 {
    setups(plan).next().map_or(0., |s| {
        s["stock"]["max"][2].as_f64().unwrap_or(0.) - s["stock"]["min"][2].as_f64().unwrap_or(0.)
    })
}
fn unavailable(id: &str, label: &str, reason: &str) -> Value {
    json!({"id":id,"label":label,"available":false,"reason":reason})
}
fn consequences(
    id: &str,
    label: &str,
    out: &Value,
    ordinary_height: f64,
    library: &Value,
) -> Value {
    let p = &out["plan"];
    let operations: Vec<_> = setups(p)
        .flat_map(|s| s["operations"].as_array().into_iter().flatten())
        .collect();
    let omitted: BTreeSet<_> = p["skipped"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["feature"].as_str())
        .collect();
    let left: Vec<_>=omitted.iter().map(|id|json!({"feature":id,"reasons":p["skipped"].as_array().into_iter().flatten().filter(|s|s["feature"]==**id).map(|s|s["reason"].clone()).collect::<Vec<_>>()})).collect();
    let cut: BTreeSet<_> = operations
        .iter()
        .filter(|o| o["tool"].is_string())
        .flat_map(|o| o["features"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    let remaining = operations
        .iter()
        .filter(|o| o["kind"] == "roughing")
        .filter_map(|o| o["web_mm"].as_f64())
        .fold(0_f64, f64::max);
    let grip_omission = p["skipped"].as_array().into_iter().flatten().any(|s| {
        p["decisions"].as_array().into_iter().flatten().any(|d| {
            d["id"] == s["decision"]
                && matches!(
                    rule_name(&d["rule"]),
                    Some("machine.clamping" | "machine.travel")
                )
        })
    });
    let finished = out["error"].is_null()
        && !grip_omission
        && remaining <= 1e-6
        && operations
            .iter()
            .any(|o| o["kind"] == "roughing" && o["tool"].is_string());
    let mut seconds = 0.;
    let mut tools = BTreeSet::new();
    let mut previous = None;
    let mut changes = 0;
    let rapid = library["rapid_mm_min"]
        .as_f64()
        .filter(|r| *r > 0.)
        .unwrap_or(5000.);
    for program in out["programs"].as_array().into_iter().flatten() {
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
    let grips: Vec<_>=setups(p).filter_map(|s| {
        let grip=s["clamping"]["grip_height_mm"].as_f64()?;
        let height=s["stock"]["max"][2].as_f64()? - s["stock"]["min"][2].as_f64()?;
        let wall=s["flip_seating"].is_object();
        risks.push(format!("{}: grips {grip} mm ({:.1}% of setup stock height){}",s["id"].as_str().unwrap_or("setup"),100.*grip/height,if wall {" on finished wall; soft jaws advised"} else {""}));
        Some(json!({"setup":s["id"],"height_mm":grip,"stock_fraction":grip/height,"finished_wall":wall}))
    }).collect();
    risks.extend(
        p["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(String::from),
    );
    risks.push("Holding force and support flatness are not simulated. Without modeled parallels, through cuts are not checked against supports or the bed.".into());
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
        "outline_left_mm":remaining,"features_cut":cut,"covered_by_roughing":p["covered_by_roughing"],"features_left_out":left,
        "setups":setups(p).count(),"extra_stock_mm":(stock_height(p)-ordinary_height).max(0.),
        "stock_height_mm":stock_height(p),"estimated_seconds":seconds,"tools":tools,"tool_changes":changes,"tools_needed":p["tools_needed"],
        "grips":grips,"risks":risks,"shop_needs":["dimensioned vise","parallels or equivalent supports"]})
}

/// Potential screw clearance holes only; this does not validate plate workholding.
fn suitable_fixture_holes(analysis: &Value, mesh: &Value, ordinary: Option<&Value>) -> bool {
    let threaded: BTreeSet<_> = ordinary
        .into_iter()
        .flat_map(setups)
        .flat_map(|s| s["operations"].as_array().into_iter().flatten())
        .filter(|o| matches!(o["kind"].as_str(), Some("thread" | "tap")))
        .flat_map(|o| o["features"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
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
    mut execute: impl FnMut(&Value, &Value, &Value, Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let mut run_library = library.clone();
    // Check workholding choices only after all independent runs.
    if let Some(ds) = run_library
        .get_mut("decisions")
        .and_then(Value::as_array_mut)
    {
        ds.retain(|s| rule_name(&s["rule"]) != Some(capp::candidates::RULE));
    }
    run_library["overrides"]["workholding_option"] = json!("none");
    let ordinary = capp::flip::plan(analysis, mesh, &run_library);
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
            Some(out) if out["error"].is_string() => unavailable(
                "vise_ordinary",
                "Vise, ordinary grip",
                out["error"].as_str().unwrap(),
            ),
            Some(out) if !setups(&out["plan"]).all(|s| s["clamping"].is_object()) => unavailable(
                "vise_ordinary",
                "Vise, ordinary grip",
                "stock cannot be held by the described vise; see machine/clamping checks",
            ),
            Some(out) => consequences(
                "vise_ordinary",
                "Vise, ordinary grip",
                out,
                ordinary_height,
                library,
            ),
        };
        candidates.push(c);
        runs.push(ordinary_run.take());
        run_library["overrides"]["workholding_option"] = json!("vise_flip_face");
        let flip = capp::flip::plan(analysis, mesh, &run_library).and_then(|p| {
            if !is_flip(&p) {
                let reason = p["decisions"]
                    .as_array()
                    .into_iter()
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
                let c = if let Some(e) = out["error"].as_str() {
                    unavailable("vise_flip_face", "Vise, flip and face", e)
                } else if !is_flip(&out["plan"]) {
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
                        library,
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
    let mut out = if let Some(i) = chosen {
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
            let out = json!({"error":null,"programs":[],"plan":{
                "setups":[{"stock":{"min":[0,0,-10],"max":[10,10,0]},"operations":[{"kind":"roughing","tool":"em","web_mm":0}]}],
                "skipped":[{"feature":"f","reason":"not held","decision":"d-1"}],"decisions":[{"id":"d-1","rule":rule}],
                "covered_by_roughing":[],"tools_needed":[],"warnings":[]}});
            let c = consequences(
                "vise_ordinary",
                "Vise, ordinary grip",
                &out,
                10.,
                &json!({}),
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
        let threaded = json!({"setups":[{"operations":[{"kind":"thread","features":["a"]}]}]});
        assert!(!suitable_fixture_holes(&a, &m, Some(&threaded)));
    }
}
