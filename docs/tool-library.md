# The tool library

The tool library, machines and workholding the planner reads. The [README](../README.md) gives the overview; the shipped libraries are described in [libraries/README.md](../libraries/README.md).

The tool library is JSON: see [`examples/tools.example.json`](../examples/tools.example.json).
Each mapped tool field may also be supplied by its ISO 13399 symbol, in the same units
(mm, or degrees for `SIG`). The [mapping table](../rust/capp/rulebase/reference/iso13399-fields.json)
lists every field, its type-specific alias and public sources, or why it has no exact alias.
For example, `diameter` may be `DC` for an end mill, and `overall_length` may be `OAL`.
Mill `flute_length` uses `APMX`; drill `flute_length` uses `LCF`. Unmapped fields keep
their current names. Both names may be given if their values are equal; differing values
refuse the library with an error naming the tool and both fields. Missing fields are
reported using the current name. This translates names only, without converting dimensions.

`process` plans and simulates each available workholding candidate, then chooses by
`setup.workholding`: finishes the outline and removes grip stock, fewest features left
out, fewest setups, least extra stock, shortest machining time. The Plan tab shows the
choice and alternatives; selecting another available option replans it. Both vise
routes require a dimensioned vise in the first mill. Fixture plates and margins/tabs
are listed with their implementation blockers, without simulated results.

Vise setups compare cited clamping force with estimated tangential load for
plan-known engagement: facing, flats, slots/notches, V-channels, walls and chamfers
(`machine.grip_vs_cut`). The Haas DX6 plans 3440 lbf at 40 ft·lbf: moderate hand
tightening, half its rated maximum (step2cam judgement). Facing is stepped down
when feasible; other checked cuts warn with a remedy. Drilling, spotting and tapping
record thrust into supports. Roughing of every kind, helix-milled holes/counterbores
and thread milling are checked against the stock under their emitted feed moves,
including helix turns. The largest measured mean force, engagement, move and position
enter the same decision trace; over-capacity moves warn with a remedy and leave the program unchanged. The demo vise has no published force and
warns “grip not checked against the cut” once per setup, keeping its cutting data.
Contact pressure, lift and turning moments remain unchecked. See
[formula, sources and limits](capp-plan.md#vise-grip-against-the-cut-m3).

Supply a choice with `decisions`:
`{"rule":"setup.workholding","subject":"part","choice":"vise_ordinary","source":"user","reason":"shop preference"}`.
The checked choice and its provenance are stored in `plan.supplied_decisions` for replay.
The old `overrides.workholding_option` still works: `"none"` means `vise_ordinary`,
`"vise_flip_face"` selects that route, both recorded as user choices. An explicit
supplied decision takes precedence. With neither, the default rule chooses; all three
demo plates now select flip-and-face to remove the ordinary grip's attached outline.
See [the CAPP planning documentation](capp-plan.md) for consequences and limits.

The example library is a placeholder; the demo Carvera and opt-in Haas libraries cite vendor data and label assembly/policy judgements. camlib has no defaults for cutting data or machines,
on purpose.

- The optional `controller` identifies the intended camlib post; camlib's posts write the G-code. `rapid_mm_min` supplies the page and candidate motion-time estimates.
- `machines`: what the machines can do, for the planner. The plan is cut on the first `mill`:
  - `axes.travel_mm` (x, y, z): a setup whose stock does not fit is not cut, and its features say
    why;
  - `workholding`: a vise's `max_jaw_opening_mm`, so stock wider than it is flagged;
  - `spindle.max_rpm`: faster cutting data run at that speed, the feeds scaled to match;
  - `spindle.max_tool_diameter_mm`: a tool with a wider shank is never chosen;
  - `tool_changer`: its `pockets` (a setup needing more tools is flagged), or `"kind": "manual"`.

  `spindle.collet_change: "manual"` adds a setup warning when selected tools have
  different shank sizes, independently of the tool changer; it does not alter tool choice.

  Each is a decision in the plan's trace. Without `machines` none of this is checked, and the
  trace says so. The demo's default machine is the **Makera Carvera**, from its published specifications:
  360 × 240 × 140 mm of travel, 15,000 rpm, a 1/4" collet (shanks to 6.35 mm) and a 6-tool
  changer. The demo supplements the kit with sourced 4/6 mm Makera cutters and a long small cutter as optional inventory for deep small features. Matching 6, 4, 3 mm and 1/8" collets require manual changes; the 1/4" capacity is not mixed-shank ATC compatibility. Larger holes are bored. See [library assumptions](../libraries/README.md#carvera-demo).
- **Tool changes:** after each tool change the machine must leave the spindle's gauge line
  at `tool_change_gauge_z_mm` over `tool_change_xy_mm`, in the setup's work coordinates (an
  M6 that moves there). camlib plans every new tool's first move down from there and checks
  it; a post must start from the same position, so the program and the plan agree. By default it is over X0 Y0, at the clearance height
  plus the longest gauge length, so every tool's tip is above the clearance height.
- `tools`: every tool needs `id`, `type` (`end_mill`, `drill`, `spot_drill`, `tap`, `thread_mill` or `chamfer_mill`), `number`,
  `diameter`, `overall_length`, `stickout`, `holder_length`, `holder_diameter`,
  `spindle_rpm` and `coolant`; tools other than taps also need `feed_mm_min`. By type:
  - **End mills:** `flute_length`, `plunge_mm_min`, `stepdown_mm` and `stepover_mm`.
  - **Drills:** `flute_length` and `point_angle`.
  - **Taps:** `diameter` is nominal thread diameter; also require `pitch`, `tap_type`
    (`cut` or `form`), `chamfer_form` (e.g. bottoming/plug), `chamfer_pitches` (maximum
    incomplete lead), `thread_length` and `shank_diameter`. `hand` defaults to `right`.
    Optional `flute_style`: `straight`, `spiral_flute`, or `spiral_point` (through only).
    Supply `spindle_rpm`, including optional group cutting data; omit `feed_mm_min`:
    rigid tapping derives feed as pitch × rpm. Reach includes chamfer and stock above the hole.
  - **Thread mills:** single-profile. `diameter` is the tooth's, so they also need
    `tooth_height`, `neck_diameter` (narrower than the tooth), `neck_length` and
    `plunge_mm_min`.
  - **Chamfer mills:** `flute_length`, `angle` (90 is the only one used), `tip_diameter` and
    `plunge_mm_min`.

  There are no defaults for these: camlib checks the holder against the stock, so a guessed
  holder would make that check meaningless. Optional: `shank_diameter`, `length_register`
  (default: `number`), `retract_feed_mm_min` (default: the feed), `flutes` (a whole number from
  1 to 1000; default 2 for drills and spot drills, 3 otherwise), `material` (`carbide` or `hss`;
  default `carbide`), `peck_mm` (drills: a positive
  cap on the depth rule’s peck increment; zero or absent means no cap), and for
  thread mills `min_pitch_mm`/`max_pitch_mm` (the pitches the tooth suits), `radial_passes`
  (default 2) and `spring_passes` (default 1).

  Drilling through 3×D uses one plunge; deeper holes use full-retract pecks of 0.5×D,
  capped by a positive `peck_mm`. Above 5×D, a plan warning and linked trace note name
  the hole and its depth/D ratio: deep-hole drilling (pilot plus deep-hole drill) is not
  planned, so full-retract pecks are used. Depth includes stock above the hole and
  requested breakthrough, and D is the selected drill's diameter.
- Library settings, with their defaults:

  | Setting | Default | Meaning |
  | --- | --- | --- |
  | `clearance_mm` | 10 | clearance height above the stock top |
  | `retract_mm` | 2 | retract and R plane above the stock top |
  | `breakthrough_mm` | 1 | how far a through hole is cut past its exit, at full diameter |
  | `drill_tolerance_mm` | 0.05 | how much smaller than the hole a drill may be (never larger) |
  | `gouge_tolerance_mm` | not declared (existing 0.001 mm material default) | permitted material penetration, shared by simulation and posting; Haas explicitly declares 0.05 mm with its reason; residual and output-coordinate tolerances are separate |
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

When contact geometry is available, later vise setups check finished-part contact on both jaws. The planner clips
analytically planar mesh faces to each jaw band and reports certain contact area in mm²;
conservative envelopes of drills, threads and edge breaks reduce that area. No contact on
either jaw withholds that setup. Missing contact geometry leaves contact unknown and retains
the conservative envelope warning without withholding the setup. Carried height-field stock is an upper bound and cannot
certify contact. This does not establish holding force. Flip seating and preparation still
need their separate checks.

Vise libraries may inventory `parallels_heights_mm`, with provenance in `_sources`.
The first setup chooses the tallest height satisfying grip limits; flip seating chooses
the deepest passing grip on already-finished sides. The support bars
are passed with the jaws as fixtures. A fixed `parallels_height_mm` remains supported.
Flip seating searches those real heights against the mesh. Without a library inventory or
fixed/override height, the cited rulebase standard set supplies heights, labelled
“standard size, not checked against your shop”; a standard-set miss retains the computed height or unmodelled support and warns once per setup, labelled “not a standard size”.

Standard blank and parallel sizes come from the cited rulebase data in
[`standard-sizes.json`](../rust/capp/rulebase/rules/standard-sizes.json), read 2026-11-14:
LittleMachineShop 1893 heights (½–1⅝ inch in ⅛ inch steps, exactly 25.4 mm/inch)
and Metal Stockist UK's aluminium flat bars (inch products use exact conversions, with supplier labels retained). A library's parallels inventory,
fixed height or setup override wins. Otherwise the first setup selects the tallest
standard height meeting grip limits; the flip selects the deepest grip passing
all limits and mesh seating; if preparation later fails, retry the next passing shallower seat. A standard-set miss keeps the prior computed height or unmodelled support and warns once per setup; an explicit shop inventory stays strict. No shop inventory is inferred.
First-setup stock uses the smallest containing bar area, trying both orientations,
with thickness along the tool axis, width across the narrower side, and sawn length
rounded up to whole mm. Extra thickness goes below the part, preserving the top margin and tool reach; side and length extra is shared
equally to preserve minimum margins (step2cam judgement). If no bar fits, keep the
computed size and warn. Later setups record remaining material, not purchased bars.
Plan `stock_size` and clamping `parallels_size` record provenance; the setup sheet
adds “standard size, not checked against your shop” only to standard sizes;
computed fallbacks say “not a standard size”.

Later side jaws use the full part mesh's extent within their grip-height band. A remote
boss outside that band does not set the opening; only analytic planar faces at the chosen
jaw planes count toward certain contact. Stock remains the full carried envelope for cutting.
