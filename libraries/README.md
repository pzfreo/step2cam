# Real machine libraries

This directory contains opt-in resources-format libraries for named machines and
catalogue tooling. Pass one as the third argument to the Rust `process` example,
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

`machines[].controller` is an optional string naming a camlib post. The VF-2
uses `haas_ngc`, corresponding to camlib's `examples/haas_ngc_post.py` for Haas
Next Generation Control. It is metadata only: no Rust code consumes it, no
post is selected and no G-code is emitted. It anticipates
[camlib Rust posting support](https://github.com/stevecad/camlib/issues/120).

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
including 4.4, 4.5 and 10 mm, a Guhring 723 10 mm 90° spot drill, a TOYO
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
parallels dimensions leave support unmodeled, as the planner reports.

Omissions: the 6 mm ball mill has no supported resource type. The optional
16 mm chamfer cutter is absent because the sourced TOYO family stops at 12 mm;
no matching maker geometry/cutting-data pair was verified. Its 12 mm cutter's
published axial cutting length is retained even when a feature needs more reach.
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
Flute counts are `_flutes` metadata because the resource format has no native
flute-count field; feed calculations use actual counts. Assembly envelopes,
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
