//! camlib's intent checker compares what each written block says with the source: change one word
//! of the G-code and the post must fail.
use serde_json::Value;
use std::{fs, path::PathBuf};
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
    match post_tampered(&manifest, machine, Some(tamper)) {
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
fn a_wrong_spindle_speed_is_refused() {
    refused(&|l| if l.starts_with("M3 ") { "M3 S1234".into() } else { l.into() });
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
