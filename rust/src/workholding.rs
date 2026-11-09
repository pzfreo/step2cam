//! Full candidate runs, including camlib retries. Never infer one option's cost from another.
use serde_json::{Value, json};
use std::collections::BTreeSet;

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
                    d["rule"].as_str(),
                    Some("machine.clamping@1" | "machine.travel@1")
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
        "grips":grips,"risks":risks,"shop_needs":["dimensioned vise","parallels or equivalent supports"],
        "simulation":"camlib passed; see chosen run for detailed checks"})
}

pub fn process(analysis: &Value, mesh: &Value, library: &Value) -> Result<Value, String> {
    let mut run_library = library.clone();
    // The candidate decision is checked after all runs, not refused by per-operation finish.
    if let Some(ds) = run_library
        .get_mut("decisions")
        .and_then(Value::as_array_mut)
    {
        ds.retain(|s| {
            !s["rule"]
                .as_str()
                .is_some_and(|r| r.split('@').next() == Some(capp::candidates::RULE))
        });
    }
    run_library["overrides"]["workholding_option"] = json!("none");
    let ordinary = capp::flip::plan(analysis, mesh, &run_library)?;
    let ordinary_height = stock_height(&ordinary);
    let mut runs = vec![];
    let mut candidates = vec![];
    let vise = capp::machines::mill(library)
        .and_then(|m| m["workholding"].as_array())
        .and_then(|ws| ws.iter().find(|w| w["kind"] == "vise"))
        .is_some_and(|v| {
            ["jaw_width_mm", "jaw_height_mm", "max_jaw_opening_mm"]
                .iter()
                .all(|k| v[*k].as_f64().is_some_and(|n| n.is_finite() && n > 0.))
        });
    if vise {
        let out = super::process_planned(analysis, mesh, &run_library, ordinary.clone(), vec![])?;
        let mut c = consequences(
            "vise_ordinary",
            "Vise, ordinary grip",
            &out,
            ordinary_height,
            library,
        );
        if let Some(e) = out["error"].as_str() {
            c = unavailable("vise_ordinary", "Vise, ordinary grip", e);
        } else if !setups(&out["plan"]).all(|s| s["clamping"].is_object()) {
            c = unavailable(
                "vise_ordinary",
                "Vise, ordinary grip",
                "stock cannot be held by the described vise; see machine/clamping checks",
            );
        }
        candidates.push(c);
        runs.push(Some(out));
        run_library["overrides"]["workholding_option"] = json!("vise_flip_face");
        let planned = capp::flip::plan(analysis, mesh, &run_library)?;
        if is_flip(&planned) {
            let out = super::process_planned(analysis, mesh, &run_library, planned, vec![])?;
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
            candidates.push(c);
            runs.push(Some(out));
        } else {
            let reason = planned["decisions"]
                .as_array()
                .into_iter()
                .flatten()
                .rev()
                .find(|d| d["rule"] == "setup.flip_face@3")
                .and_then(|d| d["says"].as_str())
                .unwrap_or("flip unavailable");
            candidates.push(unavailable("vise_flip_face", "Vise, flip and face", reason));
            runs.push(None);
        }
    }
    candidates.push(unavailable("fixture_plate","Fixture plate, screwed through holes","not available yet: screw-head/tool clearance, supported seating and residual stock transfer are not implemented; fixture-plate holding cannot be simulated soundly"));
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
        // Preserve the reviewable legacy plan when there is no validated holding option.
        run_library["overrides"]["workholding_option"] = json!("none");
        super::process_planned(analysis, mesh, &run_library, ordinary, vec![])?
    };
    capp::candidates::choose(&mut out["plan"], library, &candidates)?;
    Ok(out)
}
