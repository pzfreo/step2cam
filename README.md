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
  `left`, `right`). Each setup is the part turned so its tool comes down -Z, then moved so
  **X0 Y0 is the stock's minimum corner and Z0 the stock top**. The setup records its
  `rotation` and `origin_model` (the model point at X0 Y0 Z0) so you can find it on the
  machine. A plain through hole is cut from whichever end the preferred setup sees.
- **Operations** in each setup, in setup coordinates:
  - `facing` down to the part's top (the `--rough-from` setup only, default `top`);
  - `roughing` of the whole part from the mesh, stopping `--skin` (0.5 mm) above the part's
    bottom so the part stays held (`--rough-from` setup only, needs the mesh);
  - `holes`: holes of one diameter and depth, with their centres, bore top, bottom and how the
    bottom ends (`through`, `flat`, `drill_point`, `unknown`);
  - `counterbore`: the enlargements at hole mouths (counterbores, spotfaces, stepped mouths).
- **`skipped`**: everything recognised but not planned, with the reason: oblique holes,
  countersinks, turned features (lathe work), and, without a mesh, pockets, slots and steps.
  With a mesh these are cleared by roughing (`covered_by_roughing`).

## The tool library

`post` reads a JSON tool library: see [`examples/tools.example.json`](examples/tools.example.json). **Every
number in it is a placeholder.** camlib has no defaults for cutting data or machines, on
purpose; nor does this bridge.

- `machine`: camlib's `Machine` fields for your controller (only `linuxcnc` is wired up here;
  camlib's examples show GRBL, Haas, FANUC, Siemens and Centroid posts).
- `tools`: `end_mill` and `drill`, each with its T number, dimensions, holder, cutting data,
  and for end mills `stepdown_mm`/`stepover_mm`; a drill can have `peck_mm`.
- `clearance_mm`, `retract_mm` (above the stock top), `breakthrough_mm` (through holes),
  `drill_tolerance_mm`, roughing allowances, optional `facing_tool`/`roughing_tool` ids.

How each operation gets a tool:

| Plan operation | camlib operation | Tool |
| --- | --- | --- |
| `facing` | `Facing` | the largest end mill (or `facing_tool`) |
| `roughing` | `Roughing` over the part's mesh, raster clearing | the largest end mill (or `roughing_tool`) |
| `holes` | `Drilling` | a drill within `drill_tolerance_mm` of the diameter |
| `holes` with no such drill | `Bore` (helical) | the largest end mill that fits |
| `counterbore` | `Bore` (helical) | the largest end mill that fits |

Through holes are drilled to full diameter `breakthrough_mm` past the bottom. Blind holes stop
with the drill's **tip** at the hole's bottom, never deeper than the model (a flat-bottomed
hole keeps a drill point; the post says so). A hole no tool fits is reported, not guessed.

## Safety

camlib plans, simulates, checks every move against the stock and posts with its own checks;
`post` refuses a program with violations or uncertain results unless you `--accept` them. But
neither tool knows your work offsets, clamps or tool lengths, and the roughing mesh is
specify-core's triangulation (deflection 0.1% of the part's diagonal), not an exact surface:
the roughing allowance is kept at least twice that. **Simulate the G-code in your own tools,
dry-run above the part, and single-block the first run.**

## Not done (yet)

Finishing passes (walls and floors are left with the roughing allowance), countersinks and
chamfers, tapping and thread milling (specify-core reads threads from answers, not geometry),
contouring with tabs, 4/5-axis and oblique holes, lathe work. Each is a camlib operation away:
the plan format leaves room for them.

## Tests

```bash
python3 -m unittest discover -s tests
```

Standard library only: the post stage is tested against a stand-in for camlib that records the
operations and tools the bridge asks for. The fixtures in `tests/fixtures` are specify-core's test part `plate.step` and its output for
it and `spindle.step`; with specify-core installed, `plan_step` is checked against them.

## License

[Apache-2.0](LICENSE). specify-core is Apache-2.0; camlib is MIT. This project imports both
and includes neither.
