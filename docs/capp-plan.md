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

Settlement caps roughing at stock top minus `min(flute_length, stickout) - 1 mm`, before
camlib sees it. This is the stopgap for stevecad/camlib#122; once camlib checks each layer's
cutting depth, the flute limit can be relaxed to holder clearance. The plan keeps
`covered_by_roughing` as feature ids and records their conservative lowest Z and family in
`roughing_feature_depths`, in the rough-from frame. Features extending below the cap are
skipped with a reach reason and needed-tool query, including the 1 mm margin in both the
minimum flute length and stickout. Flats below the cap are removed; crossing wall bands are
finished down to the cap, with a needed-tool warning for the allowance left below it. Wholly
unreachable wall bands are removed, and dependent edge chamfers follow the missing-wall
handling. No second roughing setup is added. The resource-aware `capp::flip::plan` option can instead reserve sacrificial bottom stock before planning, so the jaws do not obstruct the part.

A vise with explicit `max_jaw_opening_mm`, `jaw_width_mm` and `jaw_height_mm` also
records `setups[].clamping`: the stock faces gripped, grip height, jaw top Z, clearance,
and named jaw/support boxes in the setup frame. The `machine.clamping` rule and
`rules/workholding.json` choose the long stock sides across the narrower XY dimension
(X on a tie), gripping at least 3.175 mm or 20% of stock height, up to 50%. These values
are step2cam judgement: no applicable public source for a minimum grip of standard vise
jaws was found. They are not a holding-force calculation. Stock and the faced part top
must stand above the jaws. Missing jaw data means no inferred dimensions or fixture scene,
and the trace says so.

Side cuts stop at least 1 mm above the jaw top, or higher if tool reach requires it.
Roughing, flats and wall bands reuse the reach cap; open pockets at gripped stock edges
retain their upper cut without breakthrough. Outside chamfers are left out if the selected
cutter's tip would descend below the cap; their width is not silently changed. Closed
interior features retain their depth.
Features and wall allowance below the cap are reported as remainders requiring another
setup, without suggesting a longer tool would solve the clamping obstruction.
Each jaw is passed to camlib as a fixture prism, checked against the whole tool assembly.
Jaw thickness is conservatively modeled equal to jaw height. Without `parallels_height_mm`,
the stock must be raised on parallels or similar so the jaws grip only the chosen height.
That support is not modeled or checked, so through cuts are not checked against it.
If `parallels_height_mm` is supplied, stock seats on two modeled edge bars; grip becomes
jaw height minus parallel height and must satisfy the same limits. Bar envelopes use the rule's minimum grip as
width (or one third of the opening for narrow stock) and span the stock length. Their
prisms are included in verification so through cuts cannot silently enter them.

A rejected grip names the failed limit: minimum grip, jaw height, maximum grip as a
fraction of stock height, or the part top plus side-cut clearance. When parallels set a
grip above the maximum, the trace gives that maximum in mm and calls for taller parallels;
a grip below the minimum calls for shorter parallels.

Clamping proposals use supplied decisions. For example, with 20 mm jaws:

```json
{"decisions":[{"rule":"machine.clamping","subject":"top","choice":{"axis":"x","grip_height_mm":3.5,"parallels_height_mm":16.5},"source":"user","reason":"shop fixture"}]}
```

`axis` selects the setup stock's X or Y min/max face pair, and `grip_height_mm` is required.
If `parallels_height_mm` is omitted it inherits the vise value; explicit null disables modeled
parallels. A specified/inherited height must agree with the given grip and jaw height.
Proposals pass the same opening, grip and part-clearance checks as chosen clamping and are
traced with outcome `given`. Invalid proposals are refused with their reason and the rule
chooses clamping instead. Absent setup IDs are traced refusals. A proposal needs complete vise
dimensions. `resources.overrides.clamping` is deprecated but still translated to supplied
decisions with source `user` and a reason identifying the deprecated field.

Later jobs still start from fresh stock (camlib#121), but their grip bands are audited now.
After caps, exclusions and tool resolution, each later setup's two stock-face contact bands
(from stock bottom to jaw top, limited along the jaw width) are transformed through its
rotation and `origin_model` into each earlier setup's frame. The audit compares them with
conservative cutting envelopes of retained, tooled earlier operations, using their depths
and hole/slot/pocket bounds where available. It warns of possible removed grip material and
records the overlapping operations/faces in `chosen.prior_setups` of the given/chosen
clamping decision, with the audit decision also linked from `clamping.because`.
Roughing and contour envelopes include preserved part material; grouped-hole boxes include
the space between holes. Missing frames or unsupported geometry are reported as unchecked.
This is a conservative warning, not a stock simulation, proof of retained contact area, or
holding-strength calculation. For ordinary setups no operations are removed by this audit. Execution retries
recompute it after refusals, so excluded earlier operations do not count as cuts.

`capp::flip::plan(analysis, mesh, resources)` implements the self-contained #86 vise
flip-and-face option. Selection is explicit in the resources:

```json
{"overrides":{"workholding_option":"vise_flip_face"}}
```

Without `overrides.workholding_option`, or with `"none"`, `process` keeps the ordinary
machine-independent plan and its machining behavior, including the demo's default plans.
It adds a trace decision recording whether the flip was available and that it was not
selected. Only `"vise_flip_face"` requests the flip. Availability requires a dimensioned
vise, a web or through feature, and the seating, grip, opening and reach checks below.
An unavailable selected option keeps ordinary planning and traces the reason, including
missing vise data, ineligible parts, explicit clamping proposals, and execution fallback.
Unknown values (including null and non-strings) are rejected during resource normalisation.
The switch chooses an option directly; a rule choosing between options belongs to #94.
No options table, supplied-decision protocol or automatic choice is introduced.

Stock in this option includes bottom stock B, solved from
`B >= max(3.175, 0.2 * (part_height + top_stock + B)) + 1 + 1`.
The first 1 mm is the existing side-cut clearance; the additional 1 mm facing allowance
is step2cam judgement. B is rounded upward to 0.0001 mm for numerical safety, not a machining
tolerance. Supplied parallels prescribe grip as jaw height minus parallel height; both
setups must satisfy the grip limits or the option is withheld. Op 1 roughs and finishes
the complete outline to the part bottom, with no part web; through holes and slots break
into the sacrificial stock by up to 0.5 mm (judgement, capped by the library breakthrough,
to retain tool reach). Op 2 faces that stock to the bottom face at thickness. With
this stock rule Op 1 reaches the whole outline, so no invented bottom wall operation is
needed; genuinely bottom-entered features retain their setup.

Op 2 measures real planar face area in each opposed jaw band by clipping mesh
triangles. Each side needs at least 60% of the nominal band area and 3.175 mm of
effective contact length (area divided by grip height). These are judgement values
in `workholding.json`, not a holding-force calculation. Holes, pockets and edge
breaks reduce contact instead of rejecting the entire band. The retained contact
polygons, area, fraction and effective length per side are recorded in seating,
clamping and the trace. A top round/chamfer leaves the actual contact below the
break; the nominal jaw top and seating height remain physically consistent. When
minimum grip gives too little contact, search grip upward in the same 0.25 mm
steps, within the existing 50% and jaw-height limits; use the least acceptable
grip. The 16 mm support search covers large edge chamfers on benchmark-sized
blocks; bars cannot overlap each other.

Each parallel footprint needs 75% horizontal face coverage and 3.175 mm effective
support length (area divided by bar width), also judgement. Two 3.175 mm wide bars
search inward up to 16 mm, in 0.25 mm steps, to seat beyond edge breaks. Support
area, fraction and offset are recorded per bar; material projecting below the
seat in the flipped frame must not interfere with their footprints. The most
complete opposed contact is preferred, with narrower-axis precedence on a tie.
Missing support faces or insufficient side contact withhold the option with the
measured reason. Numerical area tolerance is 1e-6 relative, and 0.0002 mm frame
matching covers compounded four-decimal serialization; neither is a manufacturing
tolerance. Missing parallels are chosen at the required height; supplied parallels
must pass the existing grip limits.

The clamping-history audit requires retained, tooled wall contour segments covering
all real contact polygons at their depths, and retained facing/flats at the support
level. Both are recorded as positive evidence and required even if no earlier cut
overlaps the grip bands, including when roughing and all finishing were refused.
An overlap-free audit alone never establishes a grip. Clamping is described as a
proposal until retained operations confirm it. Several segments can prepare a side interrupted by notches. Modelled opening
operations are recognised by their original geometric inputs: their missing contact
is already measured from the final mesh. New or changed cuts still run through the
conservative envelope audit; other setups retain that audit unchanged. Exclusions
recompute preparation. If a grip or facing becomes unavailable, `process` replans
without the flip, traces the reason and resets operation-index exclusions (which
belong to the previous plan). It then retries ordinary planning/execution normally.
A direct `capp::finish` caller also gets an explicit warning that the part remains on
B mm of sacrificial stock and needs facing by hand or another setup; its bottom
operations are withheld. Thus an unavailable featureless facing cannot disappear
without explaining the unfinished part.

Until camlib#121 / #45, Op 2 starts from a fresh part box plus bottom stock. Its trace
warns that this does not model the true underside shape, stock history, or Op 1's margin
remaining below the jaw line. Facing explicitly overlaps the box by the original XY margin
to clear this material; a facing cutter too small for that overlap withholds the option. Jaw and edge-support fixtures and
the transformed target mesh remain checked by camlib; box stock is not a simulation of
physical seating or a holding-strength calculation.

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
| ISO 2306 tap drills | M3–M6 coarse Table 1 values checked against published preview; D − P check |
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
   rejects stock too wide to hold when jaw dimensions are supplied (otherwise retaining the
   opening warning), its spindle caps the speed (feeds scaled) and leaves out tools
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
- **Surface finish and ISO 2768**, once a rule uses them.
- **Optional:** an AP238 (STEP-NC) export; a second execution engine.

### Tapping

For 3-axis milling, `tool.thread.tap_or_mill` prefers matching M3–M6 taps in ISO 513
N material; the size/material cutoff is step2cam judgement, informed by
[Sandvik’s tapping guidance](https://www.sandvik.coromant.com/en-gb/knowledge/threading/tapping).
The library `tap` needs nominal `diameter`, `pitch`, `tap_type` (`cut`/`form`),
`chamfer_form`, maximum `chamfer_pitches`, `thread_length`, `shank_diameter`, mounted
lengths/holder and spindle rpm. Hand defaults right; no independent feed is accepted.
Spiral-point flutes are through-only. Supplied thread operations may set `hand: "left"`;
camlib then mills internal climb threads CCW from top to bottom. Specify LH designations
are not parsed yet. Through threads carry `exit_clear_mm` from the hole and group separately
when exit clearances differ. Tap selection compares the shared `(chamfer_pitches + 1) ×
pitch + guard` travel with that clearance after allowing for the drill point and exit margin;
insufficient room selects a thread mill instead of dropping the thread in the job. OAL and DCONMS aliases have the same dimension
conventions as other tools; thread/lead properties remain unmapped.

[`tap-drills.json`](../rust/capp/rulebase/reference/tap-drills.json) holds the cut drill
table checked in the ISO 2306 Table 1 preview (which also gives minor diameter limits), and the separately sourced
[Tapmatic form formula](https://www.tapmatic.com/tapping-formulas.php) at a shop assumption
of 65% thread. Only the cut rows are checked against the standard preview; the form formula is not. The model bore
must be within 0.02 mm of the reference; drilling diameters are never changed.
A blind tap must cover full thread plus its chamfer and leave one pitch of drilled
bottom clearance. Thread length and stickout reach from stock top to tip plus camlib’s
guard. Rejections include the numeric depths. With no suitable tap the existing
thread-mill selector runs. Missing-tool text offers a cut tap only when the same material,
size and pre-drill conditions pass. Blind queries give maximum chamfer (mm and pitches)
and the corresponding stock-top reach, rounded down/up to 0.01 mm; the reach includes
camlib's numerical guard. Through queries give reach plus the tap's chamfer.

Camlib receives rigid `tapping`, with synchronized feed = pitch × rpm. Through travel
includes chamfer plus one pitch: the job conservatively extends simulated solid backing
below the exit and drills that space first, checking actual exit restrictions before tapping.
The simulation-only bound never changes unrelated drilling/bore floors, breakthrough
allowances or the verification base: these use the real setup stock bottom. The adapter
compares a stock view intersected with the real stock box, against the unchanged target;
this excludes artificial backing, while gouges within the real stock still fail. That report
has no per-motion sweep proof attached because the stream proof covers the extended box.
Tap notes state actual tap-tip and drill-tip travel below the part, including the drill point,
so the operator can keep parallels clear.
Unlike thread milling, tapping keeps the normal part/stock comparison because its
simulation cutter is the pre-drill diameter and retains thread flanks.

### Drilling depth

The drill selector requires flutes reaching from stock top to the modeled bottom, plus the
requested nonnegative breakthrough for through holes, each candidate's conical point length
(`diameter / (2 * tan(point_angle / 2))`) and camlib's 16 × 1e-5 mm guard with 1 nm for roundoff. Rust computes the
virtual `tool.point_length` field; no trigonometry is added to the rule language. The point
allowance is deliberately conservative for tip-referenced bottoms and obstructed exits.
Missing-tool queries restate this same reach requirement, rounded upwards.

`reference/drilling.json` records the public sources and parameters for conventional drills:
one plunge through 3×D, then full-retract pecks of 0.5×D. Depth is measured from stock top,
including requested breakthrough, excluding point allowance. Both Haas and Hannibal agree on
the 3×D onset. The half-diameter increment lies in both Nachi and Harvey
Tool’s published ranges. Applying it as a constant across materials is a conservative starting
policy, not a universal manufacturer value. Full retract is used at intermediate and greater depths because the
tool schema has no through-tool coolant declaration, required by Haas's high-speed chip-break
guidance. A positive tool `peck_mm` caps the rule increment; zero or absent means no cap,
and shallow holes still use one plunge. The settled operation's `technology` carries these
camlib options and its trace cites `process.hole.peck_by_depth`. Above the reference's 5×D
threshold, the plan also warns with the hole feature IDs and depth/D ratio that deep-hole
drilling (pilot plus deep-hole drill) is not planned, so full-retract pecks are used. The warning
is linked to the operation in its trace. At or below 5×D this warning is absent; the threshold
uses the same stock-top depth (including requested breakthrough) and selected drill diameter
as the peck decision. It is a planning warning threshold, not a universal drill capability limit.

Gun drills remain outside the library: the adapter's conical, two-flute shape and generic
coolant settings cannot faithfully describe their single-lip geometry and coolant needs.

### Chamfers

Modelled equal-leg 45° top chamfers use `edge_chamfer`, with `width` as the leg and
no `radius`. A whole outside loop needs its wall finished; hole and slot mouths
need their bore or slot at size. Interrupted convex perimeters may bridge removed
notches only when the mesh lies inside the bevel. Other edges stay out with a reason.
The largest 90° chamfer mill that covers the full leg plus 0.5 mm tip clearance
and fits the loop is selected. Camlib lowers that cone in width steps of at most
1 mm; these passes split the load but cannot extend its conical cutting edge.
Deburr breaks finished edges and also requires the width to fit its cone.
Camlib measures chamfer flute reach from the job's stock top, including air above
a lower flat. Such a chamfer can therefore need longer flutes than its local leg.
Geometry refusals explain the cause and leave the edge for hand chamfering. They
do not request tools: a larger cone cannot resolve an unsupported loop. Missing-tool
queries apply to planned operations whose geometry can be cut.

Mouth chamfers record their required hole/slot features in `needs_openings`. After
tool resolution, an absent or tool-less opening blocks its chamfer and supplies the
reason. A combined round-as-chamfer operation is conservatively blocked as a whole
if any required opening cannot be cut.

### Rectangular recesses

Rectangular slots and pockets use the `notch` operation, including edge-open blind slots.
Their `outer` vertices retain the model's corner arcs; `corner_radius` records that radius
(zero for sharp corners), and `inside_corners` is four for a closed recess or two for an
edge-open recess. The largest end mill narrower than the width whose flutes reach the depth
cuts it. If its radius exceeds the model radius, `process.notch.corner_radius` warns in the
plan that those inside corners retain the cutter radius. No dog-bone relief is generated:
the analysis and answers currently have no way to request it.

Matching upright concave corner fillets can recover straight-wall records' missing ends and
radius. A corner pair implies an open opposite end only when the mesh clears the full run-out;
otherwise the previously validated interior cut is retained for that partial record.

### Spot drilling

`finish` inserts `spot_drill` groups after first settlement, before replaying any
execution exclusions. Selection uses the same twist-drill selector as hole resolution.
Each spot retains its centre, mouth Z, diameter, tip depth and feature ids; the adapter
splits a group into drilling cycles by absolute tip Z, without changing tool. Pruning
runs after each replayed exclusion, so later indices match the plan the engine saw;
no refused spot is recreated.

The public [Harvey guide](https://www.harveyperformance.com/in-the-loupe/choosing-spot-drill/)
and [MSC application guide](https://www.mscdirect.com/resources/buying-guides/drills-countersinks)
were read on 2026-11-08. Both advise a spot point angle equal to or greater than the
following drill's, so the drill starts at its centre. Harvey also describes using the
spot to chamfer a hole, and plunging a larger spot tool only to the required diameter.
These support `process.spot_drill` and `tool.spot_drill` in the rulebase.

We require full diameter coverage rather than falling back to an undersized spot.
Depth is `(diameter / 2) / tan(point_angle / 2)`, measured to the tip. A countersink
is finished only with full diameter coverage and angle agreement within 1 degree;
otherwise it keeps its hand-finishing skip. The adapter uses no peck and a 0.2-second
dwell for a clean spot/chamfer. `process.json` classifies feature families rather than
operation kinds, so spot drilling needs no new feature-family case there.

For an enlarged countersink (larger than the following drill), the reviewer-approved
spot-and-chamfer rule uses the countersink angle alone, matched within 1 degree. The
selector receives a zero minimum angle for this query. Ordinary spots still require
a point angle at least as large as the following twist drill's.

### Supplied decisions

`resources.decisions` is an array of `{rule, subject, choice, source, reason}` objects.
`rule` accepts the trace rule ID with or without `@version`; the installed rule validates it.
`subject` is exactly the trace subject. Sources are `user`, `agent`, or `agent:<name>`
(for example `agent:shop`); `reason` explains the caller's choice.

This round supports tool IDs for `tool.hole.drill_or_bore`, `tool.counterbore.bore_mill`,
`tool.slot.bore_mill`, `tool.notch.pocket_mill`, `tool.thread_mill`,
`tool.v_channel.v_bit`, and `tool.edge_chamfer.chamfer_mill`, plus the clamping object above.
Hole choices may be drills or boring end mills. Every tool passes the same selector keep
conditions and inputs as the rule. Clearing and follow-roughing tools, spot drills, setup
order, and all other decisions are not suppliable this round; clearing still uses
`roughing_tool` and `facing_tool`.

When several operations share a subject such as `top:holes`, include `operation` with the
execution name from the finished plan, for example `top-5-holes`. Ambiguous subjects are
refused. A tool example (use the actual operation number in your plan):

```json
{"decisions":[{"rule":"tool.hole.drill_or_bore","subject":"top:holes","operation":"top-5-holes","choice":"drill4.2","source":"agent:shop","reason":"reserved drill available"}]}
```

Malformed entries (including unknown rule IDs and invalid sources) return an error from
`finish`. Unacceptable choices, unsupported rules and unmatched subjects are refused:
`requirement.supplied_refused` records the reason and a warning states that the rule's own
choice is used. Rejection never bypasses the rule's ordinary feasibility checks.

Every new trace decision has `source: "rule"` by default; an accepted supplied decision
carries its source and reason. The page shows a source tag with the reason in its title.
Older saved traces without `source` remain schema-compatible.

The plan stores `supplied_decisions`, each with `accepted`, `decision` (the accepted decision
or refusal warning ID), and `refusal_reason` when refused. Copy this list to a freshly planned
plan before calling `finish` to replay choices without asking again. Stored status fields
are recomputed. For the same rule ID, subject and optional operation, the last entry wins;
precedence is stored plan, deprecated clamping, then explicit `resources.decisions`.

Spot creation and pruning use the validated supplied hole tool: boring mills are not spotted,
and a drill's diameter and point angle set its spotting requirements. Since spot groups precede
holes, numbered hole choices are matched against a consistent finished layout. If several layouts
use the same number of supplied choices, or no consistent layout exists, indexed hole choices
are refused with a numbering reason; the rules then choose the layout. A consistent layout
that uses more of the supplied choices takes precedence. Execution exclusions still only prune
spot features; surviving features may be regrouped for the actual drills, and a refused spot
is never recreated.
