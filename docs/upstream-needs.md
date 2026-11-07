# What step2cam needs from specify-core

specify-core is its own project and is not modified from here. Each need says what is missing,
where step2cam uses it, the stand-in step2cam uses meanwhile, and how to tell the need is met.
Checked against specify-core 6899e50 (2026-11-05).

## S1. A published JSON Schema for the analysis

**Missing.** The analysis has a version (`schema: 1`, `ANALYSIS_SCHEMA` in `src/api.rs`) but no
JSON Schema. The PMI has one (`haecceity.pmi-1.json`, under
`crates/specify-core-py/python/specify_core/schema/`), and the analysis should have one beside
it. It needs to cover the fields a consumer reads: `binding`, `faces`, `features` (each family's
`record`), `dimensioning`, `existing`, `part` and `resume.answers`.

**Used by.** step2cam's planning (`rust/src/plan.rs`, and `capp` from step 2 of
`docs/capp-plan.md`), which reads specify-core's features as they are and has no feature schema
of its own.

**Stand-in.** step2cam checks the fields it reads as it reads them, and refuses with a message
when one is missing or of the wrong type (`plan::plan`: "not a specify-core analysis", "features
is not a list", and so on). A change to the analysis format is found by
`tests/specify-link.sh`, which runs the shipped specify-core WASM on the fixtures in CI, rather
than by a schema check.

**Met when** specify-core ships the analysis schema with its releases, and step2cam's tests
validate every fixture's analysis against it.
