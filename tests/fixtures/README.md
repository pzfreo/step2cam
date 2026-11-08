# Test fixtures

Every part has its STEP file, specify-core's analysis of it (`*.analysis.json`) and, where a test
needs one, its mesh (`*.mesh.json`).

Each STEP file except `synthetic/stepped-plate-large-chamfer.step` and
`synthetic/stepped-plate-chamfer-mouths.step` (plain geometry from `stepped_plate.py`, added
for #47) carries GD&T as AP242 PMI, written into it by `specify-core write` from the answers
in `answers/`:
- the part's material (Aluminium 6082-T6; `plate-m5` keeps its earlier `6082-T6`), process,
  general tolerance (ISO 2768-m), surface finish and edge break;
- datums A, B and C on the faces specify-core proposes, with flatness 0.02 on A and
  perpendicularity 0.05 on B and C;
- position tolerances on the holes;
- one fitted feature each, where the part has one to fit: H7 on the plate's ⌀8 hole, the
  stepped plates' ⌀10 hole and the mounting plate's ⌀28 hole; h6 on the spindle's ⌀15 seat,
  with runout 0.02 on its ⌀24 and ⌀10.

The answers keep each plan as it was before the GD&T: holes specify-core would call tapped are
answered `general`, except in `plate-m5`, which exists to be tapped. The planner does not use the
tolerances yet (issues #40–#42).

| Part | Geometry from |
|---|---|
| `plate.step`, `plate-m5.step` | specify-core's test part `plate.step` |
| `spindle.step` | specify-core's test part `spindle.step` |
| `mounting-plate.step` | Thompson Labs' `mounting-plate.step`, MIT (`mounting-plate.LICENSE`) |
| `synthetic/stepped-plate*.step` | `synthetic/stepped_plate.py` (including the plain 5 mm top chamfer regression for #47) |
| `synthetic/tall-wall-pocket.step` | `synthetic/tall-wall-pocket.py`; no GD&T answers needed (camlib reach verdict tests) |
| `synthetic/square-slots.step` | `synthetic/square_slots.py`; no GD&T answers needed (geometry and corner-deviation tests) |
| `synthetic/walls/*.step` | made for the wall tests (`rust/tests/walls.rs`) |

To change a part's GD&T, or after regenerating its geometry, write the answers into the plain
STEP file and analyse the result again:

```bash
specify-core analyse -o plain.analysis.json plain.step
specify-core write -o part.step --accept-defaults plain.step plain.analysis.json answers/part.answers.json
specify-core analyse -o part.analysis.json part.step
```

Write the mesh with the specify-core WebAssembly release the site ships (`SPECIFY_VERSION` in
`demo/build.sh`), not the command-line tool: they triangulate some curved faces differently, and
`tests/specify-link.sh` checks every frozen analysis and mesh against that release.

The plain geometry of the specify-core and Thompson Labs parts is in those projects;
`stepped_plate.py` writes the stepped plates' again.
