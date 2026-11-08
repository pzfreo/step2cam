# Real machine libraries

This directory contains opt-in resources-format libraries for named machines and
catalogue tooling. Pass one as the third argument to the Rust `process` example,
or use `--tools libraries/haas-vf2.json` with the benchmark runner. The demo default
and benchmark reference library are independent and unchanged.

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

The shop inventory has six Guhring 3309 flat end mills (3–20 mm), thirteen Guhring
206 metric tap/clearance drills, a Guhring 723 10 mm 90° spot drill, a TOYO
TCM90-12 90° chamfer mill, and seven Harvey 8821xx single-form thread mills for
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

The 3309 catalogue's tiny corner chamfer is not represented by the flat-end-mill
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
