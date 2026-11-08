//! Computer-aided process planning (CAPP): specify-core's analysis and mesh, with a shop's
//! resources, in; a process plan out. JSON in, JSON out, no I/O. See `docs/capp-plan.md`.
//!
//! [`plan::plan`] decides what to cut in which setup; [`settle::finish`] takes out what these
//! resources cannot cut, and what an execution engine refused, and names each operation's tool.
pub mod machines;
mod needed;
pub mod plan;
pub mod reference;
pub mod resources;
pub mod rules;
pub mod settle;
pub mod trace;

pub use settle::{exclude, finish, resolve, settle};
