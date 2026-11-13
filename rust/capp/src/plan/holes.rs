//! Holes: their setup-frame geometry, threads asked for on them, and their drilling,
//! counterbore and thread operations.

use super::*;

struct Entry {
    setup: &'static str,
    axis: V3,
    mouth: V3,
    reversed: bool,
    steps: Vec<(f64, f64)>,
}

/// The enlargements between a hole's opening and its bore, widest (outermost) first.
fn steps_of(record: &Value) -> Result<Vec<(f64, f64)>, String> {
    let mut steps: Vec<&Value> = record.get("outer_steps").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    for key in ["cbore", "spotface"] {
        if truthy(record.get(key)) {
            steps.push(&record[key]);
        }
    }
    let mut out = steps.iter().map(|s| Ok((f(s, "diameter")?, f(s, "depth")?))).collect::<Result<Vec<_>, String>>()?;
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(out)
}

/// A counterbore step: (diameter, top Z, floor Z).
type Enlargement = (f64, f64, f64);

pub(super) struct Hole {
    pub(super) feature: String,
    pub(super) center: model::Point2,
    pub(super) top_z: f64,
    pub(super) bore_top_z: f64,
    bottom_z: f64,
    pub(super) diameter: f64,
    bottom: String,
    /// For a through hole, how far below its exit the tool may go (`None`: unrestricted).
    exit_clear: Option<f64>,
    enlargements: Vec<Enlargement>,
    pub(super) csink: Value,
    thread: Option<Tapped>,
}

/// The thread a hole gets: its size, and where it runs in the setup.
struct Tapped {
    spec: String,
    major: f64,
    pitch: f64,
    top_z: f64,
    bottom_z: f64,
}

/// An internal thread asked for on a hole.
#[derive(Clone)]
pub(super) struct Thread {
    pub(super) spec: String,
    major: f64,
    pitch: f64,
    full_depth: Option<f64>,
}

/// (major diameter, pitch) of a metric thread spec (`M5`, `M10x1.25`).
fn thread_size(spec: &str) -> Option<(f64, f64)> {
    let (size, fine) = spec.split_once('x').map_or((spec, None), |(a, b)| (a, Some(b)));
    let major: f64 = size.strip_prefix('M')?.parse().ok()?;
    let pitch = match fine {
        Some(text) => text.parse().ok()?,
        None => crate::reference::coarse_pitch(size)?,
    };
    (major > 0. && pitch > 0. && pitch < major).then_some((major, pitch))
}

/// The internal threads asked for, by hole feature id: from Specify's answers
/// (`hole.function:<hole or pattern>` = `tapped:M5` or `tapped:M5@5.5`). Threads another tool wrote,
/// which state no size, and answers that cannot be used become warnings.
pub(super) fn threads(analysis: &Value, trace: &mut Trace) -> HashMap<String, Thread> {
    let features: Vec<&Value> = analysis["features"].as_array().into_iter().flatten().collect();
    let holes: Vec<&&Value> = features.iter().filter(|f| f["family"] == "holes").collect();
    let faces_of = |f: &Value| -> Vec<u64> { f["faces"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect() };
    let mut found = HashMap::new();
    for (key, value) in analysis["resume"]["answers"].as_object().into_iter().flatten() {
        let (Some(target), Some(answer)) = (key.strip_prefix("hole.function:"), value.as_str().and_then(|v| v.strip_prefix("tapped:"))) else {
            continue;
        };
        let (spec, full) = answer.split_once('@').map_or((answer, None), |(a, b)| (a, Some(b)));
        let Some((major, pitch)) = thread_size(spec) else {
            trace.warn("requirement.thread.unknown_size", target, format!("{target}: tapped {spec} is not a metric size this bridge knows; not threaded"), json!({"features": [target]}));
            continue;
        };
        let full_depth = match full.filter(|text| !text.is_empty()).map(str::parse::<f64>) {
            None => None,
            Some(Ok(depth)) => Some(depth),
            Some(Err(_)) => {
                trace.warn("requirement.thread.unreadable_depth", target, format!("{target}: tapped '{}' has an unreadable depth; not threaded", value.as_str().unwrap_or("")), json!({"features": [target]}));
                continue;
            }
        };
        let Some(feature) = features.iter().find(|f| f["id"].as_str() == Some(target)) else {
            trace.warn("requirement.thread.no_feature", target, format!("{target}: tapped {spec}, but the part has no such feature now; not threaded"), json!({"features": [target]}));
            continue;
        };
        let thread = Thread { spec: spec.to_string(), major, pitch, full_depth };
        if feature["family"] == "holes" {
            found.insert(target.to_string(), thread);
        } else {
            // A hole pattern or set: the holes whose faces it is made of.
            let own = faces_of(feature);
            for hole in &holes {
                let faces = faces_of(hole);
                if !faces.is_empty() && faces.iter().all(|x| own.contains(x)) {
                    found.insert(hole["id"].as_str().unwrap_or("").to_string(), thread.clone());
                }
            }
        }
    }
    for item in analysis["existing"].as_array().into_iter().flatten() {
        if item["type"] != "internal thread" {
            continue;
        }
        let on = faces_of(item);
        for hole in &holes {
            let id = hole["id"].as_str().unwrap_or("");
            if !found.contains_key(id) && faces_of(hole).iter().any(|x| on.contains(x)) {
                trace.warn("requirement.thread.no_size", id, format!("{id}: the file marks an internal thread here but states no size; not threaded (answer it in Specify to thread it)"), json!({"features": [id]}));
            }
        }
    }
    found
}

/// A hole placed in a setup.
pub(super) struct Placed {
    pub(super) setup: &'static str,
    pub(super) hole: Hole,
    /// Warnings as (rule, text).
    pub(super) warnings: Vec<(&'static str, String)>,
    /// Setups it lies along that were rejected because part material stands above its opening.
    pub(super) blocked: Vec<&'static str>,
}

/// A hole no setup can cut: the rule that refused it, why, and what the why ends by saying is
/// needed (after `: needs `), if anything.
pub(super) struct Refused {
    pub(super) rule: &'static str,
    pub(super) reason: String,
    pub(super) needs: Option<&'static str>,
}

/// A hole as setup-frame geometry, or why it is skipped. The outer error stops planning.
pub(super) fn hole_geometry(feature: &Value, cx: &Context, thread: Option<&Thread>) -> Result<Result<Placed, Refused>, String> {
    let record = &feature["record"];
    let id = feature["id"].as_str().unwrap_or("").to_string();
    let axis0 = unit(v3(&record["axis"])?)?;
    let mouth0 = v3(&record["location"])?;
    let depth = f(record, "depth")?;
    let diameter = f(record, "diameter")?;
    let plain = !["cbore", "spotface", "csink", "outer_steps"].iter().any(|k| truthy(record.get(*k)));
    let mut entries = vec![(axis0, mouth0, false)];
    if record["bottom"].as_str() == Some("through") && plain && !truthy(record.get("exit_obstructions")) {
        entries.push((scale(axis0, -1.), add(mouth0, scale(axis0, depth)), true));
    }
    let mut options: Vec<Entry> = vec![];
    let mut blocked: Vec<&str> = vec![];
    for (axis, mouth, reversed) in entries {
        let Some(setup) = setup_for(axis)? else { continue };
        let steps = if reversed { vec![] } else { steps_of(record)? };
        let widest = steps.iter().map(|s| s.0).fold(diameter, f64::max);
        if let Some(mesh) = cx.triangles {
            let frame = &cx.frames[setup];
            let at = frame.point(mouth);
            if blocked_above(frame.triangles(mesh), (at[0], at[1]), at[2], widest / 2.) {
                blocked.push(setup);
                continue;
            }
        }
        options.push(Entry { setup, axis, mouth, reversed, steps });
    }
    if options.is_empty() {
        if !blocked.is_empty() {
            return Ok(Err(Refused { rule: "setup.hole.blocked_above", reason: format!(
                "part material above its opening from {}: a three-axis tool cannot reach it",
                blocked.join(", ")
            ), needs: None }));
        }
        let axis = v3(&record["axis"])?;
        let needs = "a setup this bridge does not make";
        return Ok(Err(Refused { rule: "setup.hole.oblique", reason: format!(
            "oblique axis [{}, {}, {}]: needs {needs}",
            show(r(axis[0])), show(r(axis[1])), show(r(axis[2]))
        ), needs: Some(needs) }));
    }
    let rank = |name: &str| cx.preferred.iter().position(|p| *p == name).unwrap_or(usize::MAX);
    let mut best = 0;
    for (i, o) in options.iter().enumerate() {
        if rank(o.setup) < rank(options[best].setup) {
            best = i;
        }
    }
    let chosen = &options[best];
    let name = chosen.setup;
    let frame = &cx.frames[name];

    // `location` is the axis point at the opening; `diameter`/`depth` describe the narrowest bore,
    // `depth` from the top of that bore to the deep end.
    let mut enlargements = vec![];
    let mut at = chosen.mouth;
    for (step_d, step_depth) in &chosen.steps {
        let floor = add(at, scale(chosen.axis, *step_depth));
        enlargements.push((r(*step_d), r(frame.point(at)[2]), r(frame.point(floor)[2])));
        at = floor;
    }
    let bore_top = at;
    let bottom = frame.point(add(bore_top, scale(chosen.axis, depth)));
    let centre = frame.point(chosen.mouth);
    let bottom_kind = record.get("bottom").and_then(Value::as_str).unwrap_or("unknown").to_string();
    let mut hole = Hole {
        feature: id.clone(),
        center: [r(centre[0]), r(centre[1])],
        top_z: r(centre[2]),
        bore_top_z: r(frame.point(bore_top)[2]),
        bottom_z: r(bottom[2]),
        diameter: r(diameter),
        bottom: bottom_kind.clone(),
        exit_clear: None,
        enlargements,
        csink: Value::Null,
        thread: None,
    };
    let mut warnings = vec![];
    if bottom_kind == "through" {
        let part_bottom = cx.part_faces[name];
        let on_face = (bottom[2] - part_bottom).abs() <= LEVEL_EPS;
        let clear = match cx.triangles {
            Some(mesh) => clear_below(frame.triangles(mesh), (bottom[0], bottom[1]), bottom[2], diameter / 2.),
            None => if on_face { None } else { Some(0.) },
        };
        hole.exit_clear = clear.map(r);
        if !on_face {
            warnings.push(("process.hole.short_of_far_face", format!(
                "{id}: the through hole ends {} mm short of the part's far face in the {name} setup; \
                 anything between (a counterbore or chamfer at the far end, a shoulder) is not planned",
                show(r(bottom[2] - part_bottom))
            )));
        }
    }
    if let Some(thread) = thread {
        // The thread runs down the bore from its top: all of it for a through hole, else the
        // minimum full thread asked for (specify-core's default when none was).
        let (top, mut end) = (hole.bore_top_z, hole.bottom_z);
        let length = top - end;
        let mut fits = true;
        if hole.bottom != "through" {
            let full = thread.full_depth.filter(|d| *d != 0.).unwrap_or_else(|| crate::reference::full_thread(depth, thread.pitch));
            if full > length + 1e-9 {
                warnings.push(("requirement.thread.too_deep", format!("{id}: {} thread {} mm deep is deeper than the hole ({} mm); not threaded", thread.spec, show(full), show(r(length)))));
                fits = false;
            } else {
                end = top - full;
            }
        }
        if fits {
            hole.thread = Some(Tapped { spec: thread.spec.clone(), major: r(thread.major), pitch: thread.pitch, top_z: r(top), bottom_z: r(end) });
        }
    }
    hole.csink = if !chosen.reversed && truthy(record.get("csink")) { record["csink"].clone() } else { Value::Null };
    Ok(Ok(Placed { setup: name, hole, warnings, blocked }))
}

/// Holes of one diameter, depth and ending as one operation over their centres, in the order of
/// those values.
pub(super) fn hole_operations(holes: &[Hole]) -> Vec<model::Cut> {
    struct Group<'a> {
        members: Vec<&'a Hole>,
    }
    let same = |a: &Hole, b: &Hole| {
        a.diameter == b.diameter && a.top_z == b.top_z && a.bore_top_z == b.bore_top_z && a.bottom_z == b.bottom_z
            && a.bottom == b.bottom && a.exit_clear == b.exit_clear
    };
    let mut groups: Vec<Group> = vec![];
    for h in holes {
        match groups.iter_mut().find(|g| same(g.members[0], h)) {
            Some(g) => g.members.push(h),
            None => groups.push(Group { members: vec![h] }),
        }
    }
    let clear_key = |h: &Hole| h.exit_clear.unwrap_or(-1e18);
    groups.sort_by(|a, b| {
        let (x, y) = (a.members[0], b.members[0]);
        x.diameter.total_cmp(&y.diameter)
            .then(x.top_z.total_cmp(&y.top_z))
            .then(x.bore_top_z.total_cmp(&y.bore_top_z))
            .then(x.bottom_z.total_cmp(&y.bottom_z))
            .then(x.bottom.cmp(&y.bottom))
            .then(clear_key(x).total_cmp(&clear_key(y)))
    });
    groups
        .iter()
        .map(|g| {
            let h = g.members[0];
            model::Cut::Holes(model::Holes {
                diameter: h.diameter, top_z: h.top_z, bore_top_z: h.bore_top_z,
                bottom_z: h.bottom_z, bottom: h.bottom.clone(),
                centers: g.members.iter().map(|m| m.center).collect(),
                features: g.members.iter().map(|m| m.feature.clone()).collect(),
                exit_clear_mm: (h.bottom == "through").then_some(h.exit_clear),
                countersinks: g.members.iter().any(|h| !h.csink.is_null())
                    .then(|| g.members.iter().map(|h| Some(h.csink.clone()).filter(|c| !c.is_null())).collect()),
            })
        })
        .collect()
}

pub(super) fn counterbore_operations(holes: &[Hole]) -> Vec<model::Cut> {
    let mut groups: Vec<(Enlargement, Vec<model::Point2>, Vec<String>)> = vec![];
    for h in holes {
        for e in &h.enlargements {
            match groups.iter_mut().find(|g| g.0 == *e) {
                Some(g) => {
                    g.1.push(h.center);
                    g.2.push(h.feature.clone());
                }
                None => groups.push((*e, vec![h.center], vec![h.feature.clone()])),
            }
        }
    }
    groups.sort_by(|a, b| a.0.0.total_cmp(&b.0.0).then(a.0.1.total_cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2)));
    groups
        .into_iter()
        .map(|(e, centers, features)| {
            model::Cut::Counterbore(model::Counterbore { diameter: e.0, top_z: e.1, floor_z: e.2, centers, features })
        })
        .collect()
}

/// What threaded holes must share to be one thread operation; operations come out in the
/// order of these fields.
#[derive(PartialEq)]
struct ThreadKey {
    spec: String,
    major: f64,
    pitch: f64,
    start_diameter: f64,
    top_z: f64,
    bottom_z: f64,
    through: bool,
    exit_clear: Option<f64>,
}

impl ThreadKey {
    /// Field by field, floats by `total_cmp`, an unrestricted exit last.
    fn order(&self, other: &Self) -> std::cmp::Ordering {
        let clear = |k: &Self| k.exit_clear.unwrap_or(f64::INFINITY);
        self.spec.cmp(&other.spec)
            .then(self.major.total_cmp(&other.major))
            .then(self.pitch.total_cmp(&other.pitch))
            .then(self.start_diameter.total_cmp(&other.start_diameter))
            .then(self.top_z.total_cmp(&other.top_z))
            .then(self.bottom_z.total_cmp(&other.bottom_z))
            .then(self.through.cmp(&other.through))
            .then(clear(self).total_cmp(&clear(other)))
    }
}

pub(super) fn thread_operations(holes: &[Hole]) -> Vec<model::Cut> {
    let mut groups: Vec<(ThreadKey, Vec<&Hole>)> = vec![];
    for h in holes {
        let Some(t) = &h.thread else { continue };
        let k = ThreadKey {
            spec: t.spec.clone(), major: t.major, pitch: t.pitch, start_diameter: h.diameter,
            top_z: t.top_z, bottom_z: t.bottom_z, through: h.bottom == "through", exit_clear: h.exit_clear,
        };
        match groups.iter_mut().find(|g| g.0 == k) {
            Some(g) => g.1.push(h),
            None => groups.push((k, vec![h])),
        }
    }
    groups.sort_by(|(a, _), (b, _)| a.order(b));
    groups
        .into_iter()
        .map(|(k, members)| {
            model::Cut::Thread(model::Thread {
                spec: k.spec, major: k.major, pitch: k.pitch, start_diameter: k.start_diameter,
                top_z: k.top_z, bottom_z: k.bottom_z, through: k.through,
                centers: members.iter().map(|m| m.center).collect(),
                features: members.iter().map(|m| m.feature.clone()).collect(),
                hand: None,
                // Preserve unrestricted legacy plans; only a measured obstruction adds a bound.
                exit_clear_mm: k.exit_clear.filter(|_| k.through),
            })
        })
        .collect()
}
