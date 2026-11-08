# MachinePlan-10K benchmark

A hand-run benchmark for recognition and planning changes, using the fixed 100-part seed-1
sample in `sample.txt`. No dataset, downloaded cache or generated run reports belong in Git;
`baseline.json` is the intentionally committed comparison data.

MachinePlan-10K contains 10,000 synthetic 2.5-axis steel parts, their STEP geometry and labelled
process plans. Attribution: C. Dharmarajan, Athul; Arvanitis, Elena; Ameta, Gaurav (2026),
*MachinePlan-10K: A BRep-Based Synthetic Dataset for Machining Process Planning with Tool-Aware
Feature Sampling*, version 1.0, Zenodo, [doi:10.5281/zenodo.21653081](https://doi.org/10.5281/zenodo.21653081).
The dataset is licensed [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
Its plans are a CAM system's unedited defaults (every feed is 250 mm/min): they are a reference
for features and hole sequences, not for good machining practice. This benchmark compares
feature counts and plan outcomes; it does not yet compare hole operation sequences.

## Run

Requires Python 3.9+, Rust/Cargo, the pinned camlib (`rust/fetch-camlib.sh`), and `specify-core`
on PATH. Python uses only the standard library. Run from the repository root:

```sh
rust/fetch-camlib.sh
python3 -I tools/benchmark/run.py --out /tmp/current.json
python3 -I tools/benchmark/compare.py tools/benchmark/baseline.json /tmp/current.json
python3 -m unittest discover tools/benchmark
```

Always use `python3 -I` for code that reads dataset files. The runner imports only its own
sibling modules explicitly. It builds the release `process` example once, then runs
`specify-core analyse`, `specify-core mesh` and the built `process` executable directly per part.
The executable path comes from Cargo's JSON artifact output, respecting `CARGO_TARGET_DIR`
and Cargo target-directory configuration. Workers never invoke Cargo.
Default parallelism is four parts (`--jobs N`); the tool library defaults to `demo/tools.json`
(`--tools PATH`). Each per-part subprocess has a ten-minute timeout; the initial build allows thirty minutes. Errors are retained per part;
the runner writes the report before exiting nonzero if any part failed.

The default source is Zenodo's ZIP content endpoint. Only ZIP metadata and selected STEP,
operations JSON and details members are read, through validated HTTP ranges. A server that
ignores ranges is rejected before its body is read. Use `--zip /path/MachinePlan-10K.zip` for
a local archive. Extracted members and temporary CLI results stay under
`~/.cache/step2cam-benchmark` (`--cache PATH`, outside the repo). Members are bounded to 128 MiB
and paths are checked for traversal and escaping symlinks; nothing is executed from the ZIP.
Fetched ranges (including ZIP metadata) and extracted archive members are cached for this fixed
record URL; recognition and planning always run again. The first remote run reads roughly 60 MB
of ZIP metadata before the selected members. Delete the cache to force a complete refetch. Use a fresh cache
to verify remote fetching, for example `--limit 3 --cache /tmp/mp10k-remote`.

Allow several minutes for 100 parts after the initial Rust build; CPU and network performance
vary. A cold release build can exceed ten minutes. One overall `timing.wall_seconds` records
the run, including fetching and excluding the initial build and instrumentation probe.

On macOS each stage (`analyse`, `mesh`, `process`) runs under `/usr/bin/time -l`. Its
`instructions retired` line supplies per-part, per-stage counts in `instructions.parts`.
`instructions.stages` contains totals, medians and measurement coverage for each stage.
A total is null unless every part has a count; the median uses available counts, or null
if none are available. Unattempted stages, non-macOS hosts and missing counters record null.
The runner warns once on stderr when counts are unavailable. It probes time first: macOS
sandboxes can deny its resource sysctls, making time itself fail even when the child succeeds;
in that case the binaries run directly and all counters are null.

`data` is the deterministic comparison payload, sorted by part and key with no timestamps.
Variable measurements live separately in `instructions`, `timing` and `fetch`. Performance
is information only, with no performance target or automatic failure threshold.

## Interpretation and refresh

Labels count unique Geometry Groups from `_operations.json`'s operation details: repeated
spot/drill/enlarge operations on the same group count once. STEP1HOLE/STEP1POCKET map to hole,
FG_CHAMFER_SURFACE to chamfer, POCKET_* to pocket, SLOT_* to slot and CORNER_NOTCH_* to notch.
Unknown groups remain visible as `other`. Specify's own family names are preserved. Only holes
and chamfers have directly comparable taxonomies; totals include their counts and equal-count
parts (including parts with zero of both). Failed recognition is excluded from equality checks.

Every recognised feature records `cut`, `left_out` with a grouped reason, `covered` by roughing,
or `unaccounted` (e.g. a hole-pattern container). A feature referenced by a retained operation
counts as cut, even if other operations also refer to it; cut takes precedence over covered,
then left out. This measures the final plan's claims, not machining completeness or quality.
Warnings and reasons retain their full text with numeric values replaced by `#`.

Known limit: pockets, slots and notches cannot be matched face by face: the dataset's details
give no face IDs. That would require labelled geometry/face correspondence in the dataset,
or reconstructing and geometrically matching each labelled feature to Specify's faces.

The comparison prints family totals/deltas, changed reasons, and changed parts with before/after
data. It also prints instruction-total deltas per stage and lists common parts whose summed
three-stage count changed by strictly more than 20% in either direction. Incomplete counts
are not compared as whole-part totals; a change from zero to a positive count is listed.
Old reports without instruction measurements remain readable. Performance and wall time
never affect the exit status. Fewer cuts are highlighted for review, but do not fail the gate.
Only a newly failing part or a new error on a previously failing part exits 1; existing errors
and resolved errors exit 0. Added/removed parts are shown and should be reviewed for sample
compatibility. Invalid input reports are command errors.

To refresh the baseline, use the unchanged 100-part sample and `demo/tools.json`, run to a file
outside the repo, compare against the existing baseline, review all differences, then copy the
validated report to `tools/benchmark/baseline.json` and commit it. Keep the baseline paired with
the planner commit it measures; the tool library SHA-256 is recorded in the report. The original
baseline reproduced `_ref/first-run-report.txt`. The current baseline includes main's Carvera
machine and its 6.35 mm shank limit, so larger tools from that first run cannot be chosen.
Fixture GD&T updates do not affect these dataset parts.
