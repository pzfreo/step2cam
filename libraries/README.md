# Machine libraries

This directory contains resources-format libraries for named machines and
catalogue tooling, plus the page's generic LinuxCNC mill with placeholder tools and limits. Pass one as the third argument to the Rust `process` example,
or use `--tools libraries/haas-vf2.json` with the benchmark runner. The demo default (`demo/tools.json`) is the Carvera aluminium library described
below. The benchmark reference library (`tools/benchmark/tools.json`) is independent
and unchanged; generic planner regressions use `tests/fixtures/placeholder-tools.json`.

Every machine capability, vise dimension, holder envelope and tool family needs a
public source. Keep source URLs, catalogue identifiers, units, conversions and
cutting-data derivations in `_sources` or `_comment` (schema-supported metadata).
Do not fill missing manufacturer cutting data with guessed values: omit that
tool and document the gap. Catalogue dimensions describe nominal tools, not a
measured assembly. Cutting data are **starting values**, requiring setup and
material verification. Assembly choices and shop policies must be identified as
such, separately from manufacturer recommendations.

`machines[].controller` names the intended control (`haas_ngc`, `linuxcnc`,
`carvera`). It is metadata pending browser posting (#52). Carvera and Haas have
upstream Rust posts; browser posting waits on stevecad/camlib#188.
`controller_settings` holds only installed settings and machine limits. Its names
follow controller manuals or machining meanings, independently of any CAM engine.
Omission means unknown; `_sources` distinguishes catalogue configurations from
observed settings. Never substitute an engine default for an unknown machine fact.

## Generic LinuxCNC mill

`linuxcnc-vmc.json` replaces the page's generic machining centre. Its three linear
axes, limits, vise, tools and cutting data are placeholders copied from the
independent benchmark library; it describes no specific commercial machine.
The benchmark library is unchanged so controller identity cannot change its
plan outcomes or input hash. `_sources.controller` cites LinuxCNC 2.9 and camlib.

The new cutting ceiling, tool/length-offset ceilings and tool-change location
are explicitly **placeholders**, not LinuxCNC defaults. Their provenance and the
LinuxCNC configuration references are in the entry. No new stored value claims to
be a LinuxCNC default. Fixed language semantics stay in the upstream controller.

Optional vise `parallels_heights_mm` lists available matched-pair heights in mm; cite
its inventory under the vise's `_sources`. Without mesh seating, selection takes the tallest height compatible
with grip limits. Mesh seating defines the flip support geometry and required height;
an explicit inventory must include that height. Both kinds enter the fixture scene.
Without seating or inventoried support, support remains explicitly unchecked.
The shipped libraries have no parallels inventory; catalog availability alone is not shop inventory.


## Haas VF-2

The library models the standard 8,100 rpm CAT40 VF-2 with flood coolant and a
Kurt DX6 manual vise. `_sources.machine` records the checked Haas UK table and
archived Haas specification sheet: the main Haas page returned no spec table.
Rapids and maximum cutting feed are dealer/model-year figures, not universal
limits. The spindle's `max_tool_diameter_mm` is the **20 mm ER32 shank capacity**;
it is distinct from the changer's 89 mm full-pocket tool envelope and 12 lb
weight limit, which have no schema fields and are recorded in `_comment`.

The shop inventory has six Guhring aluminium end mills (single-flute 6793 at 3–16 mm,
four-flute RF100 A 3202 at 20 mm), fifteen Guhring 206 metric tap/clearance drills
including 4.4, 4.5 and 10 mm, Guhring 723 90° and 724 120° 10 mm spot drills
(the 120° one spots the 118° drills; the 90° one cuts countersinks), a TOYO
TCM90-12 90° chamfer mill, a Harvey 30302 90° tipped-off engraver, four Dormer E559 NO3 cut taps (M3/M4/M5/M6), and seven Harvey 8821xx single-form thread mills for
M3, M4, M5, M6, M8, M10 and M12 coarse pitches. This inventory exceeds 20 tools;
load only the tools a setup needs. Tool numbers identify inventory entries, not
a promise that all entries are in carousel pockets simultaneously. The planner
checks the number of distinct tools required by a setup.

All use the IMTE IMT100314 CAT40 ER32 chuck, 76.2 mm gauge projection and 50 mm
nut envelope. Fit the appropriate collet and retention knob; holder dimensions
are not the tool's stickout. Chosen stickouts and plunge policies are explicit
in `_policy`, rather than claimed as catalogue values. Feed/rpm calculations
are documented per family: rpm is capped at 8,100 **in the library**, preserving
maker chip load when calculating feed. Chart sizes between columns use the next
smaller diameter's chip load. Thread feeds include the chart's internal-circle correction at the named
nominal thread diameter; larger threads retain that conservative centerline feed. Thread pitch limits deliberately support only
the named coarse pitch, rather than claiming an unverified range.

The RF100 A 20 mm cutter's 0.45 mm corner chamfer is not represented by the flat-end-mill
resource type. Vise jaw thickness is conservatively modeled by the planner as
jaw height; the vise body, mounting, actual support/parallels and a complete
machine collision envelope are not represented by this library. Missing
parallels dimensions leave support unmodeled outside mesh-seated flip setups, as reported.

Omissions: the 6 mm ball mill has no supported resource type. The optional
16 mm chamfer cutter is absent because the sourced TOYO family stops at 12 mm;
no matching maker geometry/cutting-data pair was verified. Its 12 mm cutter's
published axial cutting length is retained even when a feature needs more reach.
Flute counts (`flutes`) are given where the cited source states them: the end mills,
the engraver, the thread mills (four) and the chamfer mill (seven). Tool `material`
is given where the cited source states it: the 206 drills and E559 taps are HSS, the
723 and 724 spot drills carbide. The others take the library defaults (2 flutes for a
drill or spot drill, 3 otherwise; carbide), which each job's notes name. The taps'
`_flutes` stays informational: their cited source does not state it.

Some MachinePlan stock exceeds the 406 mm Y travel or the DX6 opening and cannot
be cut with this configuration.

The 6793 geometry comes from Guhring's aluminium catalogue: the 6 mm has
20 mm LOC and 57 mm OAL, with a chosen 25 mm stickout. The 20 mm RF100 A
has 38 mm LOC / 104 mm OAL and a chosen 43 mm stickout. The old 3309 bright
reductions were valid for its chart; these replacements use their own published
aluminium rows. RF100 A's 1,625 SFM slotting row gives 7,883 rpm and
4,805.48 mm/min at four flutes and .0060 inch/tooth. This is an aluminium
starting point, not a verified cycle. The 6793 uses its single-flute slotting
row, with feed computed for one flute. Stickouts, engagement and plunge policies
are explicitly marked step2cam judgement. Harvey's engraver uses its own
non-ferrous row, including the chart's 50% vertical-plunge reduction.

## Carvera demo

The demo models the optional **1/4 inch maximum shank capacity (6.35 mm)**
Carvera configuration, with the original 1/8 inch Essential set tools retained.
It is a shared shop inventory supplemented for aluminium plate work, not a claim that
the machine ships with these twenty tools. Makera confirms a free starter
kit, but a complete itemised in-box cutting inventory was not verified.

Collets must match each shank: the 6 mm and 4 mm Makera cutters require the
corresponding optional collets; kit cutters use 1/8 inch; the supplementary
thread mill uses the optional 3 mm collet; the spot drill and pointed engraver
use 1/4 inch. A 1/4 inch collet cannot grip all these sizes. Manual collet changes
are required. The machine declares `spindle.collet_change: "manual"`. Every setup selecting
more than one shank size receives a traced setup warning independently of ATC
pocket count; neither this warning nor the maximum-capacity check changes tools
or establishes mixed-shank ATC compatibility. Six pockets describe the changer, not the whole inventory;
load tools for the setup and arrange the manual changes before running it.

Makera's metal O-flute cutters now include **4 x 22 mm** and **6 x 25 mm**,
alongside the Essential set's 3.175 x 12, 2 x 8 and 1 x 3 mm cutters. Their
published generic **Aluminum** presets remain 12,000 rpm / F500 / plunge200 /
DOC0.2 mm for the larger sizes (DOC0.15 for 2 mm, DOC0.1 for 1 mm). These are
not specifically 6082 data. Each tool cites the Fusion file and read date.
The kit drills, chamfer/engraving bits and M3/M5 thread mills are retained.
Ball ends have no supported resource/adapter type; wood/composite bits are excluded.

Supplementary catalogue tools, with geometry and cutting derivations cited per
tool in `demo/tools.json`, are:

- Harvey 35862 uncoated square long-flute mill: actual diameter .0620 inch
  (1.5748 mm), LOC24.13 mm, OAL63.5 mm, shank3.175 mm, three flutes. This
  is optional inventory for deep small features; no fixture-specific choice forces it. The
  maker's 15D finishing chart supplies aluminium chip load; shallow-layer
  pocketing engagement/plunge are explicitly step2cam judgement, not a maker
  slotting recommendation. This slender tool is intentionally fed slowly.
- Guhring 5515 carbide non-through-coolant drill: D3.3 mm, 6 mm shank,
  LOC28 mm / OAL66 mm / maximum cutting depth23.05 mm, 140° point. This
  supplies the deep pilot holes the kit drills cannot reach. Speed/feed/peck
  reductions for the 200 W air-assisted spindle are step2cam judgement,
  derived from the maker's 5515 wrought-aluminium row.
- Harvey 949516 uncoated spot drill: D/shank6.35 mm, LOC19.05 mm, 150° point,
  suitable for spotting the 140° drill and the kit's 118° drills. Cutting data
  are reduced from its own aluminium recommendation and labelled judgement.
- Harvey 25030 pointed engraver: 30°, 6.35 mm shank, catalogue LOC11.8364 mm.
  The usable cutting diameter is conservatively derived from that LOC and
  angle, rather than extending the rounded catalogue length to fit an ideal
  cone. Its narrower angle leaves some of the wider V-channel's walls
  unfinished, as the planner warns; it is not claimed to finish that channel.
- Harvey 737624 single-form M3x0.5 thread mill: D2.30 mm, 9.5 mm maximum
  thread depth, 3 mm shank. It reaches the front threads with more pilot-entry
  clearance than the kit's 2.42 mm head. Its own 7x-neck aluminium chart is used,
  with actual four-flute chip load and internal-circle feed correction.

The shared library has **no supplied decisions and no stock-grid override**.
The 5 mm slot `slots:19.20` is an obround with r2.5 ends. Its former forced
small cutter was aimed at the wrong feature and is removed. The r1.2 corners
belong to the 6 mm notch `slots:37.40.41.44`. The existing notch selector chooses
the largest cutter fitting the width and reaching the depth: with this inventory
that is D4, leaving r2 corners, explicitly warned against the model's r1.2.
It does not choose by corner radius. Improving that outcome without a
part-specific override requires separate corner-aware selection/rest-machining
work; this PR leaves the established selector unchanged.

The front M3 thread may be refused under the default stock grid: the planner's
ordinary kit-mill selection is not replaced with a fixture-specific tool, and
camlib must still prove its entry open. No Carvera taps are supplied: Makera's
manual/software documentation and published profiles were checked, but no
manufacturer documentation of rigid tapping/spindle-feed synchronization was
found. This is an inventory decision, not a claim about all possible firmware.

Makera publishes no enabled end-mill stepover; 0.4D is step2cam judgement.
Flute counts are `flutes`, from Makera's NOF or the Harvey and Guhring pages;
feed calculations use these counts. Only the supplementary Guhring 3.3 mm drill
has a sourced `material` (carbide); the others are sent to camlib as carbide by
default and each job's notes say so. Assembly envelopes,
manual-collet requirements and other policies are recorded in `_comment` and
`_policy`. Tool coolant is off (the enum cannot express air); use external air
assist for chip clearing. The placeholder vise is unchanged. Vendor files are
not redistributed, only cited dimensional/cutting facts.

This improves coverage rather than making an industrial-speed cycle. Makera's
0.2 mm DOC and F500 require many roughing passes. The miniature cutter is no longer forced onto the ordinary obround slot. A couple-of-hours target
cannot be claimed from these starting data; it would require separately validated
more aggressive cutting data or a different process/tool combination. Some stock
below the jaws and edge rounds still require later workholding or hand finishing.

Wall finishing now uses the fewest equal axial passes allowed by flute length,
normally one pass for an admitted wall, independently of roughing stepdown.
The selected depth and pass count are traced; camlib still checks full reach,
stock contact and material. Carvera estimates remain limited by Makera's
conservative presets until #38/#37.

The Haas taps use Dormer's manufacturer-authored 2024 threading catalogue,
p32/p36, linked in `_sources.taps`. E559 NO3 is a straight-flute bright HSS
bottoming cut tap suitable for machine use, form C (2–3 pitches). The full
three-pitch lead is modelled. Geometry is verified individually, including the
M6's **6.00 mm** shank, rather than the suggested 6.3 mm. The conservative
hard-wrought-aluminium starting row is Vc6 m/min; synchronous feed is derived
from rpm and pitch. A shallow blind pilot can still reject this lead, and that
rejection must not be hidden by using the lower end of the lead range. Haas
rigid tapping must be enabled; use compatible tapping collets in the sourced
CAT40 envelope. Assembly stickouts are step2cam judgement.

## Posting contract inventory

Audited 2026-11-13 against `vendor/camlib/src/gcode/machine.rs` and the Rust
`Controller` variants in `haas.rs` and `carvera.rs`,
using [Posting without Python](https://github.com/stevecad/camlib/blob/6810078b6388f750cbcb1d4c57fc3e6c79eedd5b/docs/posts.md#posting-without-python).
This is an adapter contract inventory, not a second machine definition.
**1** maps an existing value; **2** is controller behaviour (P marks a separate
adapter post-policy decision, not a stored machine fact); **3** is a machine
setting, omitted when unknown; **4** is supplied per job. The only resource-to-engine
name translation is [`machine_fields`](../rust/src/post_machine.rs).

| Engine field | Class | Resource / responsibility |
|---|---|---|
| name | 1 | machine.name, falling back to id |
| tools | 4 (1) | Selected tools only: tool id/number; gauge length = stickout + holder_length; job assigns length/diameter registers and final positions |
| controller | 2 / 3 / 4 | Named Haas/Carvera block; generic LinuxCNC omits it. Unknown premises stay absent |
| dialect | 2 | Upstream controller selected by controller; includes fixed command/cycle forms and framing |
| work_offset | 4 | Plan's selected work offset |
| frame | 4 | Setup frame |
| work_to_machine | 4 | Located setup in machine coordinates |
| tip_travel_mm | 4 (1) | Transform axes.travel_mm with located setup/tool geometry; travel extents alone do not locate machine zero |
| gauge_travel_mm | 3 | Spindle gauge line's soft limits in machine coordinates, declared instead of tip_travel_mm; unknown on all three (axes.travel_mm gives lengths, not limits) |
| output_units | 2 P / 3 | Adapter chooses mm; controller_settings.required_output_units only when controller fixes units (Carvera) |
| decimals | 2 P | Adapter 3 metric / 4 inch; also within Haas precision limits |
| tolerance_mm | 2 P | Adapter .001 mm (metric) / .003 mm (inch), including 3D coordinate quantisation; not the part's tolerance |
| feed_tolerance_mm_min | 2 P | Adapter .01 mm/min |
| traverse_feed_mm_min | 2 P (3) | LinuxCNC native rapid (null); Haas feed traverse at declared cutting ceiling; unknown for Carvera |
| max_feed_mm_min | 3 | Cutting-feed ceiling |
| max_spindle_rpm | 1 | spindle.max_rpm |
| planes | 2 | Upstream controller capability, restricted to XY for Haas reference |
| arcs | 2 P | LinuxCNC native; Haas linear policy; Carvera implementation unknown |
| helices | 2 P | As arcs |
| max_sweep_rad | 2 P | Adapter single-turn ceiling, 2 pi |
| max_tool_number | 3 | Addressable tool identity ceiling, **not** tool_changer.pockets |
| max_length_register | 3 | Length-offset address ceiling, independent of pockets |
| tool_changes | 1 | tool_changer.kind; pockets still limit tools loaded for a setup |
| flood | 1 | coolant contains flood |
| mist | 1 | coolant contains mist; air is not mist |
| revision | 2 P | Version of adapter decisions; changes with that contract |
| tool_change_position_mode | 3 | Controller's final tool-change position semantics (camlib's preserve_spindle is not mapped) |
| tool_change_home_z_mm | 3 | Gauge line's final machine Z, only for homing changes; null otherwise |
| rapid_mm_min | 1 | Existing library.rapid_mm_min; verify estimated rates before real posting |
| max_normal_accel_mm_s2 | 3 | Optional measured/configured normal acceleration ceiling; unknown on all three |
| max_axis_feed_mm_min | 3 | Optional independent axis cutting-feed limits; unknown on all three |
| motion_policy | 2 P | Optional scheduling policy; adapter does not enable it |
| schedule_post_feeds | 2 P | Adapter false; no scheduling without a declared policy |
| crash_model | 3 + 4 | Machine geometry plus located setup/tools. Unknown on all three; vise dimensions are insufficient. No engine-shaped geometry is stored in resources |

The existing library.tool_change_gauge_z_mm / tool_change_xy_mm are setup-frame
simulation premises. They do not duplicate the new machine-coordinate final
homing Z setting, and their shapes remain unchanged.

The nested `Dialect` contract is class 2 by default: commands, work_offsets, axes, centers,
tool_word, length_word, feed_word, speed_word, parentheses_comments, tool_change,
length_activation, radius_tolerance_mm, radius_relative_tolerance, fixed_modes,
tap_pitch_word, compensation_word, compensation_with_move, compensation_register,
dwell_word, dwell_resolution_s, dwell_max_s, cycle_dwell_unit, peck_reentry_mm,
chip_break_lift_mm, oriented_edge_deg, beam_s_max, beam_effect, beam_mark, x_diameter,
thread_lead_word, height_control_word, css_limit_word, probing,
probe_fast_feed_mm_min, probe_switching. Installed options/peck distances can
parameterise those behaviours; #52 passes the declared machine settings upstream,
rather than reimplementing a dialect here. Exceptions to fixed behaviour:
`oriented_edge_deg` is class 4 (the tool's oriented cutting edge in work XY);
`probing`, `probe_fast_feed_mm_min` and `probe_switching` depend on the installed
probe backend/macros (class 3), and `beam_s_max` on the installed beam source
(class 3). These remain null/disabled in the upstream LinuxCNC milling subset;
none is a claim about an installed probe or beam source. Their resource format
and setup declarations must be added if those operations are introduced.
LinuxCNC arc-radius acceptance defaults belong to its canonical dialect; an
installation overriding those INI values will need an explicit setting adapter.
Haas peck-return parameters depend on Settings 22/52 as inventoried above.

Haas adds these requirements under Rust `Machine.controller` (`name: "haas_ngc"`).
The audit qualifies their names as `controller.<field>`; the output nests them.
Carvera (`name: "carvera"`) adds `initial_gauge_length_mm`, `unmodified_coordinates`
and `automatic_tool_length`: all remain unknown until #52 obtains the setup and
calibration premises. Controller/dialect completion and posting remain #52.

| Haas reference field / parameter | Class | Responsibility |
|---|---|---|
| coordinate_system | 3 | Setting 33 Coordinate System; unknown |
| h_t_agreement | 3 | Setting 15 H and T Code Agreement; unknown |
| g92_shift_mm | 3 | Current G92 coordinate shift; unknown, never assumed zero |
| rotation_scaling | 3 | Installed Rotation and Scaling option; unknown |
| rigid_tapping | 3 | Rigid Tapping option; catalogue VF-2 includes it |
| cutter_compensation | 2 P | Adapter disables wear compensation; enabling later requires per-job D registers/wear |
| setting_22_mm | 3 | Setting 22 Can Cycle Delta Z; unknown |
| setting_52_mm | 3 | Setting 52 G83 Retract Above R; unknown, never assumed zero to admit G83 |
| program_number | 4 | Job's O number (1..99999), not machine identity |
| optional_stop_enabled | 3 | Setting 17 / panel state; unknown |
| mirror_settings_xyz | 3 | Settings 45/46/47; unknown, never assumed OFF |
| curve_policy | 2 P | Adapter linear; exact-IJK is a later explicit post policy |
| reference | 2 | Upstream manual/version identifier, 96-8210 Rev N |
| name, tools, work_to_machine, traverse_feed_mm_min, **settings | as above | define_machine parameters forward the general contract; no extra stored copy |

Haas source checks: [VF-2 specification](https://www.haas.co.uk/machines/vf-2/),
[NGC 96-8210 Rev N, Tool Offsets p165](https://www.haascnc.com/content/dam/haascnc/en/service/manual/operator/english---mill-ngc---operator%27s-manual---2020.pdf),
[Setting 15](https://haascnc.com/service/codes-settings.type=setting.machine=mill.value=S15.html),
[22](https://haascnc.com/service/codes-settings.type=setting.machine=mill.value=S22.html),
[33](https://haascnc.com/service/codes-settings.type=setting.machine=mill.value=S33.html),
[52](https://haascnc.com/service/codes-settings.type=setting.machine=mill.value=S52.html),
read 2026-11-10. The setting pages establish meanings, not installed values or
verified defaults for this VF-2. The PDF direct fetch failed; maker-indexed p165
text confirmed the offset count. Highest carousel tool identity, final M6 gauge
position, rotation/scaling, shifts and panel states remain unknown.

LinuxCNC's four new values are chosen placeholders derived from existing limits,
not invented manufacturer data. [INI configuration](https://linuxcnc.org/docs/2.9/html/config/ini-config.html)
and [tool compensation](https://linuxcnc.org/docs/2.9/html/gcode/tool-compensation.html)
were read 2026-11-10. Carvera stores only its documented millimetre requirement
([community G-codes](https://carvera-community.gitbook.io/docs/firmware/supported-commands/gcodes),
read 2026-11-10; see also [research](../docs/carvera.md)). Its cutting-feed ceiling,
T/H range, final M6 location and firmware options remain unknown. Catalogue
travel speed and the profile's commanded G53 retract do not establish those facts.

`rust/tests/post_machine.rs` audits every Rust field directly from the pinned
upstream struct, and every selected Haas/Carvera Rust controller field; a newly required,
unclassified field fails. It prints covered sources, controller, per-job and
unknown fields for each shipped machine (`-- --nocapture`). LinuxCNC's partial
mapping plus an explicit sample setup/tool table is deserialised and validated by
camlib itself. Its dialect is read from camlib's own Rust fixture **only in the
test**, not copied into production or resources. Unknown optional limits stay
omitted; successful validation does not prove a collision-safe physical machine.

camlib's Rust side has no ready-made LinuxCNC dialect yet (Python has
`GCodeDialect.linuxcnc()`), so production dialect selection waits for camlib; see #52.


### Vise force provenance

Optional vise `clamp_force_n` is a published normal force at each jaw; `clamp_torque_nm`
states the torque associated with that force. Positive finite values and a citation in
`_sources.clamp_force` (`url`, `read`, `note`) are required when a force is present.
Never infer force from dimensions or put a guessed value on a placeholder vise.

The Haas Kurt DX6 uses the [Kurt DX6 manual](https://docs.travers.com/assets/pdf/61-421-171_manual.pdf),
read **2026-11-13**, printed/PDF p4 force table: **3440 lbf at 40 ft·lbf**, converted to
**15301.88235649612 N at 54.232717933256 Nm**. Choosing 40 ft·lbf is step2cam judgement:
moderate hand tightening, half the rated maximum 80 ft·lbf. The manual's 10 ft·lbf row is
512 lbf; the manual table supersedes the product-page figures. Its p5 says the supplied
handle with correct torque provides the needed force; no extensions or impacts. Apply
the planned torque and verify actual grip. Force and torque are paired in the setup's
clamping and candidate grip data. The demo’s unvalidated vise has no force and gets a
not-checked warning, with no cutting-data changes.

[The grip rule](../docs/capp-plan.md#vise-grip-against-the-cut-m3) documents μ = 0.095
(machined/sawn aluminium side in smooth steel jaws), safety factor 2, ISCAR aluminium
cutting constants and full-width roughing engagement. Axial cycles are thrust into
supports, not checked against sliding capacity; helix milling remains a sideways check.
Holding capacity does not scale with contact area; pressure and overturning remain unchecked.
