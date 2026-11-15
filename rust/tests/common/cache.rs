//! In-memory test results, initialized once per exact input even with parallel tests.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

type Entries<T> = Mutex<HashMap<String, Arc<OnceLock<T>>>>;

pub struct Cache<T> {
    entries: OnceLock<Entries<T>>,
}

impl<T: Clone> Cache<T> {
    pub const fn new() -> Self {
        Self { entries: OnceLock::new() }
    }

    /// Return a private copy: a test's edits cannot change another test's input.
    pub fn get(&self, key: String, compute: impl FnOnce() -> T) -> T {
        let entry = self.entries.get_or_init(|| Mutex::new(HashMap::new()))
            .lock().unwrap().entry(key).or_default().clone();
        // Only matching inputs wait for each other; unrelated runs remain parallel.
        entry.get_or_init(compute).clone()
    }
}
