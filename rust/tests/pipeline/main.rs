//! Tests that share complete process inputs run together so their cache lives once.
//! Keep each former test file as a module, preserving its test names and assertions.
#[path = "../common/mod.rs"]
mod common;

mod adapter;
mod adaptive;
mod camlib_reach;
mod carvera;
mod flip;
mod front_holes;
mod grip_force;
mod haas_vf2;
mod iso13399;
mod machines;
mod material;
mod parity;
mod schemas;
mod slots;
mod stepped;
mod trace;
mod walls;
mod workholding;
mod zero;

#[test]
fn parallel_cache_requests_compute_each_input_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let cache = common::cache::Cache::new();
    let calls = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..16 {
            scope.spawn(|| {
                for key in ["first input", "second input"] {
                    let result = cache.get(key.into(), || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        key.to_string()
                    });
                    assert_eq!(result, key);
                }
            });
        }
    });
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn cached_results_are_private_copies_and_keys_are_exact() {
    use serde_json::json;
    let cache = common::cache::Cache::new();
    let mut edited = cache.get("input".into(), || json!({"plan": [1, 2]}));
    edited["plan"][0] = json!(99);
    let original = cache.get("input".into(), || panic!("a cached input ran again"));
    assert_eq!(original, json!({"plan": [1, 2]}));
    let different = cache.get("input with changed options".into(), || json!({"plan": [3]}));
    assert_eq!(different, json!({"plan": [3]}));
}
