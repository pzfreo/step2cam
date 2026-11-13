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

Later jobs carry conservative height-field stock, which can contain excess material and
therefore cannot certify jaw contact. After caps, exclusions and tool resolution, each
later setup clips the finished part's analytically planar mesh triangles to both jaw bands.
Curved tessellation facets do not count as flat faces. Earlier drill, thread and edge-break
cutting envelopes are subtracted: these operations can extend beyond the target mesh.
The remaining areas in mm² appear in `clamping.contact_area_mm2` and the primary clamping
decision's `chosen.contact_area_mm2`. Either jaw with zero certain contact withholds that
setup's operations, with a traced reason. Unavailable contact geometry instead reports
unknown contact (null area and a reason), keeps the setup, and retains the conservative
prior-cut audit. Flip preparation and seating requirements still apply.

Later side jaws use the full part mesh's extent within their grip-height band. A remote
boss outside that band does not set the opening; only analytic planar faces at the chosen
jaw planes count toward certain contact. Stock remains the full carried envelope for cutting.


The conservative earlier-cut audit remains for evidence the part-face check cannot cover,
including missing frames and unsupported operations; its scope and warnings remain explicit.
Target-preserving roughing, facing, flats and wall finishing do not invalidate measured
part material. The mesh check replaces envelope warnings where it proves retained contact,
but does not establish holding force or support. Flip seating and retained grip preparation
remain separate requirements. Execution retries recompute all evidence after refusals.

`step2cam::process` plans and simulates every available workholding candidate through
its own complete `process_planned` run, including camlib refusals, exclusions and flip
preparation checks. `capp::flip::plan` remains the lower-level #86 route constructor.
`plan.workholding = {chosen, source, candidates}` describes the comparison. Stable IDs:

- `vise_ordinary`: Vise, ordinary grip.
- `vise_flip_face`: Vise, flip and face.
- `fixture_plate`: Fixture plate, screwed through holes (unavailable).
- `margins_tabs`: Margins and tabs (unavailable until #87).

Both vise options require positive finite jaw width, height and opening on the first
mill's first listed vise. Vise options are omitted when these are missing. Unavailable
flip candidates carry the actual availability or execution failure reason. Only available
routes are simulated. A candidate planning/execution error makes that candidate unavailable;
other candidates continue. If no candidate is selected, an existing ordinary run is reused. If none are available, `chosen` is null and the legacy plan is
retained for review with an explicit trace explanation that holding is not validated.

Fixture-plate holding is withheld even on parts with through holes: screw-head/tool
clearance, supported seating and residual stock transfer across the flip cannot yet
be represented and checked soundly. For the unavailable row only, potential screw holes are unthreaded Z-axis through
holes, 3.2–9 mm diameter, with two centres at least 25% of the XY bounding-box
diagonal apart (step2cam judgement, not a fixture validation). Otherwise its reason
is "no suitable through holes". No plate fixture simulation or new machine-description
kind is claimed. A later implementation must validate those
before offering this route. The page shows a short reason and keeps the full explanation
in the tooltip and plan JSON. No resource schema extension is needed in this change.

Each available candidate records `finished`, `outline_left_mm`, distinct
`features_left_out` with all skipped reasons, `features_cut`, `covered_by_roughing`,
`setups`, `extra_stock_mm`, `stock_height_mm`, `estimated_seconds`, `tools`,
`tool_changes`, `tools_needed`, `grips`, `risks`, and `shop_needs`. `tools_needed` copies
that candidate run's full missing-tool queries (descriptions, feature IDs and operation
IDs), independently of workholding completion. **Finished** means the planned outline
has no web left, no grip or travel omission remains, a roughing operation establishes
the outline, and camlib execution succeeds. Operations missing tools do not change this
status: they are resource needs, reported separately for each candidate. It means
the workholding route releases the outline and grip stock; unsupported details such as
edge rounds still appear in `features_left_out`. The page keeps "finished" first, then
shows the left-out feature count and a short tool need (for example "needs a thread
mill"); full tool queries appear in the row's tooltip. This is not a dimensional inspection,
proof of holding force, or proof that the material comparison resolved every cell.
Grip fractions divide grip height by that setup's starting stock height; finished-wall
grips are identified and soft jaws advised. Existing simulation limitations remain in
the risks, including unmodeled supports and lack of cross-setup stock history.

Extra stock is **additional thickness over ordinary planning's starting stock** (the
common 1 mm top facing allowance is excluded). Total starting thickness is also given.
Time sums every simulated move's polyline segment length / feed, including rapids at
`rapid_mm_min` or 5000 mm/min, exactly as the page; setup labour and tool-change delays
are excluded. Tool changes count transitions between consecutive move tool IDs across
the whole run, excluding the initial tool load. Candidate consequences are summaries;
the chosen run retains its full programs, logs, trace and checks.

`setup.workholding@1` compares available candidates lexicographically: finished route,
then fewest distinct features left out, fewest setups, least extra stock, shortest
estimated machining time. Exact ties retain stable candidate order. The choice is traced
on subject `part`, source `rule`. All three demo plates default to flip-and-face because
ordinary grip leaves the outline attached. Supplied decisions can select any available
candidate, with `source` `user`, `agent`, or `agent:<name>` and a reason:

```json
{"decisions":[{"rule":"setup.workholding","subject":"part","choice":"vise_ordinary","source":"user","reason":"shop preference"}]}
```

Unknown, unavailable or malformed choices go through the standard supplied refusal
path, then the rule chooses. Accepted and refused requests remain in
`supplied_decisions`; pass this list as resources `decisions` to replay. The legacy
`overrides.workholding_option` is translated into a user request: `"none"` means
`vise_ordinary`; `"vise_flip_face"` means that candidate; its reason names the override.
Explicit decisions take precedence. Other override values still fail normalization.
The Plan tab's Workholding block shows the choice, costs and alternative reasons;
the chosen option has ✓, clickable available alternatives have ○, and unavailable ones
have ✕. Finished-wall grips say "soft jaws advised" on the row. If the chosen route
leaves the outline attached, the block explicitly says the part will not come off finished.
Clicking an available alternative replans with a user decision. Changing part or
machine clears that choice and returns to the default rule.

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
tolerance. The mesh seating defines flip parallels with height equal to jaw height minus
the seating grip, along with the measured support width and footprints. A vise may list
available `parallels_heights_mm` with `_sources`; the seating height must be achievable
from that inventory. Without seating, choose the tallest height compatible with grip limits;
without either seating or inventoried parallels, support stays explicitly unmodeled.
A fixed `parallels_height_mm` still prescribes grip. Chosen support bars
are fixture bodies, alongside the jaws, for through/deep-cut collision checks.

The clamping-history audit requires retained, tooled wall contour segments covering
all real contact polygons at their depths, and retained facing/flats at the support
level. Both are recorded as positive evidence and required even if no earlier cut
overlaps the grip bands, including when roughing and all finishing were refused.
An overlap-free audit alone never establishes a grip. Clamping is described as a
proposal until retained operations confirm it. Several segments can prepare a side interrupted by notches. Modelled opening
operations are recognised by their original geometric inputs: their missing contact
is already measured from the final mesh. New or changed cuts still run through the
conservative envelope subtraction; all later setups also require positive certain contact. Exclusions
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
bottom clearance. Specify's default full thread (drill depth less three pitches) leaves less
than that for a three-pitch lead, so for a default depth (`default_depth` on the thread) the tap cuts the drilled
full-diameter depth less its chamfer and one pitch, rounded down to 0.5 mm, and is refused only
below one pitch; the trace records the depth cut. An asked depth (Specify's `hole.full_thread` answer, else the tapped answer's `@` depth) is never shortened. Thread length and stickout reach from stock top to tip plus camlib’s
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
one plunge through 3×D, then full-retract pecks of 0.5×D. Depth is measured from where the drill enters: after roughing, the hole's mouth
when the roughing cutter fits the hole or a flats pass at the mouth's height sweeps the whole
disc over it (the cutter outreaches the hole by half a stepover), otherwise the lowest roughing
layer that keeps the bottom allowance above the mouth; without roughing, the stock top as faced; or the top of a slot, notch
or V-channel (cut after holes) whose floor it is in; it includes requested breakthrough, excluding point allowance. Both Haas and Hannibal agree on
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
uses the same depth below the entry surface (including requested breakthrough) and selected drill diameter
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

### V-channels

A V-channel is cut by running the widest sharp V-bit no wider than the channel's angle along
it, its tip on the root. A narrower bit leaves a wedge of stock on each wall, widest at the
top: (width - tip diameter) / 2 - depth × tan(bit angle / 2). Above 0.005 mm,
`process.v_channel.wall_stock` warns in the plan that the channel is partly cut and records
that stock as `wall_stock_mm`, as `process.notch.corner_radius` does for corners.

### Spot drilling

`finish` inserts `spot_drill` groups after first settlement, before replaying any
execution exclusions. Selection uses the same twist-drill selector as hole resolution.
Each spot retains its centre, mouth Z, diameter, tip depth and feature ids; the adapter
splits a group into drilling cycles by absolute tip Z, without changing tool. Pruning
runs after each replayed exclusion, so later indices match the plan the engine saw;
no refused spot is recreated.

After tool resolution, `finish` takes out every operation no tool cuts (`tool.none.left_out`)
and numbers the rest again, as after an exclusion. Its features are skipped with its reason and
need. Spot drilling left out this way does not skip its holes, since they are still drilled;
instead one warning per setup (`tool.none.unspotted`) names them, for example
`setup top: 9 holes drilled without spotting: no usable spot drill: needs …`. Execution names
in exclusions and supplied choices count only the operations the finished plan has: an
exclusion is mapped back by resolving a copy of the plan as the engine saw it.

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
`tool.v_channel.v_bit`, and `tool.edge_chamfer.chamfer_mill`, plus the clamping object above
and `setup.workholding` on subject `part` (candidate ID, checked by `process` after simulation).
Hole choices may be drills or boring end mills. Every tool passes the same selector keep
conditions and inputs as the rule. Clearing and follow-roughing tools, spot drills, setup
order, and all other decisions are not suppliable this round; clearing still uses
`roughing_tool` and `facing_tool`.

When several operations share a subject such as `top:holes`, include `operation` with the
execution name from the finished plan, for example `top-5-holes` (operations no tool cuts are
not in it, so they do not count). Ambiguous subjects are
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

Wall finishing axial engagement is chosen by `process.wall_finish.axial_passes`:
stock-top-to-floor depth divided into the fewest equal passes no deeper than the
selected tool's flute length. This replaces the roughing stepdown for finishing
only, in operation technology consumed by the camlib adapter. Full reach and
stock-contact checks still apply; equal layers do not excuse insufficient reach.
The adapter leads a wall finish onto and off its loop along the tangent, one
cutter radius long, where the outline is convex (`process.wall_finish.lead`):
camlib's contour takes straight tangent leads only and chooses where the loop
starts, so on any other outline a lead could cut into the part and the cutter
plunges at the wall instead. The lead extends the loop's first stroke backwards,
so it leaves the wall only where the loop starts partway round an arc. Where the
loop starts on a straight wall, or where an arc meets one (the usual case on a
rectangular outline), the lead runs along that wall's tool path and the cutter
still comes down touching the finished wall, one radius along from the loop
start. Taking the plunge off the wall there needs an arc lead or a choice of
loop start, which camlib does not offer.

An optional machine `spindle.collet_change` (`manual` or `automatic`) describes
swapping the spindle collet, separately from swapping complete tool holders.
`manual` produces a traced setup warning (`machine.spindle.manual_collets`) when
selected tools require multiple shank sizes. Absent leaves this check unspecified;
preassembled CAT40/ER holders need not claim manual spindle-collet changes.
Unused inventory tools do not affect the note. No tool choice changes.

Sequential simulation carries camlib's finished-stock mesh into the next setup using the
recorded rotation and model origin. The mesh is conservative height-field stock, including
filled voids and undercuts; it is not a measured part. Only setups with a successor export
stock. The receiving manifest encloses the carried mesh. The existing adapter derives its grid,
retract and clearance planes and tool-change position from that envelope, respecting the
library's configured values. The flip facing needs no second residual-margin overlap.
Camlib alone decides tool contact and material acceptance.

`process` adds two optional setup fields: `stock_note`, the actual initial-stock premise
(or an explicit freed, part-sized block assumption with the carry refusal), and
`before_setup`, a manual separation instruction immediately before reclamping. A `break_out`
entry describes a thin web below the part bottom and its witness. A `cut_free` entry says
that the part is not free, names the uncut wall band from the settled roughing floor, and
requires cutting free (for example sawing) and finishing that band in a later setup.

Before the part is free, the complete finished stock is carried. After the flip route's
facing, or a declared manual separation, cells outside the source-frame part footprint
expanded by one source cell are cleared to no material. Camlib's public height-field mesh
builder rebuilds the closed mesh on that source grid. The receiving stock note declares:
"the part is free, so stock outside its footprint is dropped". This is an explicit
separation premise, not a simulated manual toolpath or an added machining plan.
Capp's frame/box decision remains unchanged; execution premises live in `stock_note`.


### Vise grip against the cut (M3)

`machine.grip_vs_cut` screens each gripped setup after tool resolution. A vise may
supply positive `clamp_force_n` and optional `clamp_torque_nm`, with maker URL, date read
and explanation in `_sources.clamp_force`. Missing force gives one “grip not checked
against the cut: the library gives no clamping force for <vise>” warning per setup;
cutting data remain unchanged. The demo placeholder has no force.

The Haas DX6 uses Kurt’s manual row **3440 lbf = 15301.88235649612 N at
40 ft·lbf = 54.232717933256 Nm**. Choosing 40 ft·lbf is step2cam judgement:
a moderate hand tightening, half the rated maximum of 80 ft·lbf. The
[Kurt DX6 manual](https://docs.travers.com/assets/pdf/61-421-171_manual.pdf), read
2026-11-13, gives this force table on printed/PDF p4 and supplied-handle guidance
on p5. The manual's 10 ft·lbf row is 512 lbf; it supersedes the rounded product-page
table for this plan. Both `setups[].clamping.force_check` and candidate `grips[].force_check`
carry clamp force, assumed torque, friction, holding/allowable forces and source,
alongside the trace, for a future setup-sheet line. This does not change the setup sheet.

Holding force is `μ × clamp_force_n × contacted jaw faces`. Use **μ = 0.095**:
SMW-AUTOBLOK KNCS-N manual p32, rough to medium machined side / smooth steel jaws
0.10 with aluminium correction 0.95, appropriate for the machined or sawn side.
The 0.07 cell is for ground/smooth-finished workpieces, not this premise.
This agrees with the 0.1 steel/aluminium fixturing model in
[Zeng et al.](https://www.jstage.jst.go.jp/article/ijat/15/6/15_860/_pdf/-char/en).
The [SMW manual](https://hod-industrial.hu/wp-content/uploads/2023/06/HOD-SMW_kncs-n_en.pdf)
also gives safety factor 1.5–2; choose **2**. Both read 2026-11-13. Applying these
workholding values to a vise is step2cam judgement. Grip height does not multiply
Coulomb friction capacity: small contact area still needs pressure and moment checks.

The [ISCAR aluminium guide](https://www.iscar.ru/Catalogs/Publication/english_1/ALUMINUM_MACHINING/ALUMINUM_MACHINING.pdf),
p161, USER GUIDE / MATERIAL GROUPS, read 2026-11-13, gives hardenable wrought ISO N group 22
**kc₁ = 800 N/mm², m꜀ = 0.25**. That row is our conservative aluminium proxy
(step2cam judgement); other materials warn that no applicable reference is implemented.
For milling, `fz = feed/(rpm × flutes)`, `φ = acos(1 − 2ae/D)`,
`h = fz(1 − cos φ)/φ`, `kc = kc₁h^(−m꜀)` and
`F = kc × ap × ae × feed/(π × D × rpm)` N, equivalent to removal power / cutting speed.
Each milling operation uses its resolved feed, rpm and engagement. Facing and flats use
`ae = min(stepover, D)`, valid through full-width engagement. Roughing uses stepover when `entry_access: "stock_edge"` classifies every region as
edge-entered; `"closed"` uses full D for the first entry. An absent or `"unknown"`
classification conservatively assumes full D. The built-in planner does not select
the entry of each subregion, so leaves it absent; an outer air plunge alone does not
prove that no interior region requires a helix. Over-capacity warnings state this
unknown-entry full-slot premise explicitly. Roughing ap checks both end layers
against the faced surface and the original layer schedule; middle layers may still
cut a full stepdown.
Pockets and helix-milled holes use full-slot engagement, walls the radial allowance,
and flats the roughing step envelope below the faced top. Helix-milled holes and
counterbores use min(material depth from the actual starting surface, pitch per
revolution). The pinned bore emits one revolution per operation stepdown, with a
shortened last revolution: mounting-plate currently executes one 9.5 mm revolution
under its nominal 10 mm pitch, rather than the 1.3 mm/rev a 3° helix would imply.
Changing its toolpath is outside this force audit; no fictional smaller pitch is used.
These envelopes and default flute counts
(3 for milling tools, matching the adapter) are step2cam judgement, recorded in the rule.

Drilling, spotting, tapping and concentric boring record “thrust into the supports;
not a sliding load on the grip”, without a sliding over-capacity warning. A drill's
opposing lip forces form a torque couple, not a net sideways force. We choose this
simpler sound classification instead of a torque check needing an assumed grip lever
arm; neither torque capacity nor support thrust capacity is checked. The adapter's
end-mill hole boring is helix milling; it and thread milling retain the sideways check.

Compare **2F** with holding force. Facing above capacity gets the largest fitting depth,
rounded down to **0.05 mm** (step2cam judgement), with the final pass taking the remainder.
The trace gives original and per-pass force, depth and pass count; the adapter consumes
the capped stepdown. If even 0.05 mm cannot fit, warn with the cutting and holding forces
that the facing “cannot be held at any depth” and leave it as planned; do not fail the plan.
Other milling operations warn with both forces, the assumed diameter/depth/width
and entry or pitch, the tightening premise, and a computed maximum stepdown remedy,
while keeping their cutting data. Materials without a force/friction reference get
one not-checked warning per gripped setup. This checks
estimated mean tangential sliding load; peak forces, thrust, contact pressure, deformation,
lift, overturning, chatter and support flatness remain unchecked.
