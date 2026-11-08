# The Makera Carvera: what is public, for making step2cam useful on it

Research for making step2cam's plans usable on a Makera Carvera (read 7 November 2026). The aim
is real tools, real cutting data and G-code the machine runs, so that Carvera owners can try it
and say where it is wrong.

## Cutting data and tools: Makera's own tool libraries

Makera publishes its tool libraries, with a cutting-data preset per material, in
[MakeraInc/CarveraProfiles](https://github.com/MakeraInc/CarveraProfiles), under
`CAM_Post_Processors/Fusion360-profiles/Tool Files/` (Fusion `.tools`, `.hsmlib` and `.csv`;
FreeCAD versions alongside).

| Library | Tools | Material presets |
|---|---|---|
| O Flute Bits (single-flute spiral "O", for metal and plastics) | 25, ⌀1–6 mm | Aluminium, Brass, Copper (11 metal bits); Hardwood, Plastic, Softwood (all) |
| Drill Bits | 30, ⌀0.2–3.175 mm, all 1/8" shank | Aluminium, Brass, Carbon Fibre, Copper, Hardwood, PCB, Plastic, Softwood |
| Thread Mills | 8, M1–M8 | Aluminium, Brass, Copper, Hardwood, Plastic, Softwood |
| Corn Bits (PCB, carbon fibre) | 20 | Carbon Fibre, PCB |

Also published: ball end mills, engraving bits (V-bits), "other tools", tool holders.

Aluminium, as Makera sets it:

| Tool | ⌀ | rpm | Feed (mm/min) | Plunge (mm/min) | Depth of cut (mm) |
|---|---|---|---|---|---|
| Spiral O metal, 1 flute | 6, 4, 3.175, 2.5 | 12,000 | 500 | 200 | 0.2 |
| Spiral O metal, 1 flute | 2 | 12,000 | 500 | 200 | 0.15 |
| Spiral O metal, 1 flute | 1.5, 1 | 12,000 | 500 | 200 | 0.1 |
| Drill, 2 flute | 0.2–3.175 | 10,000 | n/a | 100 | n/a |
| Thread mill M3 / M5 / M6 / M8 | tooth 2.42 / 4 / 4.8 / 6 | 15,000 | 200 | 200 | n/a |

What this says about planning on a Carvera:
- **Light cuts:** 0.2 mm depth of cut in aluminium. A review of the Carvera Air found the
  same: the best results came from Makera's own bits and default settings, with a very shallow
  0.2 mm stepdown ([Hackster](https://www.hackster.io/news/the-maker-s-toolbox-makera-carvera-air-cnc-mill-review-068186e07296)).
  Roughing a plate takes many passes.
- **No drill above 3.175 mm:** every larger hole is milled by helix, which step2cam's
  `tool.hole.drill_or_bore` already does.
- **Small tools only:** at most ⌀6 (or 1/4") shanks, matching the collets.
- **Thread mills to M8:** with the tooth and neck sizes needed to choose one.

Caveats:
- **No licence on the repo:** it states none. The numbers are facts and can be cited, but
  Makera's files should not be copied into step2cam; the reference data should record the values
  and cite the repo.
- **Stepover not stated** for the end mills; it is chosen per operation in Fusion.
- **Machine variant:** the presets are for the Carvera; the Carvera Air's spindle is weaker (13,000 rpm).

Makera's wiki has a [Speeds & Feeds page](https://wiki.makera.com/speeds-and-feeds), which did
not render for reading here; the tool libraries carry the same kind of data.

## G-code: the Carvera's dialect

The Carvera runs Makera's Smoothieware-based firmware; the
[Carvera Community firmware](https://github.com/Carvera-Community/Carvera_Community_Firmware) is a
maintained fork, documented at
[carvera-community.gitbook.io](https://carvera-community.gitbook.io/docs/firmware/supported-commands).
Makera's Fusion post (`Carvera.cps`, "Makera Carvera Community Post v1.4.6", in the same repo)
shows what programs look like.

| | Carvera | step2cam's LinuxCNC post today |
|---|---|---|
| File | `.cnc` | `.nc` |
| Tool change | `T1 M6`; the firmware then measures the tool length (`C1`, the default) | `T1 M6` then `G43 H1` |
| Tool length offset | Applied by the firmware after `M6`; `G43`/`G49` are not in its G-code list | `G43 H`, `G49` |
| Canned drilling cycles | Not in its G-code list; Makera's post writes none, so drilling is plain G0/G1 | Read back as moves; check what is written |
| Arcs | G2/G3 with I/J, in G17/G18/G19; helices allowed (`allowHelicalMoves`) | Same |
| Work offsets | G54–G59, G59.1–G59.3 | G54–G59 |
| Units | G21 only; G20 is listed as halting the machine | G21 |
| Not listed | G40, G49, G61/G61.1/G64, G91.1, G97, G80 | All in the preamble |
| Coolant | `M7` compressed air on, `M9` off; `M801 S100`/`M802` vacuum; `M331`/`M332` auto vacuum | `M8` flood |
| Retract | `G53 Z-3` (machine coordinates) before tool changes and at the end | Clearance height |
| End | `M30` | `M2` |
| Other | `M490.1`/`M490.2` close/open the collet (manual change), `M491` calibrate tool length, `M220 S100` feed override, `M821`/`M822` light, `M370` clear auto-levelling | n/a |

A Carvera post for step2cam would mean:
- **A preamble it accepts:** `G21 G90 G17 G94 G54`.
- **Tool changes:** `T<n> M6`, with no `G43`.
- **Coolant:** air (`M7`/`M9`).
- **Retracts and end:** `G53 Z-3` retracts, ending with `M30`.
- **Drilling:** as plain moves, unless camlib already writes it that way.
- **Read-back check:** step2cam's checker reads the program back, so it would need to know the
  same dialect.
- **Uncertain codes:** where the firmware's list is silent (`G43`, `G61`, cycles), the post
  should leave them out rather than guess.

The community also maintains its own post and machine profiles:
[Carvera_Community_Profiles](https://github.com/Carvera-Community/Carvera_Community_Profiles).

## Shared projects

Makera's [wiki "Knowledge Sharing"](https://wiki.makera.com/knowledge-sharing) and blog share
project files, mostly decorative (woodwork, reliefs, signs) as SVG, images and MakeraCAM `.mkc`
projects, with some toolpaths. There is no library of engineering parts with their G-code, so
comparing step2cam's plans with "what owners do" from public files is weak evidence. It would be
better to ask owners directly once there is something to try.

## What this means for the next steps

1. **Real tools and cutting data** for the demo: Makera's O-flute end mills, drills and thread
   mills with their aluminium presets (cited, not copied). The planner will then rough at 0.2 mm
   and helix-mill every hole above 3.175 mm, which is what a Carvera owner would do.
2. **A Carvera post** (`dialect: "carvera"`) as above, with its read-back check.
3. **The disagree link**, so the first users' objections arrive as rule ids.
