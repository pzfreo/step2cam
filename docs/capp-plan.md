# step2cam in three phases: Specify, CAPP, execution

Status: step 1 (schemas), step 2 (planning, tool choice and exclusions in the `capp` crate) and
step 3 (the decision trace, with the rules inventoried in `rust/capp/rulebase/rules.json`) and
step 4 (the thread tables as reference data), step 5 (ISO 14649 names) and step 6 (tool
selection, operation order and feature handling as JSON rules) are done. Of step 7, the material
(from Specify, overridable, placed in an ISO 513 group) and cutting data by group are done; the
rest of step 7 and step 8 are planned.

Step 5 was done as names on each operation (`iso14649`: feature, operation, hole bottom
condition, exact or closest), not as a restructured plan format v2. Renaming the plan's own
structure would have meant rewriting the page and the camlib adapter with no change in what
they do, while the names alone are what an AP238 export needs. The names were checked against
STEP Tools' published AP238 lists, which settled the rows marked "check" below: a notch is a
`Rectangular_open_pocket` (or `_closed_`), roughing a `General_removal_volume`, a V-channel a
`Slot` with a `Vee_profile`; ISO 14649-11 has no milling operation for thread milling or
chamfering, so those say `exact: false`.

Step 4 moved only data that comes from a standard or recognised practice. The numeric tolerances
in the code (`LEVEL_EPS`, `AXIS_TOLERANCE`, camlib's surface-error limit, the comparison budgets)
are step2cam's own judgement or camlib's limits, not reference data, so they stay as named
constants beside the code that uses them.

Decided (7 November 2026):
- CAPP reads specify-core's output as it is. It has no feature schema of its own.
- CAPP is a crate in this repository.
- Rules are JSON for now.
- The material comes from Specify, and `resources.json` can override it.
- ISO data comes from public sources only. No standards are bought.

## Why change

Today the logic between feature recognition and camlib is about 2,500 lines of Rust in
`rust/src/plan.rs` and `rust/src/job.rs`. It works, but its knowledge is mixed in with the code
that applies it:

- **Standards data is buried in code.** `COARSE_PITCH` (ISO 261) sits in `plan.rs`, and the
  full-thread rule in `full_thread()`.
- **Rules have no names.** "Threads last", "largest end mill that fits the slot", "a wall finish
  needs roughing first" and "a flat may be at most a stepdown deep" are `if` statements in five
  different places.
- **Decisions cannot be traced.** The output says what was planned and, for skipped features, a
  sentence of prose. It does not say which rule chose an operation or a tool, what else was
  considered, or why that was rejected.
- **The phases are mixed.** `job.rs` both decides (tool choice, `settle`) and writes camlib's
  manifest. `lib.rs` feeds camlib's refusals back by editing the plan.

A search in November 2026 found no open-source project that encodes this knowledge. Open
recognisers stop at labelling features, open CAM tools expect the user to choose operations, and
the rule sets in FeatureCAM, CAMWorks, NX and CAM Assist are closed. So step2cam is building it,
and it should be built as a reusable thing with a contract, not as glue.

## The three phases

```
STEP ─► 1. Specify ─► analysis + PMI + mesh ─► 2. CAPP ─► process-plan.json ─► 3. Execution ─► G-code
        (specify-core,  (specify-core's own      rules,                         camlib adapter   + checks
         unchanged)      formats)                reference data,                post (LinuxCNC)
                                                 resources.json                 others later
```

**1. Specify** (specify-core, unchanged). STEP in, then the analysis, the PMI and the mesh out,
in specify-core's own formats. step2cam has no feature schema of its own: features, faces and
their ids, and the requirements on them, are exactly as specify-core writes them.

- **Features and geometry:** the analysis (`schema: 1`), with `features`, `faces`,
  `dimensioning` and the user's `answers`, and the mesh (`pmi-assist-mesh`).
- **Requirements:** the PMI, `haecceity.pmi/1`, whose schema is published in specify-core
  (`crates/specify-core-py/python/specify_core/schema/haecceity.pmi-1.json`). It holds
  threads, fits, tolerances, surface textures and material. Thread designations reach step2cam
  today through Specify's answers (`hole.function:` … `tapped:M6`).
- **Material:** Specify's `part.material` answer, a name from specify-core's material list
  (`src/materials.rs`), which also gives its family.
- **Upstream need:** the analysis format has a version but no published JSON Schema. CAPP
  should validate its input against a schema, and that schema belongs in specify-core next to
  the PMI's. It is recorded as S1 in step2cam's `docs/upstream-needs.md`. Until it exists, CAPP
  checks the fields it reads, as `plan.rs` does now.

**2. CAPP** (computer-aided process planning). The crate `rust/capp` in this repository, as a
workspace member beside the existing crate. It is a pure function: JSON in, JSON out, with no
I/O and no camlib.

- In:
  - specify-core's analysis, PMI and mesh, as above;
  - `resources.json`: the machine, tools, cutting data, stock and fixturing options, shop
    policy, and optional overrides such as the material;
  - the **rulebase**: versioned and shipped with the crate;
  - optionally `exclusions.json`.
- Out: `process-plan.json`. This holds setups, workplans and workingsteps: each a feature, an
  operation, a tool, technology and a strategy. It also holds what was not planned and why, and
  the **decision trace**.

**3. Execution.** It takes the process plan and produces machine output.

- **camlib adapter:** what is left of `job.rs` once its decisions move to CAPP. It turns a
  workingstep into camlib manifest entries.
- **Post-processing** (`post.rs`) and camlib's material comparison stay here.
- **Others, later:** an ISO 14649 / AP238 (STEP-NC) export of the plan, which is nearly free if
  the plan uses its vocabulary, and potentially a FreeCAD CAM adapter.
- **Refusals:** an execution engine can refuse a workingstep. The refusal goes back to CAPP as
  `exclusions.json`, and CAPP replans. The trace records "refused by camlib: …" against the rule
  that chose it. This replaces `remove_operation` editing the plan in `lib.rs`.

## Schemas

step2cam's own documents are JSON Schema (2020-12), versioned, in `schemas/`. Every document
carries `format` and `version`. Tests validate every fixture and every output against them.
Inputs from specify-core are validated against specify-core's schemas, not copies of them.

| Document | Owner | Holds |
|---|---|---|
| analysis, PMI, mesh | specify-core | Features, faces, requirements, material |
| `resources.json` | the user or the shop | Machine (axes, travels, spindle, tool changer), tool library (ISO 13399 property names), cutting data (by ISO 513 material group), stock and fixturing options, shop policy values, `overrides` |
| `rulebase/` | step2cam | Reference tables, rules, rule metadata (below) |
| `process-plan.json` | CAPP | Setups, workplans, workingsteps, unplanned features, decision trace, rulebase version and hash |
| `exclusions.json` | execution, back to CAPP | Workingsteps an engine refused, with its message |

### Material

The material comes from Specify's `part.material` answer, unless `resources.json` sets
`overrides.material`, for example to run a job in a different stock than the drawing says. The
trace records which one was used and why.

CAPP needs the ISO 513 group (P, M, K, N, S, H) to choose cutting data. A reference table maps
specify-core's material families to groups, for example aluminium to N and stainless steel to M.
A material the table cannot place is a decision with a clear reason: it falls back to the
`resources.json` default and says so.

### Output vocabulary: ISO 14649

Features stay specify-core's: a workingstep cites the specify-core feature id it machines. What
uses ISO 14649 is the plan's own structure: workplan, workingstep, setup, operations and tool
types, written in snake_case. Each workingstep also records the ISO 14649-10 feature type its
specify-core family corresponds to, so an AP238 export is a mapping.

The rows marked "check" need confirming against the published EXPRESS schema before they are
fixed.

| specify-core family or today's plan kind | ISO 14649 |
|---|---|
| hole | `round_hole`, with `through_bottom_condition` / `flat_hole_bottom` / `conical_hole_bottom` |
| hole with cbore, spotface | `counterbore_hole` (compound) |
| hole with csink | `countersunk_hole` (compound) |
| tapped hole | `round_hole` + `thread` |
| slot (round-ended) | `slot`, `rounded_u_profile` / `square_u_profile` |
| notch (open on one side) | `open_pocket` (check) or `step` |
| v_channel | `slot` with `vee_profile` (check) |
| step_levels, flats | `planar_face` / `step` |
| outer walls | `general_outside_profile` |
| fillets | `edge_round` |
| chamfers | `chamfer` |
| roughing of the rest | no exact entity; `region` is closest (check) |
| facing / flats | `plane_milling` |
| walls | `side_milling` / `bottom_and_side_milling` |
| drill, spot | `drilling`, `center_drilling` |
| thread | `thread_milling` (and `tapping` once tapping exists) |
| bore by helix | `boring` is turning-style in 14649; helical milling is `bottom_and_side_milling` with a helix strategy (check) |

Fields 14649 does not have, such as the camlib-specific strategy, go under `ext.step2cam`.

## The rulebase: how knowledge is stored

There are four kinds of knowledge. They are stored apart, so that every value has a known kind
and source:

1. **Reference data:** facts from standards, as tables with no logic. Candidates:
   - ISO 261/262 thread sizes and pitches; ISO 68-1 and 724 basic profile and minor diameter.
   - Tap-drill sizes (ISO 2306).
   - ISO 273 clearance holes.
   - ISO 286 tolerance grades and hole fits.
   - ISO 2768 general tolerances, the default when a dimension states none.
   - ISO 513 material groups.
   - ISO 13399 tool property names (DC, APMX, LF, RE, KAPR …).
   - ISO 15065 countersinks.
2. **Rules:** decisions with conditions, each with an id. Every rule names one or more sources,
   which may be a standard, recognised practice, or step2cam's own heuristic.
3. **Shop policy:** numbers a shop chooses, such as margins, allowances, clearance planes,
   breakthrough and preferred tools. They live in `resources.json`, not the rulebase. That way a
   placeholder can't pass as a standard.
4. **Geometric predicates:** questions answered by computing on the part. Examples:
   `reachable`, `room_beside`, `occupied_above`, `clear_below` and `surface_error`. These stay
   Rust code in the CAPP crate, because they are algorithms, but each has an id and returns its
   evidence, so a rule can cite it.

### Layout

All JSON, in `rust/capp/rulebase/`, embedded in the crate at build time so the WASM build needs no
file access:

```
rulebase/
  manifest.json                  version, content hash, list of files
  reference/iso261-pitch.json    { "source": {...}, "columns": [...], "rows": [...] }
  reference/iso68-profile.json   formulas: H = 0.866025 P, minor diameter D1 = D - 1.082532 P
  reference/tap-drill.json
  reference/iso286-it-grades.json
  reference/iso513-groups.json   and specify-core material family -> group
  rules/setups.json              setup and orientation choice
  rules/process/holes.json       feature -> alternative operation sequences
  rules/process/slots.json
  rules/process/walls.json
  rules/tools.json               tool selection
  rules/technology.json          cutting data selection
  rules/sequence.json            precedence and grouping
```

### A rule

```json
{
  "id": "process.hole.threaded.thread_mill",
  "version": 1,
  "stage": "process_selection",
  "applies_to": { "family": "holes" },
  "when": [
    { "fact": "requirement.thread.system", "is": "iso_metric" },
    { "predicate": "reachable", "from": "setup" }
  ],
  "then": {
    "sequence": [
      { "operation": "center_drilling", "unless": { "fact": "policy.skip_spot", "is": true } },
      { "operation": "drilling", "diameter": { "table": "tap_drill", "key": "requirement.thread.designation" } },
      { "operation": "thread_milling", "depth": { "rule": "thread.full_depth.blind" } }
    ]
  },
  "preference": 20,
  "sources": [
    { "kind": "standard", "ref": "ISO 2306", "via": "public table, see reference/tap-drill.json" },
    { "kind": "step2cam", "note": "thread milling, not tapping: no tapping head on the demo machine" }
  ],
  "rationale": "A metric thread is drilled to the tap-drill size and then thread-milled."
}
```

`applies_to` names specify-core's family; facts such as `requirement.thread.designation` are
read from specify-core's PMI and answers through a fixed, documented set of fact names. That set
is the one place that knows specify-core's field paths.

The rule language stays deliberately small:
- **Conditions:** an AND list of fact comparisons and predicate calls.
- **Outcome:** either an operation sequence, or a choice among candidates with ordered ranking
  keys. Most tool rules are "the largest that fits", which ranking keys express.
- **Values:** looked up from tables or computed by named formulas.

There is no general expression language: a rule that needs more becomes a named Rust function
with an id and metadata, and the trace treats it the same way. If more than a handful of rules
end up as functions, that is a sign the rule language needs one more construct, not that every
rule should be code.

### Traceability

Every outcome in `process-plan.json` cites the decisions behind it:

```json
"workingsteps": [
  { "id": "ws-top-07", "feature": "holes:4", "iso14649_feature": "round_hole",
    "operation": "thread_milling", "tool": "tm4", "because": ["d-012", "d-031", "d-044"] }
],
"decisions": [
  { "id": "d-031", "stage": "tool_selection", "subject": "ws-top-07",
    "question": "which tool for thread_milling M6x1 in holes:4",
    "rule": "tools.thread_mill.largest_fitting@1",
    "inputs": { "pitch": 1.0, "start_diameter": 5.0, "depth_below_top": 12.0 },
    "considered": [
      { "candidate": "tm6", "rejected": "its ⌀4.8 tooth does not fit the ⌀5 hole", "rule": "tools.thread_mill.fits_hole@1" },
      { "candidate": "tm3", "rejected": "its ⌀2.2 neck would rub the crests", "rule": "tools.thread_mill.neck_clears@1" }
    ],
    "chosen": "tm4",
    "depends_on": ["d-012"] }
]
```

This makes the following possible:
- **Why this tool, and why not that one:** read the decision.
- **Which outputs rely on a rule or table:** search `rule` across plans.
- **Reproducibility:** the plan records the rulebase version and hash, so the same inputs and
  rulebase give the same plan, byte for byte. A test enforces this.
- **Unplanned features** are decisions too, with the rule that excluded them. This replaces
  today's `skipped` and `warnings` prose.
- **The page** can show this: click a toolpath, and see its decision chain.

## ISO standards from public sources only

There is no access to the standards themselves, so every reference table is built from public
sources. The rulebase must never copy a standard's text; it holds values only. Each table
records where its values came from and how far they can be trusted:

```json
"source": {
  "standard": "ISO 261",
  "obtained_from": ["<URL of the public table>", "<a second, independent URL>"],
  "agreement": "both sources agree on every row",
  "checked_against_standard": false
}
```

The rules for building tables:
- **Prefer formulas** that follow from a standard's published definitions over transcribed
  tables. The ISO 68-1 basic profile gives the minor diameter, and the ISO 286 standard
  tolerance factor gives the IT grades. A formula is checked against several published rows in
  a test.
- **Otherwise use two independent public sources** that agree, such as tool makers' catalogues
  and engineering references. Where they disagree, the table records both and the rule takes the
  safer value, for example the smaller tap drill.
- **Every table has a test** comparing it with spot values from its sources.
- **Mark it.** `checked_against_standard: false` shows in the trace, and the page can say so.

What public sources can realistically support:

| Standard | Public availability | Plan |
|---|---|---|
| ISO 261/262 metric thread pitches | Widely published | Table, two sources |
| ISO 68-1 / 724 basic profile, minor diameter | Formulas widely published | Formula |
| ISO 2306 tap drills | Common practice D − P is widely published; the standard's own sizes less so | Table from catalogues, D − P as a check |
| ISO 273 clearance holes | Widely published (fine, medium, coarse) | Table |
| ISO 286 IT grades and hole deviations | IT formula and common fits (H7, H8 …) widely published | Formula for IT, table for common fits only |
| ISO 2768 general tolerances (f, m, c, v) | Widely published | Table |
| ISO 513 groups P M K N S H | Widely published by tool makers | Table, plus family mapping |
| ISO 13399 property symbols | Published by tool makers for their catalogues | Names only, for the tool library |
| ISO 14649 / AP238 entity names | Papers, NIST and STEP Tools material; EXPRESS schemas may be public | Names; "check" rows above until a schema is found |
| ISO 15065 countersinks | Less widely published | Later, if a source is found |

Cutting data (speeds and feeds by material group) is not standardised. It stays shop policy in
`resources.json`, with the demo's placeholder numbers marked as placeholders.

## CAPP's internal stages

Each stage is a pure function, testable alone:

1. **Interpret requirements.** Read specify-core's PMI and answers, then resolve threads, fits
   and finishes into concrete targets from reference data. For example, `M6` becomes major 6,
   pitch 1, a minor diameter and a tap drill. An `H7` hole becomes a tolerance band, which in
   turn needs reaming or boring. The material resolves to an ISO 513 group.
2. **Setup planning.** Choose orientations, stock and work origin. This is today's `SETUPS` and
   `Frame`.
3. **Process selection.** For each feature, list the alternative operation sequences and keep
   those whose conditions hold. Then pick by preference, and record the rest as considered.
4. **Resource selection.** Choose tools and cutting data.
5. **Feasibility.** Drop what can't run and everything that depends on it. This is today's
   `settle`, and wall finishes needing roughing are an example. It also applies `exclusions`.
6. **Sequencing.** Order workingsteps under precedence rules:
   - Facing comes first.
   - Roughing comes before finishing.
   - Drilling comes before threading.
   - Threads come last in a setup.
   - Then group by tool.
7. **Emit** the process plan and its trace.

## Where today's code goes

| Today | Moves to |
|---|---|
| `plan.rs` setups, frames, stock | `capp`, stage 2 |
| `plan.rs` `COARSE_PITCH`, `thread_size`, `full_thread` | reference table plus a named formula rule |
| `plan.rs` `threads()` reading Specify's answers | `capp` stage 1, behind the fact names |
| `plan.rs` `hole_geometry`, `slot_operation`, `notch_operation`, `v_channel_operation`, `wall_operations`, `edge_operations` | `capp` stage 3 rules, with their geometry as predicates |
| `plan.rs` mesh geometry (`room_beside`, `occupied_above`, `clear_below`, `surface_error`, slicing) | `capp` predicates module |
| `job.rs` `Library` (`drill`, `pocket_mill`, `v_bit`, `thread_mill`, …) | `capp` stage 4 rules |
| `job.rs` `settle`, `wall_refusal`, `v_channel_refusal` | `capp` stage 5 |
| `job.rs` manifest building, `cutting`, `assembly`, depths | camlib adapter, which stays in the `step2cam` crate (phase 3) |
| `lib.rs` retry loop and `remove_operation` | `exclusions.json` round trip |
| `post.rs` | unchanged, in the `step2cam` crate (phase 3) |
| `demo/tools.json` | `resources.json`, its tools in ISO 13399 property names; its placeholder numbers marked as shop policy |

The `step2cam` crate keeps the WASM entry point (`process`) and calls `capp`, then the camlib
adapter, then the post-processor.

## Steps

Each step keeps the existing G-code goldens and the parity tests identical, unless the step says
it changes output.

1. **Workspace and schemas.** Add an empty `capp` crate as a member of `rust/`'s workspace. Write the
   JSON Schemas for `resources.json` (from `tools.json` as it is) and the current plan output,
   and validate them in tests. Record the analysis-schema need in `docs/upstream-needs.md`. No
   behaviour change.
2. **Move planning into `capp`.** Move `plan.rs`, tool selection and `settle` out of `job.rs`, and
   the geometry into `capp`, unchanged. `job.rs` becomes the camlib adapter, reading resolved
   tools from the plan. Then replace the retry loop with `exclusions`.
3. **Trace scaffolding.** Give every existing hard-coded rule an id and metadata (an inventory),
   and have each one emit a decision record. `skipped` and `warnings` become decisions. The
   output gains a trace, and the G-code stays identical.
4. **Reference data out of code.** Move `COARSE_PITCH`, the full-thread rule and the tolerance
   constants into `rulebase/reference/`, with their public sources and tests.
5. **ISO 14649 plan vocabulary.** Plan format v2 with ISO names for its structure. The page reads
   v2.
6. **Rules as data.** Move process alternatives, tool ranking and precedence into
   `rulebase/rules/`, with a small evaluator. The rules that remain functions keep their ids.
7. **New knowledge** (changes output):
   - Material from Specify with the override, mapped to ISO 513 groups.
   - ISO 286 fits from the PMI, so that reamed or bored holes are chosen.
   - ISO 2768 defaults.
   - An ISO 13399 tool library.
   - Surface finish from the PMI selecting finishing passes.
8. **Optional:**
   - AP238 (STEP-NC) export.
   - A second execution engine.
   - The page showing decision chains.

Steps 1–4 restructure without changing the G-code and could land as separate PRs. Step 5
changes the plan format. Steps 6–7 are where the rulebase starts to grow.
