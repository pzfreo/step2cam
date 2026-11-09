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
TCM90-12 90° chamfer mill, a Harvey 30302 90° tipped-off engraver, and seven Harvey 8821xx single-form thread mills for
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

The demo assumes the standard 1/8 inch (3.175 mm) collet and the aluminium-relevant
subset of Makera's **Essential Milling Bit Set - Standard (26 pcs)**. The free
starter toolkit is confirmed by Makera, but no complete itemised in-box cutting
inventory was verified. The set's itemised reseller listing is linked in the
library. It supplies metal O-flute end mills (3.175 x 12, 2 x 8, 1 x 3 mm),
drills (1, 2, 2.5, 3 mm), the 90° chamfer bit, metal engravers (60° x .1,
30° x .2/.3), and M3/M5 thread mills. The M5 cutter's 4 mm head has a
3.175 mm shank. No optional collet is needed. Six pockets describe the changer,
not the whole inventory; load each setup's selection.

Geometry and generic **Aluminum** presets were read from Makera's Fusion files
on 2026-11-09, cited per tool. They are not specifically 6082 data. The main
end mill uses 12,000 rpm / F500 / plunge200 / DOC0.2 mm; the 2 mm uses DOC0.15,
and 1 mm DOC0.1. Small real flute lengths can leave deep features uncut;
larger holes are helix-milled where reach permits. These shallow cuts make plate
roughing take much longer than on the VF-2. Ball ends have no supported resource
or camlib adapter type. Wood/composite bits are excluded.

Makera's file supplies no enabled end-mill stepover; 0.4D is step2cam judgement.
Flute counts are recorded as `_flutes` metadata because the resource format has
no flute-count field; feeds already incorporate the actual counts. Assembly
projection/envelopes and thread-pass assumptions are marked in `_policy`.
Tool coolant is off (the enum cannot express air); use the Carvera's external
air assist for chip clearing. The placeholder vise is retained unchanged.
Vendor files are not redistributed, only cited dimensional/cutting facts.

The stepped plate's 5 mm slot needs 19 mm flute reach. The Essential set's
2 mm metal cutter has only 8 mm LOC, so this inventory does **not** finish that
slot or its r1.2 corners. The planner reports the missing reach; a longer small
diameter metal cutter outside this set would be needed. This limitation is
retained in the Carvera integration test rather than disguising it with invented
geometry or changing planner rules.
