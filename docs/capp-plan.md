# step2cam in three phases: Specify, CAPP, execution

This document describes the design as built (steps 1–6 and the first part of step 7) and the work
still planned. The proposal it started from is in the history of this file.

Decided (7 November 2026):
- CAPP reads specify-core's output as it is. It has no feature schema of its own.
- CAPP is a crate in this repository.
- Rules are JSON for now.
- The material comes from Specify, and `resources.json` can override it.
- ISO data comes from public sources only. No standards are bought.

## Why

Before this work, the logic between feature recognition and camlib was about 2,500 lines of Rust in
`rust/src/plan.rs` and `rust/src/job.rs`, with its knowledge mixed into the code that applied it:
- Standards data was buried in code.
- Rules had no names.
- Decisions could not be traced.
- `job.rs` both decided and wrote camlib's manifest.

A search in November 2026 found no open-source project that encodes this knowledge:
- Open recognisers stop at labelling features.
- Open CAM tools expect the user to choose operations.
- The rule sets in FeatureCAM, CAMWorks, NX and CAM Assist are closed.

## The three phases

```
STEP ─► 1. Specify ─► analysis + mesh ─► 2. CAPP ─► plan (traced) ─► 3. Execution ─► toolpaths, checks
        (specify-core)                    rust/capp                  step2cam: camlib adapter,
                                          + resources.json           camlib's simulation and checks
                                                                     (G-code: camlib's posts, not step2cam)
```

**1. Specify** (specify-core, unchanged). STEP in; the analysis and the mesh out, in specify-core's
own formats. Features, faces, their ids and the answers given in Specify are exactly as specify-core
writes them. The material is Specify's `part.material` answer. The PMI (`haecceity.pmi/1`, whose
schema specify-core publishes) holds threads, fits, tolerances and surface textures; CAPP reads
threads from the answers today and does not yet read fits or finishes.

**2. CAPP** (`rust/capp`, a crate in this workspace). Pure functions, JSON in and JSON out, no I/O and
no camlib:
- `capp::plan::plan(analysis, mesh, options)` decides what to cut in which setup.
- `capp::finish(plan, resources, exclusions)` takes out what these resources cannot cut, and
  what an execution engine refused; places the material in its ISO 513 group; and names each
  operation's tool, cutting data and ISO 14649 names.

**3. Execution** (the `step2cam` crate): the camlib adapter (`job.rs`), which reads each
operation's tool and cutting data and writes the camlib job; the run of each job in camlib's engine,
with its checks (`simulate.rs`); and the browser entry point (`process`). step2cam writes no G-code:
camlib's posts do, from the saved job (`examples/post_with_camlib.py`). An operation camlib refuses goes back to `capp::finish`
as an exclusion (`{"operation": "top-2-roughing", "message": …}`), and the plan is finished again
from the planned one without it.

## Documents and schemas

step2cam's own documents have JSON Schemas (2020-12) in `schemas/`, and tests validate the shipped
tool libraries and every fixture's plan against them. Inputs from specify-core are specify-core's;
its analysis has no published schema yet (`docs/upstream-needs.md`, S1; specify-core#157).

| Document | Schema | Holds |
|---|---|---|
| analysis, mesh | specify-core's | Features, faces, answers (threads, material) |
| resources (`demo/tools.json`) | `resources-1` | The machine, the tools, shop policy values, `overrides`, and per tool optional `cutting_data` by ISO 513 group |
| plan | `plan-1` | Setups and operations (each with `tool`, `tool_note`, `technology`, `iso14649`, `because`), `material`, `skipped`, `warnings`, `decisions`, `rulebase` |

The plan carries `format` and `version`; the tool libraries may, and the shipped ones do not.

## The rulebase

All JSON, in `rust/capp/rulebase/`, embedded in the crate so the WASM build needs no file access:

```
rulebase/
  rules.json                     every rule: id, version, stage, rationale, sources, code, data
  reference/
    iso261-coarse-pitch.json     ISO 261 coarse pitches (two public sources)
    full-thread.json             specify-core's default minimum full thread, as a formula
    iso14649-names.json          ISO 14649 feature and operation names for each operation kind
    iso513-groups.json           ISO 513 groups, and how a material's name is placed in one
    iso13399-fields.json         exact tool property aliases by type, with public sources
  rules/
    tools.json                   tool selectors: which tools qualify, why others are turned down, which wins
    sequence.json                the order of operations in a setup
    process.json                 what becomes of a feature of each family
```

Knowledge is kept apart by kind, so every value has a known kind and source:
1. **Reference data:** values from standards or recognised practice. Each table says where its
   values came from and `checked_against_standard: false`.
2. **Rules:** each has an id and a version, and names its sources: a standard, recognised practice,
   or step2cam's own judgement. Where a rule's logic is data, it names the file (`data`).
3. **Shop policy:** margins, allowances, clearances, preferred tools and cutting data. These live in
   the resources, never in the rulebase, so a placeholder can't pass as a standard.
4. **Geometry:** reachability, the room beside a wall, what lies above a hole and the like stay Rust
   code. The rule files name these conditions, and `process.json` lists what each means.

The numeric tolerances in the code (`LEVEL_EPS`, `AXIS_TOLERANCE`, camlib's surface-error limit,
the comparison budgets) are step2cam's own judgement or camlib's limits, not reference data, so
they stay as named constants beside the code that uses them.

### Rules as data

Tool selection, the operation order and the handling of each feature family are data. The tool
selectors use a small expression language (`rust/capp/src/rules.rs`): numbers, names (the
selector's inputs, its `let` values, `tool.<field>`, the library's `drill_tolerance_mm`, step2cam's
`tip_clearance_mm`), `+ - * /`, comparisons, `&&` (which does not look past a false left side) and
parentheses. It evaluates left to right with the usual precedence, so it does the same
floating-point arithmetic, in the same order, as the code it replaced. A tool field that is missing
or not a number fails every comparison, so that tool is not chosen. Ties go to the first or the
last in library order, as each selector says.

### Traceability

Every choice is a decision citing its rule as `id@version`. Each operation's `because` names its
decisions:
- its setup;
- the order;
- the rule that made it;
- the setup chosen for its holes;
- its tool, with the tools turned down;
- its cutting data.

Each `skipped` entry names its `decision`, and each warning is a decision too. A decision a later
one replaced says `superseded_by`. When an operation is refused, its own decisions (what made it,
its tool and cutting data) are superseded by the refusal; those it shares with its setup are not.

Tests check:
- The rule ids in the code and the rule files are exactly those in `rules.json`.
- Every warning, skip and operation goes back to decisions that exist.
- The same inputs give the same plan.
- If `finish` fails, the plan keeps what it had said and decided.

The plan records which rulebase made it (`rulebase.hash`, a hash over every rule and reference
file).

The page shows the plan in a **Plan** drawer: each operation with its tool, cutting data and ISO
names, and the decision chain behind it, with each rule's rationale and sources.

### Material

The material is Specify's `part.material` answer, unless the resources set `overrides.material`.
specify-core gives the material's name but not its family (specify-core#158), so the name's whole
words are matched against `iso513-groups.json`:
- `stainless` places it in M, `aluminium` and the plastics in N, `steel` in P, and so on.
- A printed material (`(FDM)`, `(DMLS)` …) gets no group.
- A name nothing matches gets no group.
- Tool steel goes in P, as supplied.
- With no group, or no data for it, each tool's own cutting data are used, and the trace says
  which.

### ISO 14649 names

Each operation carries its ISO 14649 names (`iso14649`), from STEP Tools' published AP238 lists.
This is a field on each operation, not a restructured plan: renaming the plan's own structure would
have rewritten the page and the camlib adapter for no change in what they do, while the names are
what an AP238 export needs.

| Operation kind | Feature | Operation | Exact |
|---|---|---|---|
| facing, flats | `Planar_face` | `plane_finish_milling` | yes |
| roughing | `General_removal_volume` | `bottom_and_side_rough_milling` | yes |
| holes | `Round_hole` + bottom condition | `drilling` (drill) / `bottom_and_side_finish_milling` (bored by helix) | drilled only |
| counterbore | `Counterbore_hole` | `bottom_and_side_finish_milling` | no: the feature includes the bore |
| thread | `Thread` | none in ISO 14649-11 | no |
| slot | `Slot` | `bottom_and_side_finish_milling` | yes |
| notch | `Rectangular_open_pocket` / `_closed_` | `bottom_and_side_finish_milling` | yes |
| v_channel | `Slot` (`Vee_profile`) | `bottom_and_side_finish_milling` | no |
| wall_finish | `General_outside_profile` | `side_finish_milling` | yes |
| edge_chamfer | `Edge_round` | none in ISO 14649-11 | no |

## ISO standards from public sources only

There is no access to the standards themselves. The rulebase holds values, never a standard's text.
- **Formulas first:** prefer formulas that follow from a standard's published definitions.
- **Otherwise two sources:** use two independent public sources that agree. Where they disagree,
  take the safer value.
- **Tested:** every table has a test against spot values from its sources.

| Standard | Status |
|---|---|
| ISO 261 coarse pitches | Done: table, two sources |
| ISO 513 groups | Done: groups from Seco and Mitsubishi Materials; placing by name is a stand-in |
| ISO 14649 / AP238 names | Done: STEP Tools' published lists |
| ISO 68-1 / 724 minor diameter | Planned: formula |
| ISO 2306 tap drills | Planned: table from catalogues, D − P as a check |
| ISO 286 fits | Planned: specify-core already computes fit limits |
| ISO 2768 general tolerances | Planned: recorded; nothing uses them yet |
| ISO 13399 tool property symbols | Done: exact aliases by type, public manufacturer lists; current names retained |
| ISO 15065 countersinks | Later, if a source is found |

Cutting data are not standardised: they are shop policy, in the resources.

## Steps

Done:
1. The `capp` crate and schemas for the tool library and the plan.
2. Planning, settling and tool choice moved into `capp`; `job.rs` is the camlib adapter; refusals
   are exclusions.
3. The decision trace, with every rule inventoried.
4. The thread tables as reference data.
5. ISO 14649 names on each operation.
6. Tool selection, operation order and feature handling as JSON rules.
7. (part) The material, its ISO 513 group, and cutting data by group.
8. (part) Machines: the 3-axis mill. The resources' `machines` list what each machine can do; the
   plan is cut on the first mill. Its travels refuse a setup whose stock will not fit, its vise
   flags stock too wide to hold, its spindle caps the speed (feeds scaled) and leaves out tools
   it cannot hold, and its tool changer flags a setup needing more tools than it has pockets.
   Each is a traced decision (stage `machine`). The demo's machine is the Makera Carvera, from its
   published specifications.

9. ISO 13399 tool library aliases: a sourced table, normalized at the library entry points.
   Equal paired names are accepted; conflicting values are refused. Fields without exact
   equivalents keep their current names.

Steps 1–6 left the G-code unchanged. Each was compared with `main`: 8 fixtures and a tapped plate,
with up to 5 tool libraries, one with a twin of every tool to test tie-breaks. Step 7 changes it
only when a material is given and a tool states cutting data for its group. Step 8 changes it only
where a machine limits something; with the example library's generous mill, outputs match.

Planned:
- **The lathe.** A machine of kind `lathe`, and turned features (specify-core's `turned_bodies`,
  `turned_steps`, `face_grooves`) planned on it rather than skipped: setups held in a chuck,
  facing, turning, grooving and parting, each traced, then handed to camlib's turning (#57).
- **More of the mill:** an indexer or trunnion (3+2) merging the setups it can reach into one
  clamping; spindle power limiting cutting data.
- **Fits** (ISO 286, from Specify's `fit:H7` answers): drill under size and ream; camlib has a
  reaming operation. This needs a reamer tool type.
- **Tapping** as an alternative to thread milling; camlib has a tapping operation.
- **Surface finish and ISO 2768**, once a rule uses them.
- **Optional:** an AP238 (STEP-NC) export; a second execution engine.
