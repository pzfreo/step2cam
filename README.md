# step2cam: STEP features to camlib toolpaths

## Read this first: what step2cam is, and what it is not

**step2cam is an experiment in explainable process planning, not CAM you can trust to run on a
machine.** For simple prismatic (2.5D) parts it drafts a plan (setups, operations, tools, order)
and shows the rule behind every choice. Mechanical engineers and machinists will rightly point out
that real process planning is a much harder problem, and step2cam does not model most of what
makes it hard:

- **Workholding and datums:** how the part is located and clamped, which faces are reference
  faces, how a second setup picks up the first. It assumes a part in a vise.
- **Tolerances and GD&T,** which in practice decide the order of operations and what goes in one
  setup. It ignores them.
- **Cutting physics:** tool reach and deflection, chatter, chip evacuation, thin walls, machine
  rigidity. Its rules are simple, and "the largest tool that fits" is often not what a machinist
  would choose.
- **Real cutting data:** every feed, speed and tool in the demo is a placeholder.
- **Real parts:** interacting features, drafts and freeform shapes break its recognition and
  planning. The samples are friendly.

camlib's check proves the toolpaths do not cut into the model; it does not show that the part
will be good: finish, accuracy, burrs, whether it survives being cut. Commercial planners with
decades of rules still produce a first draft that a machinist edits.

**Use it** to see how recognition and planning could fit together, to read a plan's reasoning,
and to argue with specific rules: every decision names the rule it was made by, so "rule X is
wrong here, because…" is the most useful feedback. **Do not cut anything from its output without
someone who knows machining checking it first.**

`step2cam` turns the features [Specify](https://github.com/pzfreo/specify-core) recognises in a
STEP part (holes, slots, counterbores, the part's shape) into toolpaths and G-code with
[camlib](https://github.com/stevecad/camlib), the CAM engine. It is written in Rust and runs in the
browser as WebAssembly, or natively.

Between feature recognition (by [pzfreo/quiddity](https://github.com/pzfreo/quiddity)) and camlib
is a plain JSON plan.

```
part.step ─▶ specify-core: analysis, mesh ─▶ capp: plan, finish ─▶ job::jobs ─▶ post::post ─▶ LinuxCNC G-code
             (quiddity: features)            (what to cut, with   (camlib's    (camlib: how)  one per setup
                                              which tool)          manifest)
```

The planning is the `capp` crate (`rust/capp`): `capp::plan::plan` decides what to cut in which
setup, and `capp::finish` takes out what the tool library cannot cut and names each operation's
tool. `step2cam` is the camlib adapter (`job.rs`), the post-processor (`post.rs`) and the browser
entry point. An operation camlib refuses goes back to `capp::finish` as an exclusion, and the plan
is finished again without it.

## Use

The easiest way is the demo below. Without a browser, `rust/examples/process.rs` takes
specify-core's output and a tool library, and prints the plan, the G-code and the log as JSON:

```bash
rust/fetch-camlib.sh    # camlib at the pinned commit
specify-core analyse -o analysis.json part.step
specify-core mesh -o mesh.json part.step
cargo run --release --manifest-path rust/Cargo.toml --example process -- analysis.json mesh.json examples/tools.example.json
```

## Demo

**https://pzfreo.github.io/step2cam/** opens a STEP file and shows its recognised features beside
the toolpaths that cut them, in one camera. Hover or click a feature, a toolpath or a G-code line
and the others follow; press Play and the feature being cut lights up. Save writes each setup's
G-code as `<part>-<setup>.nc`, with comments at its top saying what the file does not.
Nothing loads until you open a file or pick a sample: the simple plate is specify-core's test
part `plate.step`, and the complex plate is `tests/fixtures/synthetic/stepped-plate.step`.

The page tells the story in three panes, in the order the work happens:
1. **What the part is:** the features specify-core recognised.
2. **How to make it:** the process plan.
3. **Making it:** camlib's toolpaths and G-code.

A bar above the panes sums up each step for the part opened: faces and features found; setups,
operations, tools and decisions; lines of G-code and the gouge check.

In the plan pane:
- **The part:** its material and ISO 513 group.
- **Each setup:** its operations in the order they will be cut.
- **Each operation:** opens inline to show its tool, cutting data, ISO 14649 names and the decisions
  behind it, each citing the written rule it was made by. **Full view** lists every tool it
  considered, with why each was turned down or passed over, and each rule's rationale, sources and
  use in this plan.
- **Problems:** what was refused, what was not cut and the warnings, each with its rule.
- **Save plan:** downloads the plan as JSON, trace included.

Hovering or clicking a feature, an operation, a toolpath or a G-code line lights the same thing in
all three panes.

On a screen narrower than 1300 px one pane is open and the others are rails: click a rail or a
step in the bar to open it. On a wider screen, **Expand** does the same.

It runs entirely in your browser, so your file is not uploaded: `specify-core`'s WebAssembly
package recognises the features, and `rust/` (step2cam's planning and posting, with camlib's
engine) makes the toolpaths. It uses `demo/tools.json`: **placeholder tools and cutting data, not
for a real machine.** An operation camlib refuses, or that no tool fits, is left out and reported.
Machining times are estimates from the feed rates; the timeline compresses them so that holes are
not lost beside roughing.

The toolpath view draws the block of stock being cut away as the program plays (`demo/stock.js`):
a height field per setup, lowered under each cutting move's tool shape, and drawn as what lies
under all of them. It is for looking at, not a check: it cannot show an undercut, and it draws a
thread mill as a flat disc. camlib's comparison of the stock with the part is the check.

```bash
demo/build.sh && python3 -m http.server -d site 8790   # needs Rust, wasm32-unknown-unknown and wasm-pack
```

Every block written is read back and checked by camlib's whole-program intent checker, against
the plan's motions, tools, spindle, coolant and work offsets, and camlib's stock-contact checks
run. A program that fails is not given out. Only LinuxCNC output in mm is wired up.

## The plan (`specify-cam-plan`, version 1)

The plan says **what** to cut, independent of tools and machine.

- **Stock**: the part's box plus `margin` (2 mm) around the sides, `top_stock` (1 mm) above
  and `bottom_stock` (0) below (`capp::plan::Options`).
- **Setups**: one per direction a hole must be entered from (`top`, `bottom`, `front`, `back`,
  `left`, `right`). Each setup is the part turned so its tool comes down -Z, then zeroed with
  **X0 Y0 on the minimum corner and Z0 on the top** of:
  - the **stock**, in the `rough_from` setup (the first; default `top`);
  - the **part's own box**, in every later setup. By then roughing has cut away the stock
    around the part, so zero on the part, after removing the web roughing leaves (below).

  Each setup records its `zero` (`stock` or `part`), its `rotation` and `origin_model` (the
  model point at X0 Y0 Z0).
- **Operations** in each setup, in setup coordinates:
  - `facing` down to the part's top (the `rough_from` setup only);
  - `roughing` of the whole part from the mesh (the `rough_from` setup only, needs the mesh).
    It stops `skin` (0.5 mm) above the part's bottom, so it leaves a web that thick all round
    the part (`web_mm`), which no operation removes. It also clears the side margin, which is
    where vise jaws on the stock would be: hold the stock from below (a fixture plate, screws or
    double-sided tape) or leave more margin;
  - `holes`: holes of one diameter and depth, with their centres, bore top, bottom and how the
    bottom ends (`through`, `flat`, `drill_point`, `unknown`). A through hole also carries
    `exit_clear_mm`: how far beyond its exit there is no part material, `null` when it exits
    the part altogether;
  - `counterbore`: the enlargements at hole mouths (spotfaces, counterbores, stepped mouths),
    widest and outermost first;
  - `thread`: internal threads, from Specify's answers (see [Threads](#threads));
  - `slot`: a closed slot with round ends, cut to size with a camlib `Pocket` of its outline and
    the largest end mill that fits, after roughing, from whichever setup reaches it. Other slots
    are left to roughing;
  - `notch`: square-ended slots and notches, as a pocket with the largest end mill that fits and
    reaches, cut past the part on a notch's open side. specify-core's record says where the box is,
    and the mesh must show it empty above the floor, or it is not cut. Its inside corners stay as
    round as the cutter;
  - `v_channel`: a V-channel that runs out of the part at both ends, with a sharp V-bit no wider
    than the channel, its tip along the root. A narrower bit leaves a little on the walls, and the
    log says how much. A channel in a flat goes with that flat;
  - `flats`, straight after roughing, where the part has upward flat faces between its top and
    the roughing floor. Those are off the roughing layers, so they can have up to a stepdown (plus
    the bottom allowance) left on them; camlib's `Flat` cuts each at its exact height, in one pass
    with the roughing end mill, staying a tool radius from walls. camlib proves that clear of the
    part, not how much stock is in the way, so a flat that would cut deeper than that, where
    roughing did not reach, is refused and left out with the walls that stand on it;
  - `wall_finish`: the outside of each solid outline with upright walls, to size, in each band
    between flats, so a raised boss's wall stops at the plate it stands on, climb milled. It needs
    roughing to have run. A contour knows only its outline, so anything of the part above the floor
    and within reach of the cutter stops it: `room` is how far away the nearest such face is, 0 for
    an overhang or a flared foot. With less than the end mill's diameter plus twice the radial
    allowance, the wall keeps the roughing allowance, as a sloped wall or the inside of a pocket does;
  - `edge_chamfer`: see [Top edge rounds](#top-edge-rounds). An outside chamfer goes with its wall:
    if that wall is left out, so is the chamfer, and its rounds are reported as not cut;
  - `thread`: last, as nothing should cut near a finished thread.
- **Checks against the mesh** (only when the mesh is given):
  - a hole is cut only from a side where no part material is above its opening;
  - a plain through hole may be cut from its far end instead, unless quiddity reports that
    end partly capped;
  - the clearance beyond a through hole's exit is measured.

  Without a mesh none of this is checked, and the plan's `warnings` say so.
- **`skipped`**: every feature that is not planned, with the reason: oblique or unreachable
  holes, countersinks, chamfers, edge rounds not chamfered, turned features (lathe work), and
  features roughing cannot reach (a flat face of the feature points away from the roughing setup),
  or every feature when there is no mesh or roughing does not run. Features of an operation camlib
  refuses say why. **`covered_by_roughing`**: features roughing reaches. That is a heuristic from
  their flat faces. Where `flats` or `wall_finish` do not reach them, they keep the roughing
  allowance.
- **`tool`** and **`tool_note`** on each operation, once `capp::finish` has run: the id of the tool
  that cuts it, or why none can (`tool_note`).
- **`decisions`**: the trace. Every choice the planning makes is a decision citing the rule it was
  made by, as `id@version` from `rust/capp/rulebase/rules.json`, which lists each rule's stage,
  rationale, sources and where the code applies it. Where a rule's logic is itself data, it says
  so (`data`): tool selection (`rulebase/rules/tools.json`: which tools qualify, why the others
  are turned down, which one is preferred), the order of operations (`rules/sequence.json`) and
  what becomes of each feature family (`rules/process.json`). The geometry the rules ask about
  (reachability, room beside a wall and the like) stays code. Each operation's `because` names the
  decisions behind it (its setup, the order, the rule that made it, the setup chosen for its holes,
  its tool and the tools turned down), each entry of `skipped` names its `decision`, and each
  warning is a decision too. A decision a later one replaced says `superseded_by`.
- **`material`**: the material Specify was told (`part.material`), the resources' override
  (`overrides.material`) if any, and its ISO 513 group (P, M, K, N, S, H) once `capp::finish`
  has run. A tool may state its cutting data for a group (`cutting_data: {"N": {"feed_mm_min": …,
  "spindle_rpm": …}}`); each operation's **`technology`** is the cutting data it uses, and says
  whether they are the tool's own or its data for the group. The group is found from the whole
  words of the material's name (a printed material, or a name nothing matches, has none) until
  specify-core gives the material's family (`docs/upstream-needs.md`, S2).
- **`iso14649`** on each operation, once `capp::finish` has run: its names in ISO 14649 (STEP-NC),
  the feature it machines and the operation, from
  `rust/capp/rulebase/reference/iso14649-names.json`. The names are from STEP Tools' published
  AP238 lists, not checked against ISO 14649 itself. Where the names are only the closest
  (thread milling, edge chamfers, V-channels, counterbores, holes bored by helix) they say so
  (`exact: false`).
- **`warnings`**: things to look at. For example, a through hole that ends short of the part's
  far face, where a counterbore or chamfer at the far end, or a shoulder between steps, is not
  planned (quiddity's depths do not include shoulders).

## Top edge rounds

Where the whole top edge of the outside wall (only if that wall is finished and runs straight down
to the roughing floor), or of a hole or slot cut to size, has one convex radius, a 45° chamfer is
cut that meets the round at its middle, so it never cuts into the part. It is a chamfer, not the round,
and needs a `chamfer_mill` tool in the library: the demo has one. Bottom edges are not cut.

## Threads

A hole is threaded when the part says what thread it is: an answer stored in the STEP file
by [Specify](https://github.com/pzfreo/specify-core), `"hole.function:<hole>": "tapped:M5"`.
The answer can be on one hole or on a hole pattern, which threads every hole in the pattern.

- **Sizes:** ISO metric, coarse pitch (`M1.6` to `M24`) or a fine pitch written `M10x1.25`. The
  coarse pitches are the table `rust/capp/rulebase/reference/iso261-coarse-pitch.json`, from two
  public sources that agree, not checked against ISO 261 itself.
  Right-hand only.
- **Depth:** a through hole is threaded all the way. A blind hole is threaded as deep as the
  answer asks (`tapped:M5@5.5`: at least 5.5 mm of full thread), or by default as deep as
  Specify's own default: the hole's depth less three pitches, rounded down to 0.5 mm
  (`rulebase/reference/full-thread.json`).
- **How:** milled with a single-profile thread mill to the thread's major diameter, after the
  hole is drilled. The hole must already be at full diameter as deep as the thread goes;
  otherwise the thread is left out and the log says why. Check the fit with a plug gauge or
  a screw before taking the part off the machine.
- **Neck clearance:** a small thread mill's neck passes close to the drilled wall (0.1 mm for
  the example tool in an M5 hole). camlib proves it clear on its stock grid, so the job makes
  that grid fine enough for the narrowest neck in the setup, and says so. If that would need
  more than 16 million cells for the part, the thread is left out: use a thread mill with a
  narrower neck.
- **Threads written by other CAD tools:** these state no size in the file, so they are a
  warning and are not cut. Answer the hole in Specify to have it threaded.

Thread milling is used, not tapping, because it is safer on a machine that is not proven:
a thread mill takes a light cut, and does not snap off in the hole if something is wrong.

## The tool library

The tool library is JSON: see [`examples/tools.example.json`](examples/tools.example.json).
**Every number in it is a placeholder.** camlib has no defaults for cutting data or machines,
on purpose.

- `machine`: camlib's `Machine` fields for your controller. Only `linuxcnc` and
  `"tool_change_position_mode": "table"` are wired up here; camlib's examples show GRBL, Haas,
  FANUC, Siemens and Centroid posts.
- **Tool changes:** after each tool change the machine must leave the spindle's gauge line
  at `tool_change_gauge_z_mm` over `tool_change_xy_mm`, in the setup's work coordinates (an
  M6 that moves there). camlib plans every new tool's first move down from there and checks
  it, and the post starts from the same position, so the program and the plan agree. By default it is over X0 Y0, at the clearance height
  plus the longest gauge length, so every tool's tip is above the clearance height.
- `tools`: every tool needs `id`, `type` (`end_mill`, `drill`, `thread_mill` or `chamfer_mill`), `number`,
  `diameter`, `overall_length`, `stickout`, `holder_length`, `holder_diameter`,
  `feed_mm_min`, `spindle_rpm` and `coolant`. Also, by type:
  - **End mills:** `flute_length`, `plunge_mm_min`, `stepdown_mm` and `stepover_mm`.
  - **Drills:** `flute_length` and `point_angle`.
  - **Thread mills:** single-profile. `diameter` is the tooth's, so they also need
    `tooth_height`, `neck_diameter` (narrower than the tooth), `neck_length` and
    `plunge_mm_min`.
  - **Chamfer mills:** `flute_length`, `angle` (90 is the only one used), `tip_diameter` and
    `plunge_mm_min`.

  There are no defaults for these: camlib checks the holder against the stock, so a guessed
  holder would make that check meaningless. Optional: `shank_diameter`, `length_register`
  (default: `number`), `retract_feed_mm_min` (default: the feed), `peck_mm` (drills), and for
  thread mills `min_pitch_mm`/`max_pitch_mm` (the pitches the tooth suits), `radial_passes`
  (default 2) and `spring_passes` (default 1).
- Library settings, with their defaults:

  | Setting | Default | Meaning |
  | --- | --- | --- |
  | `clearance_mm` | 10 | clearance height above the stock top |
  | `retract_mm` | 2 | retract and R plane above the stock top |
  | `breakthrough_mm` | 1 | how far a through hole is cut past its exit, at full diameter |
  | `drill_tolerance_mm` | 0.05 | how much smaller than the hole a drill may be (never larger) |
  | `radial_allowance_mm`, `bottom_allowance_mm` | 0.3, 0.2 | roughing allowances (at least twice the mesh's error) |
  | `facing_tool`, `roughing_tool` | the largest end mill | tool ids |
  | `tool_change_gauge_z_mm` | clearance + longest gauge length | the spindle gauge line's work Z after a tool change |
  | `tool_change_xy_mm` | `[0, 0]` (X0 Y0) | where X and Y are after a tool change |
  | `cell_size_mm` | the stock's size / 400, at least 0.1 | camlib's stock simulation grid (made finer where a thread mill's neck needs it) |

How each operation gets a tool:

| Plan operation | camlib operation | Tool |
| --- | --- | --- |
| `facing` | `Facing` | the largest end mill (or `facing_tool`) |
| `roughing` | `Roughing` over the part's mesh, raster clearing | the largest end mill (or `roughing_tool`) |
| `holes` | `Drilling` | the largest drill no bigger than the hole and within `drill_tolerance_mm` of it |
| `holes` with no such drill | `Bore` (helical) | the largest end mill that fits |
| `counterbore` | `Bore` (helical) | the largest end mill that fits |
| `thread` | `ThreadMilling` (internal, helical, climb) | the largest thread mill whose pitch range takes the pitch, whose tooth fits the drilled hole, whose neck clears the thread crests and which reaches the thread's bottom |
| `slot` | `Pocket` | the largest end mill that fits |
| `notch` | `Pocket` | the largest end mill narrower than the width that reaches the depth |
| `v_channel` | `Trace` | the widest `chamfer_mill` with `tip_diameter` 0, an `angle` no wider than the channel's, and `stepdown_mm` |
| `flats` | `Flat` | the largest end mill (or `roughing_tool`) |
| `wall_finish` | `Contour` | the largest end mill (or `roughing_tool`) |
| `edge_chamfer` | `Chamfer` | a 90° chamfer mill wide enough |

How deep holes are cut (the log has a note for each case):

- **Through, exiting the part:** full diameter `breakthrough_mm` past the exit. The drill's
  point goes further, usually below the stock bottom: support the part clear of the holes
  (spoilboard, parallels). A `Bore` stops at the stock bottom (camlib bores no lower), so with
  no `bottom_stock` it leaves a skin or burr at the exit.
- **Through, with part material beyond the exit** (a cross hole into a cavity): no deeper
  than that material less 0.2 mm. If the drill's point does not fit, the drill stops short of
  full diameter at the exit, and the note says so.
- **Blind, drill point:** quiddity's depth ends where the cone starts, so the drill reaches
  full diameter there. Its own point ends where a cone of the drill's angle would; the model's
  cone angle is not read.
- **Blind, flat or unknown:** the drill's **tip** stops at the bottom, never deeper than the
  model. Full diameter then ends a tip length short of it.

A hole no tool fits is reported, not guessed.

## Safety

camlib plans, simulates and checks every move against the stock. An operation with a definite
violation is left out and reported; motions the check could not clear are counted in the log, and
**are still written**. Every block written is decoded again and compared with the plan by camlib's
intent checker, and a mismatch stops the program being given out.

The stock in the setup that holds the part's mesh is compared with that mesh, after its last cut
before thread milling. A program that definitely cuts into the part is refused, and so is one whose
comparison ran out of budget. The log says how much is undecided and how much stock at least remains
(that figure counts the web, and anything a later setup cuts). That check has limits:

- it sees the part from above, as heights on the stock grid, so it cannot decide cells beside a wall,
  and it cannot see a mistake under an overhang;
- what it can miss depends on the part, and the log gives the figures: an error in height under the
  mesh's surface error, flat faces too; sideways, under about a grid cell; and a misplaced cut
  narrower than about four cells;
- threads are not compared, because they go beyond the bore the mesh shows. They are milled last;
- only that setup is compared: others, such as side holes, are not (the log says so);
- the stock it reads is the planned one. The intent checker is what ties the written blocks to that plan.

And:

- neither tool knows your work offsets, clamps or tool lengths;
- camlib's checks here protect the stock, not the finished part (no fixtures or protected
  bodies are declared);
- the roughing mesh is specify-core's triangulation, not an exact surface. Its error
  (`surface_error_mm`) is measured: none on flat faces, the chord error on cylinders, cones and
  tori, and specify-core's bound (0.1% of the part's diagonal) on any other curved face. The
  roughing allowance is kept at least twice that. camlib refuses a mesh with more than
  0.1 mm of error, so a large part with such faces is not roughed (the plan says why).

**Simulate the G-code in your own tools, dry-run above the part, and single-block the first
run.**

## Not done (yet)

Finishing sloped walls, curved surfaces and pocket walls, a smaller tool for flats and walls the
roughing end mill cannot reach, removing the web, countersinks, chamfers, bottom edge rounds, counterbores at the far end
of through holes, tapping, left-hand, inch and external threads, contouring with tabs, fixtures and
avoid areas, tool-change modes other than `table`, 4/5-axis and oblique holes, lathe work, more than
one machine (`docs/capp-plan.md`, Planned), fits (reaming).

## Tests

For the hand-run 100-part MachinePlan-10K benchmark, see [tools/benchmark](tools/benchmark/README.md).

```bash
rust/fetch-camlib.sh
cargo test --release --workspace --manifest-path rust/Cargo.toml
```

- `rust/tests/parity.rs`: plans, jobs and G-code against `tests/fixtures/rust-golden`, a frozen
  baseline written by the Python version this project started as, since moved only by the plate's
  wall finish. It is compared by hand when an intended change moves it.
- `rust/tests/material.rs`: the finished stock against the part, including deliberately wrong
  toolpaths that must be refused.
- `rust/tests/checker.rs`: changes written blocks (preamble, plane, motion, control and end) and
  expects no program to be given out.
- `tests/specify-link.sh`: runs the specify-core release the site ships (with Node) and compares its
  output with the frozen fixtures.
- `rust/tests/walls.rs`: shapes an outline alone would cut wrongly, from `synthetic/walls/`.
- `rust/tests/plan.rs` and `tools.rs`: planning decisions and tool selection.
- `rust/tests/threads.rs`, `slots.rs` and `stepped.rs`: threads, slots, edge chamfers and what is
  reported as not cut.
- `rust/tests/schemas.rs`: the tool libraries and every fixture's plan against the JSON Schemas in
  `schemas/` (`resources-1`, `plan-1`).

The planning is being split into the `capp` crate step by step, as `docs/capp-plan.md` sets out.
What step2cam needs from specify-core for that is in `docs/upstream-needs.md`.
What comes next, in order, is in [ROADMAP.md](ROADMAP.md). How AI agents fit in is in [docs/agents.md](docs/agents.md).

The fixtures in `tests/fixtures` are specify-core's test part `plate.step`, specify-core's output
for it and for `spindle.step` and for `mounting-plate.step` (see its LICENSE file), and the
synthetic parts in `synthetic/`, which `stepped_plate.py` generates with build123d.

## License

[Apache-2.0](LICENSE). specify-core is Apache-2.0; camlib is MIT. This project imports both
and includes neither; the demo site's WebAssembly contains both.
