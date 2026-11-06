# step2cam: STEP features to camlib toolpaths

`step2cam` turns the features [Specify](https://github.com/pzfreo/specify-core) recognises in a
STEP part (holes, counterbores, the part's shape) into toolpaths and G-code with
[camlib](https://github.com/stevecad/camlib), the CAM engine.

It calls two Python libraries in turn: specify-core (feature recognition by
[pzfreo/quiddity](https://github.com/pzfreo/quiddity)) and camlib. Between them is a plain
JSON plan you can read, keep or edit.

```
part.step ─▶ specify_core.api.analyse / mesh ─▶ step2cam.plan ─▶ plan.json ─▶ step2cam.post ─▶ part-top.nc
             (quiddity: features)               (what to cut)                 (camlib: how)    part-left.nc …
```

## Install

Python 3.12 or later.

```bash
python -m pip install git+https://github.com/pzfreo/step2cam   # step2cam + specify-core (PyPI)
python -m pip install git+https://github.com/stevecad/camlib   # camlib, for `post`
```

- **specify-core** comes from PyPI as a prebuilt wheel (no Rust needed). 0.1.12 or later.
- **camlib is not on PyPI.** The `camlib` package on PyPI is an unrelated web library;
  `pip install camlib` gets that one, and `step2cam post` will say so. Install camlib from
  GitHub as above. That compiles its Rust engine, so it needs a Rust toolchain
  ([rustup](https://rustup.rs)); camlib can also build a Linux wheel in Docker
  (`packaging/build-manylinux-wheel.sh` in its repo).

Planning needs only specify-core, so you can plan on one machine and post on another.

## Use

From Python:

```python
import json
import step2cam

plan = step2cam.plan_step("part.step")            # specify-core analyses and meshes the part
tools = json.load(open("my-tools.json"))
step2cam.post(plan, tools, "out/", stem="part")   # camlib: out/part-top.nc, ...
```

From the command line:

```bash
step2cam step part.step -o part.plan.json                      # STEP -> plan
step2cam post part.plan.json --tools my-tools.json -o out/     # plan -> G-code, per setup
```

`step2cam step --part N` picks one part of an assembly; `--no-mesh` skips roughing.
`step2cam plan analysis.json --mesh mesh.json` plans from JSON that `specify-core analyse` and
`specify-core mesh` wrote.

## The plan (`specify-cam-plan`, version 1)

The plan says **what** to cut, independent of tools and machine. Read it before posting.

- **Stock**: the part's box plus `--margin` (2 mm) around the sides, `--top-stock` (1 mm) above
  and `--bottom-stock` (0) below.
- **Setups**: one per direction a hole must be entered from (`top`, `bottom`, `front`, `back`,
  `left`, `right`). Each setup is the part turned so its tool comes down -Z, then zeroed with
  **X0 Y0 on the minimum corner and Z0 on the top** of:
  - the **stock**, in the `--rough-from` setup (the first; default `top`);
  - the **part's own box**, in every later setup. By then roughing has cut away the stock
    around the part, so zero on the part, after removing the web roughing leaves (below).

  Each setup records its `zero` (`stock` or `part`), its `rotation` and `origin_model` (the
  model point at X0 Y0 Z0), and `post` prints them.
- **Operations** in each setup, in setup coordinates:
  - `facing` down to the part's top (the `--rough-from` setup only);
  - `roughing` of the whole part from the mesh (the `--rough-from` setup only, needs the mesh).
    It stops `--skin` (0.5 mm) above the part's bottom, so it leaves a web that thick all round
    the part (`web_mm`), which no operation removes. It also clears the side margin, which is
    where vise jaws on the stock would be: hold the stock from below (a fixture plate, screws or
    double-sided tape) or leave more margin;
  - `holes`: holes of one diameter and depth, with their centres, bore top, bottom and how the
    bottom ends (`through`, `flat`, `drill_point`, `unknown`). A through hole also carries
    `exit_clear_mm`: how far beyond its exit there is no part material, `null` when it exits
    the part altogether;
  - `counterbore`: the enlargements at hole mouths (spotfaces, counterbores, stepped mouths),
    widest and outermost first.
- **Checks against the mesh** (only when the mesh is given):
  - a hole is cut only from a side where no part material is above its opening;
  - a plain through hole may be cut from its far end instead, unless quiddity reports that
    end partly capped;
  - the clearance beyond a through hole's exit is measured.

  Without a mesh none of this is checked, and the plan's `warnings` say so.
- **`skipped`**: every feature that is not planned, with the reason: oblique or unreachable
  holes, countersinks, turned features (lathe work), and features roughing cannot reach (a
  flat face of the feature points away from the roughing setup), or every feature when there
  is no mesh. **`covered_by_roughing`**: features roughing reaches. That is a heuristic from
  their flat faces, and they are left with the roughing allowance (there is no finishing pass).
- **`warnings`**: things to look at. For example, a through hole that ends short of the part's
  far face, where a counterbore or chamfer at the far end, or a shoulder between steps, is not
  planned (quiddity's depths do not include shoulders).

## The tool library

`post` reads a JSON tool library: see [`examples/tools.example.json`](examples/tools.example.json).
**Every number in it is a placeholder.** camlib has no defaults for cutting data or machines,
on purpose.

- `machine`: camlib's `Machine` fields for your controller. Only `linuxcnc` and
  `"tool_change_position_mode": "table"` are wired up here; camlib's examples show GRBL, Haas,
  FANUC, Siemens and Centroid posts. A tool change is assumed to leave the tool at X0 Y0 at
  the clearance height.
- `tools`: every tool needs `id`, `type` (`end_mill` or `drill`), `number`, `diameter`,
  `flute_length`, `overall_length`, `stickout`, `holder_length`, `holder_diameter`,
  `feed_mm_min`, `spindle_rpm` and `coolant`. End mills also need `plunge_mm_min`,
  `stepdown_mm` and `stepover_mm`; drills need `point_angle`. There are no defaults for these:
  camlib checks the holder against the stock, so a guessed holder would make that check
  meaningless. Optional: `shank_diameter`, `length_register` (default: `number`),
  `retract_feed_mm_min` (default: the feed), `peck_mm` (drills).
- Library settings, with their defaults:

  | Setting | Default | Meaning |
  | --- | --- | --- |
  | `clearance_mm` | 10 | clearance height above the stock top |
  | `retract_mm` | 2 | retract and R plane above the stock top |
  | `breakthrough_mm` | 1 | how far a through hole is cut past its exit, at full diameter |
  | `drill_tolerance_mm` | 0.05 | how much smaller than the hole a drill may be (never larger) |
  | `radial_allowance_mm`, `bottom_allowance_mm` | 0.3, 0.2 | roughing allowances (at least twice the mesh's error) |
  | `facing_tool`, `roughing_tool` | the largest end mill | tool ids |
  | `cell_size_mm` | the stock's size / 400, at least 0.1 | camlib's stock simulation grid |

How each operation gets a tool:

| Plan operation | camlib operation | Tool |
| --- | --- | --- |
| `facing` | `Facing` | the largest end mill (or `facing_tool`) |
| `roughing` | `Roughing` over the part's mesh, raster clearing | the largest end mill (or `roughing_tool`) |
| `holes` | `Drilling` | the largest drill no bigger than the hole and within `drill_tolerance_mm` of it |
| `holes` with no such drill | `Bore` (helical) | the largest end mill that fits |
| `counterbore` | `Bore` (helical) | the largest end mill that fits |

How deep holes are cut (the post prints a note for each case):

- **Through, exiting the part:** full diameter `breakthrough_mm` past the exit. The drill's
  point goes further, usually below the stock bottom: support the part clear of the holes
  (spoilboard, parallels). A `Bore` stops at the stock bottom (camlib bores no lower), so with
  no `--bottom-stock` it leaves a skin or burr at the exit.
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

camlib plans, simulates, checks every move against the stock and posts with its own checks.
`post` refuses a program with violations or uncertain results unless you `--accept` them. But:

- neither tool knows your work offsets, clamps or tool lengths;
- camlib's checks here protect the stock, not the finished part (no fixtures or protected
  bodies are declared);
- the roughing mesh is specify-core's triangulation, not an exact surface. Its error
  (`surface_error_mm`) is measured: none on flat faces, the exact chord error on cylinders,
  and specify-core's bound (0.1% of the part's diagonal) on any other curved face. The
  roughing allowance is kept at least twice that. camlib refuses a mesh with more than
  0.1 mm of error, so a large part with such faces is not roughed (the plan says why).

**Simulate the G-code in your own tools, dry-run above the part, and single-block the first
run.**

## Not done (yet)

Finishing passes (walls and floors are left with the roughing allowance), removing the web,
countersinks and chamfers, counterbores at the far end of through holes, tapping and thread
milling (specify-core reads threads from answers, not geometry), contouring with tabs, fixtures
and avoid areas, tool-change positions other than `table`, 4/5-axis and oblique holes, lathe
work.

## Tests

```bash
python3 -m unittest discover -s tests
```

Three levels, each used when it can be:

- **Always (standard library):** the plan, and the tool choices against a stand-in for camlib
  that records what the bridge asks for.
- **With camlib's Python sources** (`CAMLIB_PYTHON=<camlib checkout>/python`, no engine
  needed): the same tool-choice cases built from camlib's real classes, so a wrong argument
  name fails.
- **With camlib installed:** a whole plan posted to G-code.

CI runs all three. The fixtures in `tests/fixtures` are specify-core's test part `plate.step`,
and specify-core's output for it and for `spindle.step`. With specify-core installed,
`plan_step` is checked against them.

## License

[Apache-2.0](LICENSE). specify-core is Apache-2.0; camlib is MIT. This project imports both
and includes neither.
