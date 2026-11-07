# step2cam in three phases: Specify, CAPP, execution

Status: plan only. Nothing here is built yet.

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
the rule sets in FeatureCAM, CAMWorks, NX and CAM Assist are closed. So step2cam
is building it, and it should be built as a reusable thing with a contract, not as glue.

## The three phases

```
STEP ─► 1. Specify ─► part.json ─► 2. CAPP ─► process-plan.json ─► 3. Execution ─► G-code
        (specify-core)              rules +                          camlib adapter    + checks
                                    reference data                   post (LinuxCNC)
                                    resources.json                   others later
```

**1. Specify** (specify-core, unchanged). STEP in, then analysis and mesh out. A small
**adapter** turns specify-core's analysis into CAPP's `part.json`. That way CAPP does not depend
on specify-core's internal format, and another recogniser (AAGNet, a commercial AFR export) could
feed it.

**2. CAPP** (computer-aided process planning). Pure function: JSON in, JSON out, no I/O, no camlib.

- In: `part.json` (what to make), `resources.json` (machine, tools, cutting data, stock and
  fixturing options), and the **rulebase** (versioned, shipped with CAPP, overridable).
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
  an input (`exclusions`), and CAPP replans. The trace records "refused by camlib: …" against
  the rule that chose it. This replaces `remove_operation` editing the plan in `lib.rs`.

## Schemas

All JSON Schema (2020-12), versioned, in `schemas/`. Every document carries `format` and `version`.
Tests validate every fixture and every output against them.

| Document | Produced by | Holds |
|---|---|---|
| `part.json` | Specify adapter | Units, part bounds, features (ISO 14649-10 names), faces, mesh reference, requirements (threads, fits, surface finish, general tolerance), material |
| `resources.json` | the user or the shop | Machine (axes, travels, spindle, tool changer, ISO 841 axes), tool library (ISO 13399 properties), cutting data (by ISO 513 material group), stock and fixturing options, shop policy values |
| `rulebase/` | step2cam | Reference tables, rules, rule metadata (below) |
| `process-plan.json` | CAPP | Setups, workplans, workingsteps, unplanned features, decision trace, rulebase version and hash |
| `exclusions.json` | execution, back to CAPP | Workingsteps an engine refused, with its message |

### Vocabulary: ISO 14649

The plan uses ISO 14649's entity names, written in snake_case:

- **ISO 14649-10:** features, workplan, workingstep and setup.
- **ISO 14649-11:** milling operations.
- **ISO 14649-111:** milling tools.

Current names map like this. The rows marked "check" need confirming against the standard's
EXPRESS schema before they are fixed.

| Today (specify family or plan kind) | ISO 14649 |
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

Fields 14649 does not have, such as the camlib-specific strategy or mesh references, go under
`ext.step2cam`. That keeps the plan valid in the ISO vocabulary while carrying what the engine
needs.

## The rulebase: how knowledge is stored

There are four kinds of knowledge. They are stored apart, so that every value has a known kind
and source:

1. **Reference data:** facts from standards. These are tables with no logic, for example:
   - ISO 261/262 thread sizes and pitches, and ISO 724 basic dimensions (minor diameter).
   - ISO 2306 tap-drill sizes.
   - ISO 273 clearance holes.
   - ISO 286 tolerance grades and hole fits.
   - ISO 2768 general tolerances, the default when a dimension states none.
   - ISO 21920 (formerly ISO 1302/4287) surface texture parameters.
   - ISO 513 workpiece material groups (P, M, K, N, S, H) and cutting-material designations.
   - ISO 13399 tool property names (DC, APMX, LF, RE, KAPR …).
   - ISO 15065 countersinks.
2. **Rules:** decisions with conditions, each with an id. Every rule names one or more sources,
   which may be a standard, recognised practice, or step2cam's own heuristic.
3. **Shop policy:** numbers a shop chooses, such as margins, allowances, clearance planes,
   breakthrough and preferred tools. They live in `resources.json`, not the rulebase. That way a
   placeholder can't pass as a standard.
4. **Geometric predicates:** questions answered by computing on the part. Examples:
   `reachable`, `room_beside`, `occupied_above`, `clear_below` and `surface_error`. These stay
   Rust code, because they are algorithms, but each has an id and returns its evidence, so a rule
   can cite it.

### Layout

```
rulebase/
  manifest.json                  version, content hash, list of files
  reference/iso261-pitch.json    { "source": {...}, "columns": [...], "rows": [...] }
  reference/iso2306-tap-drill.json
  reference/iso286-hole-fits.json
  reference/iso513-groups.json
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
  "id": "process.round_hole.threaded.thread_mill",
  "version": 1,
  "stage": "process_selection",
  "applies_to": "round_hole",
  "when": [
    { "fact": "requirement.thread.system", "is": "iso_metric" },
    { "predicate": "reachable", "from": "setup" }
  ],
  "then": {
    "sequence": [
      { "operation": "center_drilling", "unless": { "fact": "policy.skip_spot", "is": true } },
      { "operation": "drilling", "diameter": { "table": "iso2306.tap_drill", "key": "requirement.thread.designation" } },
      { "operation": "thread_milling", "depth": { "rule": "thread.full_depth.blind" } }
    ]
  },
  "preference": 20,
  "sources": [
    { "kind": "standard", "ref": "ISO 2306", "clause": null, "note": "tap-drill diameter" },
    { "kind": "step2cam", "note": "thread milling, not tapping: no tapping head on the demo machine" }
  ],
  "rationale": "A metric thread is drilled to the tap-drill size and then thread-milled."
}
```

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
  { "id": "ws-top-07", "feature": "hole:4", "operation": "thread_milling", "tool": "tm4",
    "because": ["d-012", "d-031", "d-044"] }
],
"decisions": [
  { "id": "d-031", "stage": "tool_selection", "subject": "ws-top-07",
    "question": "which tool for thread_milling M6x1 in hole:4",
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
- **Which outputs rely on a rule:** search `rule` across plans. The ISO 2306 table is an example.
- **Reproducibility:** the plan records the rulebase version and hash, so the same inputs and
  rulebase give the same plan, byte for byte. A test enforces this.
- **Unplanned features** are decisions too, with the rule that excluded them. This replaces
  today's `skipped` and `warnings` prose.
- **The page** can show this: click a toolpath, and see its decision chain.

## CAPP's internal stages

Each stage is a pure function over the documents above, testable alone:

1. **Interpret requirements.** Resolve threads, fits and finishes into concrete targets from
   reference data. For example, `M6` becomes major 6, pitch 1, a minor diameter and a tap drill.
   An `H7` hole becomes a tolerance band, which in turn needs reaming or boring.
2. **Setup planning.** Choose orientations, stock and work origin. This is today's `SETUPS` and
   `Frame`.
3. **Process selection.** For each feature, list the alternative operation sequences and keep
   those whose conditions hold. Then pick by preference, and record the rest as considered.
4. **Resource selection.** Choose tools and cutting data. The tool library uses ISO 13399
   properties; cutting data is looked up by ISO 513 material group and tool material.
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
| `plan.rs` setups, frames, stock | CAPP stage 2 |
| `plan.rs` `COARSE_PITCH`, `thread_size`, `full_thread` | reference table plus a named formula rule |
| `plan.rs` `threads()` reading Specify's answers | Specify adapter (requirements in `part.json`) |
| `plan.rs` `hole_geometry`, `slot_operation`, `notch_operation`, `v_channel_operation`, `wall_operations`, `edge_operations` | CAPP stage 3 rules, with their geometry as predicates |
| `plan.rs` mesh geometry (`room_beside`, `occupied_above`, `clear_below`, `surface_error`, slicing) | geometric predicates module |
| `job.rs` `Library` (`drill`, `pocket_mill`, `v_bit`, `thread_mill`, …) | CAPP stage 4 rules |
| `job.rs` `settle`, `wall_refusal`, `v_channel_refusal` | CAPP stage 5 |
| `job.rs` manifest building, `cutting`, `assembly`, depths | camlib adapter (phase 3) |
| `lib.rs` retry loop and `remove_operation` | `exclusions.json` round trip |
| `post.rs` | unchanged (phase 3) |
| `demo/tools.json` | `resources.json`, its tools in ISO 13399 property names; its placeholder numbers marked as shop policy |

## ISO standards: what can be encoded, and how

- **Data, not text.** ISO standards are copyrighted, and the rulebase must not copy their text.
  Tables of values such as thread pitches, tap drills and fit limits are what CAPP needs. Each
  table cites its standard, edition and clause, and the values are checked against the standard
  itself, not taken from secondary sources. That needs access to the standards, for example the
  ones you own or a library.
- **Data models.** The ISO 14649 / ISO 10303 EXPRESS schemas are published for implementers.
  The JSON schemas can follow them closely enough that an AP238 export is a mapping, not a
  redesign.
- **What specify-core must supply** for requirements to matter:
  - thread designations, which it already gives through Specify's answers;
  - fits and tolerances on holes;
  - surface finish;
  - material.
  Where it does not, the adapter fills in ISO 2768 general tolerances as the stated default, and
  the trace says so.

## Steps

Each step keeps the existing G-code goldens and the parity tests identical, unless the step says
it changes output.

1. **Schemas for what exists.** Describe today's inputs and plan output as JSON Schema and
   validate them in tests. No behaviour change.
2. **Trace scaffolding.** Give every existing hard-coded rule an id and metadata (an inventory),
   and have each one emit a decision record. `skipped` and `warnings` become decisions. The
   output gains a trace, and the G-code stays identical.
3. **Reference data out of code.** Move `COARSE_PITCH`, the full-thread rule and the tolerance
   constants into `rulebase/reference/`, citing sources.
4. **Separate decision from execution.** Move tool selection and `settle` from `job.rs` into CAPP.
   `job.rs` becomes the camlib adapter, reading resolved tools from the plan. Then replace the
   retry loop with `exclusions`.
5. **ISO 14649 vocabulary.** Plan format v2 with ISO names. Add the Specify adapter and
   `part.json`. The page reads v2.
6. **Rules as data.** Move process alternatives, tool ranking and precedence into
   `rulebase/rules/`, with a small evaluator. The rules that remain functions keep their ids.
7. **New knowledge from ISO** (changes output):
   - ISO 286 fits, so that reamed or bored holes are chosen.
   - ISO 513 material groups for cutting data.
   - An ISO 13399 tool library.
   - Surface finish selecting finishing passes.
8. **Optional:**
   - AP238 (STEP-NC) export.
   - A second execution engine.
   - The page showing decision chains.

Steps 1–4 restructure without changing behaviour and could land as separate PRs. Step 5 changes
the plan format. Steps 6–7 are where the rulebase starts to grow.

## Decisions needed before starting

- **Where CAPP lives:** a module in this crate, a separate crate in this repository, or its own
  repository.
- **Rules format:** JSON, matching JSON in and out, or YAML/TOML for easier hand editing.
- **Material:** from Specify (an answer the user gives), or from `resources.json` per job.
- **Standards access:** which ISO standards you have to check the tables against.
