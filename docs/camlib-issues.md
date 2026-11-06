# Issues to raise on camlib

Draft issues for [stevecad/camlib](https://github.com/stevecad/camlib), found while connecting
step2cam to it. None of them is a camlib bug: in each case camlib correctly refused input it
could not prove safe or consistent. The docs or the error message made the cause hard to find.

They were checked against camlib `d05c0df` (0.1.0) and its open issues on 2026-11-06; none of
them is reported yet. The PyPI name clash (the `camlib` package on PyPI is an unrelated web
library) is already camlib #21, so it is not repeated here.

Each section below is one issue: the heading is the title, the rest is the body.

---

## SurfaceMesh: document the 0.1 mm `surface_error_mm` limit, and say which check failed

**What happened**

A `Roughing` operation whose target was a `SurfaceMesh` with `surface_error_mm=0.117` failed
while posting with:

```
camlib.simulation.SimulationError: simulation stream: invalid surface mesh version/units/error/triangle count
```

The cause was the limit in `SurfaceMesh::prepare` (`src/surface.rs`), which accepts
`surface_error_mm` only in `0..=0.1`. That limit is not in `docs/operations.md` (Roughing) or
`docs/python.md` (`camlib.surface`). The message covers four checks at once (version, units,
error, triangle count), so it does not say which one failed.

**Why it matters**

A mesher that bounds its error relative to part size passes 0.1 mm at about a 100 mm part, and
the user cannot tell from the message why roughing refused it. The mesh in this case came
from another tool (specify-core's triangulation, 0.1% of the part's diagonal), not from
`camlib.cadquery.export_surface_mesh`, whose `max_error_mm` range does show the limit.

**Suggestions**

- Document the 0.1 mm limit under Roughing (and the other mesh operations) and on
  `SurfaceMesh`.
- Give each check its own message with the value and the limit, for example
  `surface_error_mm 0.117 exceeds the 0.1 mm limit`.
- Optionally, check it when the `SurfaceMesh` is built in Python, so it fails at the call that
  made it rather than during posting.

---

## Multi-tool jobs: post fails at the first tool change unless `Setup.tool_change` matches the tool table

**What happened**

A three-tool milling job (facing and roughing with an end mill, then two drills) used the
pattern in `docs/getting-started.md`: each `ToolSetting.for_tool(...,
position_after_mm=header.initial_position_mm)`, `tool_change_position_mode="table"`, and no
`Setup(tool_change=...)`. Posting failed at the first tool change:

```
ValueError: event 1981: decoded tool-change pose deviation 2.392918452288283mm exceeds output tolerance
```

Without `Setup.tool_change` the planner keeps the tool tip where the previous operation ended,
while the post's table puts each new tool at `position_after_mm`, so the two disagree. The
fix, declaring `Setup(tool_change=ToolChangePosition(gauge_z_mm=..., xy_mm=...))` and setting
each tool's `position_after_mm` to the matching tip (`gauge_z_mm` minus its gauge length),
worked. `docs/posts.md` mentions this for `z_home`, but not as needed for `table`.

**Why it matters**

The getting-started example works only because it uses a single tool. Copying it for a
second tool gives an error during posting that does not point at `Setup.tool_change`.

**Suggestions**

- Show a multi-tool job in getting-started, with `Setup(tool_change=ToolChangePosition(...))`
  and matching `position_after_mm`.
- Check, before posting, that each tool's declared pose after a change agrees with the setup's
  tool-change pose, or name `Setup.tool_change` / `position_after_mm` in the error.

---

## Undercut neck check: give the clearance and the cell size it needs

**What happened**

Thread milling M5 (⌀4.2 drilled hole) with a single-profile thread mill (⌀3 tooth, ⌀2 neck)
failed with:

```
the tool neck may touch material beside an undercut (thread groove); use a narrower neck or a finer stock cell size to prove the clearance
```

The neck passes 0.1 mm from the drilled wall, and the stock grid was 0.26 mm. A grid of
about 0.057 mm (the clearance over a cell's diagonal, with a little in hand) proved it.

**Suggestion**

The message is already helpful. It would be more so if it also gave the neck's clearance
and the coarsest cell size that can prove it, as this is what the caller has to work out
next. Real single-profile thread mills for small threads run this close to the wall, so this
will be common.
