# The plan format

What the planner writes, how it cuts threads and edge rounds, what the checks cover, and what is not done yet. The [README](../README.md) gives the overview.

## The plan (`specify-cam-plan`, version 1)

The plan says **what** to cut, independent of tools and machine.

- **Stock**: the part's box plus minimum `margin` (2 mm) around the sides, `top_stock` (1 mm) above
  and `bottom_stock` (0) below (`capp::plan::Options`), rounded up to a cited standard flat bar
  and whole-mm sawn length. A catalog miss keeps the computed size and warns.
- **Setups**: one per direction a hole must be entered from (`top`, `bottom`, `front`,
  `back`, `left`, `right`). Each setup turns its tool down −Z. The final process output
  records `zero.xy` and `zero.z`: pickup description, feature ids, earlier finishing setup
  and method. First zero is the stock corner against the fixed jaw (minimum on the other
  axis), Z on stock top. Later setups prefer drawing datums, then the largest reachable
  previously finished bore at least 6 mm diameter, else other finished faces. Probe access
  includes carried stock and fixtures. Hidden faces can locate by contact against the fixed
  jaw and a required stop picked up as its axis zero before loading; the sheet limits
  the stop nose height below the overhang. Non-flips prefer a direct finished upward
  face for Z; non-flip Z and side probing allow 0.5 mm of stock-envelope slack.
  Flip Z uses a finished seat on modeled parallels; absent support evidence requires stock touch-off, a bar-variation warning
  and measuring after facing.
  Missing X/Y evidence warns and retains the frame. `rotation` and `origin_model` locate
  all coordinates. The intermediate planner retains legacy stock/part frame labels;
  final pickup selection follows execution refusals. See [pickup policy](capp-plan.md#setup-zero-and-pickup-m10).
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
  - `thread`: internal threads, from Specify's answers (see [Threads](#threads)). A blind thread
    whose depth is the default, not asked for, carries `default_depth: true`;
  - `slot`: a closed slot with round ends, cut to size with a camlib `Pocket` of its outline and
    the largest end mill that fits, after roughing, from whichever setup reaches it. Other slots
    are left to roughing;
  - `notch`: square-ended slots and notches, as a pocket with the largest end mill that fits and
    reaches, cut past the part on a notch's open side. specify-core's record says where the box is,
    and the mesh must show it empty above the floor, or it is not cut. Its inside corners stay as
    round as the cutter;
  - `v_channel`: a V-channel that runs out of the part at both ends, with a sharp V-bit no wider
    than the channel, its tip along the root. A narrower bit leaves a little on the walls: the
    plan warns that the channel is partly cut and says how much is left. A channel in a flat
    goes with that flat;
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
  refuses, or that no tool in the library cuts, say why. **`covered_by_roughing`**: features roughing reaches. That is a heuristic from
  their flat faces. Where `flats` or `wall_finish` do not reach them, they keep the roughing
  allowance.
- **`tool`** and **`tool_note`** on each operation, once `capp::finish` has run: the id of the tool
  that cuts it. An operation no tool cuts is taken out of `operations` (those after it are
  numbered again) and each of its features is in `skipped` with why and the tool it needs; one
  with no features of its own gets a warning instead. Spot drilling left out this way does not
  skip its holes, since they are still drilled: a warning per setup (rule `tool.none.unspotted`)
  says how many holes are drilled without spotting and names them. `capp::resolve` alone still
  leaves such an operation in place with `tool: null` and why (`tool_note`).
- **`tools_needed`**, after `capp::finish`: missing-tool queries, each with a `description`,
  `features` and `operations` (operation ids or decision subjects; an operation no tool cuts is
  named as it was numbered before it was taken out). Identical descriptions merge;
  alternatives stay together, without attempting a minimal shopping list. Tool-related reasons
  end with `: needs <description>`. Bounds come from the selectors in
  `rust/capp/rulebase/rules/tools.json` and the library's drill tolerance. Diameter limits round
  conservatively to 0.01 mm; flute reach rounds up to whole mm with 0.001 mm clearance for the
  execution engine's numerical guard. A drill may additionally need its point length to reach
  full diameter. Excessive stepdown is a cutting-data issue, not a missing-tool query. When
  roughing is refused for tool reach, its covered features inherit the same query.
- **`decisions`**: the trace. Every choice the planning makes is a decision citing the rule it was
  made by, as `id@version` from `rust/capp/rulebase/rules.json`, which lists each rule's stage,
  rationale, sources and where the code applies it. Where a rule's logic is itself data, it says
  so (`data`): tool selection (`rulebase/rules/tools.json`: which tools qualify, why the others
  are turned down, which one is preferred), the order of operations (`rules/sequence.json`) and
  what becomes of each feature family (`rules/process.json`). The geometry the rules ask about
  (reachability, room beside a wall and the like) stays code. The camlib adapter's process
  choices are not decisions in the plan: its notes in the log cite their rules the same way
  (`process.roughing.clearing`, `process.roughing.allowance`, `process.roughing.region`, `process.entry.helix`,
  `tool.flutes.default`, `tool.material.default`). Each operation's `because` names the
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
  answer asks (`tapped:M5@5.5`: at least 5.5 mm of full thread; a `hole.full_thread:<hole>`
  answer such as `"5.5"` takes precedence over the `@` depth, as in Specify), or by default as deep as
  Specify's own default: the hole's depth less three pitches, rounded down to 0.5 mm
  (`rulebase/reference/full-thread.json`). A tap needs its lead plus one pitch of clearance below
  the thread, which for a three-pitch lead is more than those three pitches, so a tap cuts a
  default thread shallower when it must: the drilled full diameter less its lead and one pitch,
  rounded down to 0.5 mm, and the log says so. An asked depth is
  never shortened: if the tap cannot reach it, the tap is refused and a thread mill is
  used instead, if one fits.
- **How:** tapped when a matching M3–M6 tap is available in ISO 513 group N material,
  otherwise milled with a single-profile thread mill to the thread's major diameter, after the
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

Tap selection checks the model pre-drill, pitch, hand, chamfer, bottom clearance and reach.
The M3–M6 / ISO N preference is step2cam judgement, traced as `tool.thread.tap_or_mill`;
without taps the thread-mill plan is unchanged. Cut-tap drills use ISO 2306 Table 1
values checked in the published preview (D − P check); form-tap drills use Tapmatic’s formula at 65% thread, explicitly
not checked against a standard. See [tap references](../rust/capp/rulebase/reference/tap-drills.json).
A blind tap tip extends below full thread by `chamfer_pitches × pitch`, with one more pitch
of drilled clearance. Through tapping adds the required drilling travel and conservative
simulated backing below the exit to satisfy camlib’s stock bounds; obstructed exits are checked
again against the actual drilled floor. Through thread operations retain their measured
exit clearance; insufficient room for the tap, bottom clearance and drill point selects
thread milling at plan time. Supplied plan `hand: "left"` is honored by both tapping and
thread milling; Specify LH designations are not parsed yet. Other operations and the comparison base retain
exactly the real stock bottom; the stock/part comparison clips out simulated backing.
The tapping note gives tap-tip and drill-tip travel below the part so parallels stay clear.
Tapping retains the normal stock/part target:
camlib represents it at the pre-drill diameter and leaves the thread flanks in the stock.

## Safety

camlib plans, simulates and checks every move against the stock. An operation with a definite
violation is left out and reported; motions the check could not clear are counted in the log, and
**are still in the saved camlib job**. When camlib posts G-code (the demo's Save G-code), an
uncertain or violating move refuses the program unless it is accepted, and step2cam accepts
neither. Every block written is decoded again and compared with the plan by camlib's
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

**Simulate any G-code made from it in your own tools, dry-run above the part, and single-block the
first run.**

## Not done (yet)

Finishing sloped walls, curved surfaces and pocket walls, a smaller tool for flats and walls the
roughing end mill cannot reach, removing the web, countersinks, chamfers, bottom edge rounds, counterbores at the far end
of through holes, left-hand Specify answers, inch and external threads, contouring with tabs, fixtures and
avoid areas, tool-change modes other than `table`, 4/5-axis and oblique holes, lathe work, more than
one machine (`docs/capp-plan.md`, Planned), fits (reaming).

## Tests

For the hand-run 100-part MachinePlan-10K benchmark, see [tools/benchmark](../tools/benchmark/README.md).

```bash
rust/fetch-camlib.sh
cargo test --release --workspace --manifest-path rust/Cargo.toml
```

Setup-sheet checks: `npm ci --ignore-scripts && node tests/report.mjs`. They build the native
`process` example with the pinned Rust toolchain and generate plans from the committed
fixture meshes on both machines. No benchmark cache or saved reports are read. The pinned
PDF packages and pinned DejaVu font package are development-only verification dependencies.

- `rust/tests/parity.rs`: plans and jobs against `tests/fixtures/rust-golden`, a frozen
  baseline written by the Python version this project started as, since moved only by the plate's
  wall finish. It is compared by hand when an intended change moves it.
- `rust/tests/material.rs`: the finished stock against the part, including deliberately wrong
  toolpaths that must be refused.
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
What comes next, in order, is in [ROADMAP.md](../ROADMAP.md). How AI agents fit in is in [docs/agents.md](agents.md).

The fixtures in `tests/fixtures` are specify-core's test part `plate.step`, specify-core's output
for it and for `spindle.step` and for `mounting-plate.step` (see its LICENSE file), and the
synthetic parts in `synthetic/`, which `stepped_plate.py` generates with build123d.
