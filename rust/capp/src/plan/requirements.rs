//! What the drawing asks for that the plan does not do: one warning per requirement in Specify's
//! answers that no rule acts on. The material, threads and clearance holes are acted on; datums
//! and how holes are located are choices, not requirements. Answers to questions specify-core did
//! not ask, given the other answers, write nothing on the drawing and are not warned of.

use super::*;

/// specify-core's answers that ask for nothing (its `NONE`, `GENERAL` and notes that say there is
/// nothing to note).
fn nothing(value: &str) -> bool {
    ["", "none", "general", "none (as machined)", "as machined", "no edge note"].contains(&value.trim().to_lowercase().as_str())
}

/// A hole, hole pattern or turned diameter in words (`the 4 ⌀4.4 holes`, `the ⌀10 hole`), and
/// the ids of the features it is: a pattern's member holes, else itself.
fn named(features: &[Value], id: &str) -> (String, Vec<String>) {
    let Some(feature) = features.iter().find(|f| f["id"] == id) else { return (id.to_string(), vec![id.to_string()]) };
    let members: Vec<String> = feature["members"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
    let first = members.first().and_then(|m| features.iter().find(|f| f["id"] == m.as_str()));
    let Some(d) = feature["record"]["diameter"].as_f64().or_else(|| first.and_then(|f| f["record"]["diameter"].as_f64())) else {
        return (id.to_string(), vec![id.to_string()]);
    };
    let d = display_position(d);
    match (feature["family"].as_str(), members.len()) {
        (Some("holes"), _) => (format!("the ⌀{d} hole"), vec![id.to_string()]),
        (Some("hole_patterns"), n) if n > 0 => (format!("the {n} ⌀{d} holes"), members),
        _ => (format!("the ⌀{d} diameter"), vec![id.to_string()]),
    }
}

/// Warns, under `requirement.not_acted_on`, of each requirement the plan does not act on.
pub(super) fn not_acted_on(analysis: &Value, trace: &mut Trace) {
    const RULE: &str = "requirement.not_acted_on";
    let answers = &analysis["resume"]["answers"];
    let Some(all) = answers.as_object() else { return };
    let features: &[Value] = analysis["features"].as_array().map_or(&[], Vec::as_slice);
    let text = |key: &str| answers[key].as_str().filter(|v| !nothing(v));
    // specify-core asks no datum flatness or control, position or runout under "± dimensions",
    // and no ± location under "position tolerances"; callers keep both modes' answers.
    let locating = answers["part.locating"].as_str().unwrap_or("").trim().to_lowercase();
    let plus_minus = locating == "± dimensions";
    for (key, what, not) in [
        ("part.general_tolerance", "general tolerance", "not checked"),
        ("part.surface_finish", "surface finish", "not checked"),
        ("part.edges", "edge note", "not planned (break edges by hand)"),
        ("part.coating", "coating", "not planned"),
        ("part.heat_treatment", "heat treatment", "not planned"),
    ] {
        if let Some(value) = text(key) {
            let value = if key == "part.edges" { format!("\"{value}\"") } else { value.to_string() };
            trace.warn(RULE, "part", format!("{what} {value}: {not}"), json!({"requirements": [key]}));
        }
    }
    for note in answers["part.notes"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|n| !nothing(n)) {
        trace.warn(RULE, "part", format!("note \"{note}\": not acted on"), json!({"requirements": ["part.notes"]}));
    }
    for letter in "ABCDEFGH".chars() {
        let faces: Vec<String> = answers[format!("datum.{letter}")].as_array().into_iter().flatten().map(Value::to_string).collect();
        let on = if faces.is_empty() { format!("datum {letter}") } else { format!("datum {letter} (face {})", faces.join(", ")) };
        let key = format!("datum.{letter}.flatness");
        if let Some(value) = text(&key).filter(|_| !plus_minus) {
            trace.warn(RULE, &format!("datum {letter}"), format!("flatness {value} on {on}: not checked"), json!({"requirements": [key]}));
        }
        let key = format!("datum.{letter}.control");
        if let Some(value) = text(&key).filter(|_| !plus_minus) {
            // specify-core writes the control it chose (perpendicularity or position) on the
            // datum's faces; its answer does not say which.
            let kind = analysis["characteristics"].as_array().into_iter().flatten()
                .find(|c| c["kind"] == "geometric_tolerance" && c["type"] != "Flatness" && !faces.is_empty()
                    && c["faces"].as_array().is_some_and(|f| f.iter().map(Value::to_string).eq(faces.iter().cloned())))
                .and_then(|c| c["type"].as_str())
                .map_or("control".to_string(), str::to_lowercase);
            trace.warn(RULE, &format!("datum {letter}"), format!("{kind} {value} on {on}: not checked"), json!({"requirements": [key]}));
        }
    }
    // Fits, threads and knurls on holes and turned diameters.
    for (key, value) in all {
        let Some(value) = value.as_str().filter(|v| !nothing(v)) else { continue };
        let (target, on_hole) = match (key.strip_prefix("hole.function:"), key.strip_prefix("diameter.fit:")) {
            (Some(t), _) => (t, true),
            (_, Some(t)) => (t, false),
            _ => continue,
        };
        let (kind, spec) = value.split_once(':').unwrap_or(("fit", value));
        let (on, ids) = named(features, target);
        let says = match kind {
            // Acted on: threaded, or cut to the drawn clearance size.
            "tapped" | "clearance" if on_hole => continue,
            "fit" if on_hole => format!("fit {spec} on {on}: not held: cut to the drawn size with no finishing pass, ream or bore"),
            "fit" => format!("fit {spec} on {on}: not held: no finishing pass is planned"),
            _ => format!("{kind} {spec} on {on}: not planned"),
        };
        // Subject the part, not the hole: a hole's warning would be borrowed by every operation on
        // it, its spot drill included.
        trace.warn(RULE, "part", says, json!({"features": ids, "requirements": [key]}));
    }
    // Position, ± location and runout tolerances: one warning each, listing where. The page
    // would put every operation on those features before it, so it names none.
    for (prefix, what, sign) in [("hole.position:", "position", "⌀"), ("hole.location:", "location", ""), ("diameter.runout:", "runout", "")] {
        let unasked = if prefix == "hole.location:" { locating == "position tolerances" } else { plus_minus };
        if unasked {
            continue;
        }
        let (mut listed, mut keys) = (vec![], vec![]);
        for (key, value) in all {
            let (Some(target), Some(value)) = (key.strip_prefix(prefix), value.as_str().filter(|v| !nothing(v))) else { continue };
            let function = answers[format!("hole.function:{target}")].as_str();
            // specify-core asks no position of a hole whose function is general, and writes none
            // (its write report lists such an answer as not written).
            if prefix == "hole.position:" && function.is_some_and(nothing) {
                continue;
            }
            let purpose = function.filter(|f| !nothing(f)).map_or(String::new(), |f| format!(" ({})", f.replacen(':', " ", 1)));
            listed.push(format!("{sign}{value} on {}{purpose}", named(features, target).0));
            keys.push(key.as_str());
        }
        if !listed.is_empty() {
            trace.warn(RULE, "part", format!("{what} not checked: {}", listed.join("; ")), json!({"requirements": keys}));
        }
    }
    // Requirements added in Specify beyond its questions.
    for (key, extra) in all.iter().filter(|(k, v)| k.starts_with("extra.") && v.is_object()) {
        let kind = extra["kind"].as_str().unwrap_or("requirement");
        let faces: Vec<String> = extra["faces"].as_array().into_iter().flatten().map(Value::to_string).collect();
        trace.warn(RULE, "part", format!("{kind} on face {}: not checked", faces.join(", ")), json!({"requirements": [key]}));
    }
}
