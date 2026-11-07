//! camlib's intent checker compares what each written block says with the source: change one word
//! of the G-code and the post must fail.
use serde_json::Value;
use std::{cell::Cell, fs, path::PathBuf};
use step2cam::post::{Machine, post, post_tampered};

fn fixture(path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
fn job() -> (String, Machine) {
    let library = fixture("examples/tools.example.json");
    let plan = fixture("tests/fixtures/rust-golden/plate.plan.json");
    let (jobs, _) = step2cam::job::jobs(&plan, &library).unwrap();
    (jobs[0].manifest.to_string(), serde_json::from_value(library["machine"].clone()).unwrap())
}
fn refused(tamper: &dyn Fn(&str) -> String) -> String {
    let (manifest, machine) = job();
    refused_in(&manifest, machine, tamper)
}
/// The stepped plate's first job: unlike the plate, it has moves shorter than the tolerance.
fn stepped_job() -> (String, Machine) {
    let library = fixture("demo/tools.json");
    let plan = step2cam::plan::plan(&fixture("tests/fixtures/synthetic/stepped-plate.analysis.json"), Some(&fixture("tests/fixtures/synthetic/stepped-plate.mesh.json")), &Default::default()).unwrap();
    let (jobs, _) = step2cam::job::jobs(&plan, &library).unwrap();
    (jobs[0].manifest.to_string(), serde_json::from_value(library["machine"].clone()).unwrap())
}
fn refused_in(manifest: &str, machine: Machine, tamper: &dyn Fn(&str) -> String) -> String {
    match post_tampered(manifest, machine, Some(tamper)) {
        Ok(_) => panic!("the tampered program was accepted"),
        Err(failure) => failure.message,
    }
}

#[test]
fn an_untouched_program_passes_and_tampering_with_nothing_changes_nothing() {
    let (manifest, machine) = job();
    let plain = post(&manifest, machine.clone()).map_err(|f| f.message).unwrap();
    let same = post_tampered(&manifest, machine, Some(&|l: &str| l.to_string())).map_err(|f| f.message).unwrap();
    assert_eq!(plain.nc, same.nc);
}

#[test]
fn a_wrong_spindle_speed_is_refused_by_the_intent_checker() {
    let message = refused(&|l| if l.starts_with("M3 ") { "M3 S1234".into() } else { l.into() });
    assert!(message.contains("differ from source"), "{message}");
}

#[test]
fn a_reversed_spindle_is_refused() {
    refused(&|l| l.replace("M3 ", "M4 "));
}

#[test]
fn a_wrong_length_offset_is_refused() {
    let message = refused(&|l| if l.starts_with("G43 ") { "G43 H77".into() } else { l.into() });
    assert!(message.contains("length register"), "{message}");
}

#[test]
fn a_wrong_tool_is_refused() {
    refused(&|l| if l == "T3" { "T4".into() } else if l == "G43 H3" { "G43 H4".into() } else { l.into() });
}

#[test]
fn missing_coolant_is_refused() {
    refused(&|l| if l == "M8" { "M9".into() } else { l.into() });
}

#[test]
fn a_wrong_work_offset_is_refused() {
    refused(&|l| l.replace("G54", "G55"));
}

#[test]
fn a_changed_feed_is_refused() {
    refused(&|l| if l.starts_with("G1 ") && l.contains(" F") { format!("{} F9", l.split(" F").next().unwrap()) } else { l.into() });
}

#[test]
fn a_changed_arc_feed_is_refused() {
    refused(&|l| if (l.starts_with("G2 ") || l.starts_with("G3 ")) && l.contains(" F") { format!("{} F9", l.split(" F").next().unwrap()) } else { l.into() });
}

#[test]
fn a_rapid_written_as_a_feed_move_is_refused() {
    refused(&|l| if l.starts_with("G0 ") { format!("G1 {} F100", &l[3..]) } else { l.into() });
}

#[test]
fn a_cut_written_as_a_rapid_is_refused() {
    refused(&|l| if l.starts_with("G1 ") && l.contains(" F") { format!("G0 {}", l[3..].split(" F").next().unwrap()) } else { l.into() });
}

#[test]
fn an_arc_written_the_wrong_way_round_is_refused() {
    refused(&|l| if l.starts_with("G2 ") { format!("G3 {}", &l[3..]) } else { l.into() });
}

#[test]
fn a_move_written_in_the_wrong_place_is_refused() {
    refused(&|l| if l.starts_with("G1 ") { l.replacen(" Y", " Y1", 1) } else { l.into() });
}

/// One word of what the program fixes for good changed: nothing later would notice on its own.
#[test]
fn every_word_of_the_preamble_is_read_back() {
    for (word, wrong) in [("G21", "G20"), ("G90", "G91"), ("G91.1", "G90.1"), ("G17", "G18"), ("G49", "G43 H9"), ("G40", "G41"), ("G94", "G93"), ("G61.1", "G64")] {
        let message = refused(&|l| if l == word { wrong.into() } else { l.into() });
        assert!(message.contains("preamble"), "{word}: {message}");
    }
    let message = refused(&|l| if l == "G21" { String::new() } else { l.into() });
    assert!(message.contains("preamble"), "{message}");
}

/// The plane is modal, so only a change of plane is written: the preamble's G17 is what an XY arc
/// relies on, and every_word_of_the_preamble_is_read_back holds it. Nothing else may select one.
#[test]
fn an_arc_after_a_plane_written_into_the_program_is_refused() {
    let seen = Cell::new(false);
    refused(&|l: &str| {
        if !seen.get() && (l.starts_with("G2 ") || l.starts_with("G3 ")) {
            seen.set(true);
            return format!("G18 {l}");
        }
        l.into()
    });
}

#[test]
fn the_plane_is_written_only_when_it_changes() {
    let (manifest, machine) = job();
    let nc = post(&manifest, machine).map_err(|f| f.message).unwrap().nc;
    assert!(nc.contains("\nG2 ") || nc.contains("\nG3 "), "this job has arcs");
    assert_eq!(nc.lines().filter(|l| *l == "G17").count(), 1, "only the preamble's");
}

#[test]
fn every_word_of_the_end_is_read_back() {
    let message = refused(&|l| if l == "M2" { "M0".into() } else { l.into() });
    assert!(message.contains("end of the program"), "{message}");
    let message = refused(&|l| if l == "M2" { String::new() } else { l.into() });
    assert!(message.contains("end of the program"), "{message}");
    // The preamble's G49 stays; the end's does not.
    let seen = Cell::new(0);
    let message = refused(&|l: &str| {
        if l == "G49" {
            seen.set(seen.get() + 1);
            if seen.get() > 1 {
                return "G43 H9".into();
            }
        }
        l.into()
    });
    assert!(message.contains("end of the program"), "{message}");
}

/// camlib's decoder gives no move for a block it does not model, which a move shorter than the
/// tolerance would otherwise pass as.
#[test]
fn a_short_move_cannot_be_replaced_by_another_word() {
    let (manifest, machine) = stepped_job();
    // These moves exist, or this test says nothing: a move that stays on its last XYZ word.
    let plain = post(&manifest, machine.clone()).map_err(|f| f.message).unwrap();
    let short = plain.nc.lines().collect::<Vec<_>>().windows(2).find(|w| {
        let words = |l: &str| l.split(' ').filter(|w| w.starts_with(['X', 'Y', 'Z'])).map(String::from).collect::<Vec<_>>();
        w[1].starts_with("G1 ") && words(w[0]) == words(w[1])
    }).expect("the stepped plate writes a move that goes nowhere")[1].to_string();
    for wrong in ["G28", "M5", "(nothing)", "", "G1", "G1 X1e3 Y0 Z0 F100"] {
        refused_in(&manifest, machine.clone(), &|l| if l == short { wrong.into() } else { l.into() });
    }
}

#[test]
fn a_word_with_a_fraction_is_not_read_as_its_whole_part() {
    for (word, wrong) in [("G54", "G54.9"), ("G54", "G54.3"), ("T3", "T3.9"), ("G43 H3", "G43 H3.9")] {
        let message = refused(&|l| if l == word { wrong.into() } else { l.into() });
        assert!(message.contains("unreadable"), "{wrong}: {message}");
    }
}

#[test]
fn a_number_the_post_would_not_write_is_refused() {
    refused(&|l| if l.starts_with("M3 ") { "M3 S6e3".into() } else { l.into() });
    refused(&|l| if l.starts_with("G1 ") { l.replacen(" Y", " Y1e0", 1) } else { l.into() });
}

fn process_with(library: &Value) -> Value {
    step2cam::process(&fixture("tests/fixtures/plate.analysis.json"), &fixture("tests/fixtures/plate.mesh.json"), library).unwrap()
}

/// A real mismatch, with nothing tampered: a speed that rounds away at this precision. It used to
/// cost the operation and leave the rest of the program looking fine.
#[test]
fn a_program_that_does_not_read_back_is_not_given_out() {
    let mut library = fixture("demo/tools.json");
    library["machine"]["decimals"] = 2.into();
    for tool in library["tools"].as_array_mut().unwrap() {
        let rpm = tool["spindle_rpm"].as_f64().unwrap();
        tool["spindle_rpm"] = (rpm + 0.125).into();
    }
    let out = process_with(&library);
    let message = out["post_error"].as_str().unwrap_or_else(|| panic!("given out: {}", out["plan"]["warnings"]));
    assert!(message.contains("does not match the plan"), "{message}");
    assert_eq!(out["programs"].as_array().unwrap().len(), 0);
}

/// Over the machine's limit is a refusal of that operation, not a fault in the post.
#[test]
fn an_operation_the_machine_cannot_do_is_only_left_out() {
    let mut library = fixture("demo/tools.json");
    library["machine"]["max_spindle_rpm"] = 7000.into();
    let out = process_with(&library);
    assert!(out["post_error"].is_null(), "{}", out["post_error"]);
    assert!(!out["programs"].as_array().unwrap().is_empty());
}

/// Roughing covers features that have no operation of their own, until it is refused: then they
/// must not be reported as covered.
#[test]
fn nothing_is_covered_by_a_roughing_that_was_refused() {
    let mut library = fixture("demo/tools.json");
    for tool in library["tools"].as_array_mut().unwrap() {
        if tool["type"] == "end_mill" {
            tool["flute_length"] = 1.into();
            tool["stickout"] = 1.5.into();
        }
    }
    let out = step2cam::process(&fixture("tests/fixtures/mounting-plate.analysis.json"), &fixture("tests/fixtures/mounting-plate.mesh.json"), &library).unwrap();
    assert!(out["plan"]["warnings"].to_string().contains("roughing left out"), "{}", out["plan"]["warnings"]);
    assert_eq!(out["plan"]["covered_by_roughing"], serde_json::json!([]));
    assert!(out["plan"]["skipped"].to_string().contains("roughing was refused"), "{}", out["plan"]["skipped"]);
}
