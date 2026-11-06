#!/usr/bin/env python3
"""step2cam: a STEP part's recognised features -> camlib toolpaths and G-code.

Two pip-installed Python libraries, called in turn:

    specify_core.api.analyse(step)  ->  analysis   (holes, counterbores, ... by pzfreo/quiddity)
    specify_core.api.mesh(step)     ->  mesh       (triangles, for 3D roughing)
    step2cam.plan(analysis, mesh)   ->  cam plan   (what to cut, per setup; plain JSON)
    step2cam.post(plan, tools)      ->  G-code     (how: your tools and machine, by camlib)

From Python:

    import step2cam
    plan = step2cam.plan_step("part.step")                 # needs specify-core
    step2cam.post(plan, json.load(open("tools.json")), "out/")  # needs camlib

From the command line:

    step2cam step part.step -o part.plan.json
    step2cam post part.plan.json --tools tools.json -o out/

`plan` also reads analysis and mesh JSON files that `specify-core analyse`/`mesh` wrote.

The plan says *what* to cut and is independent of tools and machine: the stock, the setups
(one per direction a hole is entered from), and in each setup the facing, roughing, holes and
counterbores, in the setup's own coordinates. `post` decides *how*: which tool cuts each one,
with the cutting data in the tool library, and camlib plans, simulates, checks and posts it.

Coordinates: each setup is turned so its tool comes down -Z, then moved so X0 Y0 is the stock's
minimum corner and Z0 the stock top (every cutting Z is negative). The plan records the
rotation and the model point that became the origin, so the setup can be found on the machine.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

PLAN_FORMAT = "specify-cam-plan"
PLAN_VERSION = 1

# The directions a tool can come from, by the hole axis it enters along (`axis` points from
# the opening into the hole), as rotations (rows) that turn that axis to -Z. Each is a proper
# rotation, so a setup is the part turned over, never mirrored.
SETUPS = (
    ("top", (0, 0, -1), ((1, 0, 0), (0, 1, 0), (0, 0, 1))),
    ("bottom", (0, 0, 1), ((1, 0, 0), (0, -1, 0), (0, 0, -1))),
    ("front", (0, 1, 0), ((1, 0, 0), (0, 0, 1), (0, -1, 0))),
    ("back", (0, -1, 0), ((1, 0, 0), (0, 0, -1), (0, 1, 0))),
    ("left", (1, 0, 0), ((0, 0, 1), (0, 1, 0), (-1, 0, 0))),
    ("right", (-1, 0, 0), ((0, 0, -1), (0, 1, 0), (1, 0, 0))),
)
SETUP_ORDER = [name for name, _, _ in SETUPS]

# An axis counts as along a setup direction within this (cosine); oblique holes are skipped.
AXIS_TOLERANCE = 1e-6
# Model coordinates are rounded to this many decimals in the plan (mm).
DECIMALS = 4

# Families quiddity recognises that this bridge does not turn into operations of their own.
NOT_PLANNED = {
    "pockets": "pocket",
    "slots": "slot",
    "channels": "channel",
    "step_levels": "step",
    "risers": "riser",
    "bosses": "boss",
    "plates": "plate",
    "flats": "flat",
    "chamfers": "chamfer",
    "section_recesses": "recess",
}
TURNED = {"turned_bodies", "turned_steps", "face_grooves"}


class PlanError(ValueError):
    """The input cannot be planned (wrong format, no part, a mesh of another file)."""


# -------------------------------------------------------------------------------------------
# Small vector helpers
# -------------------------------------------------------------------------------------------


def _dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def _add(a, b):
    return [x + y for x, y in zip(a, b)]


def _scale(a, k):
    return [x * k for x in a]


def _rotate(rows, p):
    return [_dot(row, p) for row in rows]


def _unit(v):
    n = math.sqrt(_dot(v, v))
    if not n or not math.isfinite(n):
        raise PlanError(f"axis {v} has no direction")
    return [x / n for x in v]


def _r(x):
    """Rounded for the plan, with no negative zero."""
    return round(x, DECIMALS) + 0.0


def _rp(p):
    return [_r(x) for x in p]


def _setup_for(axis):
    """The setup whose tool enters along `axis`, or None for an oblique axis."""
    a = _unit(axis)
    for name, direction, rows in SETUPS:
        if _dot(a, direction) > 1 - AXIS_TOLERANCE:
            return name, rows
    return None


def _box_corners(lo, hi):
    return [[x, y, z] for x in (lo[0], hi[0]) for y in (lo[1], hi[1]) for z in (lo[2], hi[2])]


def _bounds(points):
    return (
        [min(p[i] for p in points) for i in range(3)],
        [max(p[i] for p in points) for i in range(3)],
    )


# -------------------------------------------------------------------------------------------
# plan: analysis (+ mesh) -> cam plan
# -------------------------------------------------------------------------------------------


class Frame:
    """One setup's coordinates: model point -> rotate -> translate to the stock origin."""

    def __init__(self, name, rows, stock_lo, stock_hi):
        self.name = name
        self.rows = rows
        corners = [_rotate(rows, p) for p in _box_corners(stock_lo, stock_hi)]
        lo, hi = _bounds(corners)
        # X0 Y0 at the stock's minimum corner, Z0 at its top.
        self.shift = [-lo[0], -lo[1], -hi[2]]
        self.stock = _bounds([self.point(p) for p in _box_corners(stock_lo, stock_hi)])
        # The model point at this setup's origin: rotation rows are orthonormal, so the
        # inverse is the transpose.
        back = [-s for s in self.shift]
        self.origin_model = [sum(rows[i][j] * back[i] for i in range(3)) for j in range(3)]

    def point(self, p):
        return _add(_rotate(self.rows, p), self.shift)

    def describe(self):
        return {
            "rotation": [list(row) for row in self.rows],
            "origin_model": _rp(self.origin_model),
            "stock": {"min": _rp(self.stock[0]), "max": _rp(self.stock[1])},
        }


def _hole_entries(record):
    """The ways a hole can be cut: (axis, mouth, through-reversed?) for its own opening and,
    for a plain through hole, the other end too."""
    axis = _unit(record["axis"])
    mouth = list(record["location"])
    yield axis, mouth, False
    plain = not any(record.get(k) for k in ("cbore", "spotface", "csink", "outer_steps"))
    if record.get("bottom") == "through" and plain:
        yield _scale(axis, -1), _add(mouth, _scale(axis, record["depth"])), True


def _hole_geometry(feature, frames, preferred):
    """A hole as setup-frame geometry: which setup, its centre, the bore's Z range and its
    enlargements; or a reason it is skipped."""
    record = feature["record"]
    options = []
    for axis, mouth, reversed_ in _hole_entries(record):
        found = _setup_for(axis)
        if found is not None:
            options.append((found[0], axis, mouth, reversed_))
    if not options:
        return None, f"oblique axis {_rp(record['axis'])}: needs a setup this bridge does not make"
    rank = {name: i for i, name in enumerate(preferred)}
    name, axis, mouth, reversed_ = min(options, key=lambda o: rank[o[0]])
    frame = frames[name]

    # quiddity: `location` is the axis point at the opening; `diameter`/`depth` describe the
    # narrowest bore, `depth` measured from the top of that bore; enlargements (outer steps,
    # outermost first, then the counterbore or spotface) sit between the opening and the bore.
    steps = [] if reversed_ else list(record.get("outer_steps") or [])
    for key in ("cbore", "spotface"):
        if record.get(key) and not reversed_:
            steps.append(record[key])
    enlargements = []
    at = mouth
    for step in steps:
        floor = _add(at, _scale(axis, step["depth"]))
        enlargements.append(
            {
                "diameter": _r(step["diameter"]),
                "top_z": _r(frame.point(at)[2]),
                "floor_z": _r(frame.point(floor)[2]),
            }
        )
        at = floor
    bore_top = at
    bottom = _add(bore_top, _scale(axis, record["depth"]))
    centre = frame.point(mouth)
    hole = {
        "feature": feature["id"],
        "center": [_r(centre[0]), _r(centre[1])],
        "top_z": _r(centre[2]),
        "bore_top_z": _r(frame.point(bore_top)[2]),
        "bottom_z": _r(frame.point(bottom)[2]),
        "diameter": _r(record["diameter"]),
        "bottom": record.get("bottom", "unknown"),
        "enlargements": enlargements,
    }
    if record.get("csink") and not reversed_:
        c = record["csink"]
        hole["csink"] = {
            "major_diameter": _r(c["major_diameter"]),
            "included_angle": _r(c["included_angle"]),
        }
    if reversed_:
        hole["entered_from_exit"] = True
    return (name, hole), None


def _group(holes, key, kind):
    """Holes with the same key as one operation over their centres."""
    groups = {}
    for hole in holes:
        groups.setdefault(key(hole), []).append(hole)
    ops = []
    for k in sorted(groups):
        members = groups[k]
        op = {"kind": kind}
        op.update(dict(k))
        op["centers"] = [h["center"] for h in members]
        op["features"] = [h["feature"] for h in members]
        ops.append(op)
    return ops


def _mesh_for(mesh, frame):
    positions = mesh["positions"]
    out = []
    for i in range(0, len(positions), 3):
        out.extend(_rp(frame.point(positions[i : i + 3])))
    return out


def _model_bounds(dimensioning):
    """The part's box in model coordinates. quiddity gives `bounds` in its dimensioning
    `frame` (a point is origin + x*bx + y*by + z*bz), so the box's corners are carried back;
    with a frame turned against the model axes the box is the (looser) box of that box."""
    bounds, frame = dimensioning.get("bounds"), dimensioning.get("frame")
    if not bounds:
        return None
    if not frame:
        return bounds
    corners = []
    for c in _box_corners(*bounds):
        p = list(frame["origin"])
        for k, axis in enumerate(("x", "y", "z")):
            p = _add(p, _scale(frame[axis], c[k]))
        corners.append(p)
    return _bounds(corners)


def plan(analysis, mesh=None, *, margin=2.0, top_stock=1.0, bottom_stock=0.0, skin=0.5,
         rough_from="top", prefer=None):
    """The cam plan for an analysis (and optionally its mesh), as a JSON-ready dict."""
    if not isinstance(analysis, dict) or "features" not in analysis or "dimensioning" not in analysis:
        raise PlanError("not a specify-core analysis (specify_core.api.analyse, or `specify-core analyse`)")
    bounds = _model_bounds(analysis["dimensioning"])
    if mesh is not None:
        if mesh.get("format") != "pmi-assist-mesh":
            raise PlanError("not a specify-core mesh (specify_core.api.mesh, or `specify-core mesh`)")
        if mesh.get("binding") != analysis.get("binding"):
            raise PlanError("the mesh and the analysis are of different files or parts")
        bounds = mesh.get("bbox") or bounds
    if not bounds:
        raise PlanError("the analysis has no part bounds")
    part_lo, part_hi = [list(map(float, b)) for b in bounds]

    # The stock is the part's box grown by the margins, in model coordinates: `top_stock` and
    # `bottom_stock` along the rough-from direction, `margin` across it.
    if rough_from not in SETUP_ORDER:
        raise PlanError(f"--rough-from must be one of {', '.join(SETUP_ORDER)}")
    rows = dict((n, r) for n, _, r in SETUPS)[rough_from]
    up = list(rows[2])  # model direction of the rough-from setup's +Z (rows are orthonormal)
    grow_lo, grow_hi = [], []
    for i in range(3):
        if up[i] > 0.5:
            grow_lo.append(bottom_stock)
            grow_hi.append(top_stock)
        elif up[i] < -0.5:
            grow_lo.append(top_stock)
            grow_hi.append(bottom_stock)
        else:
            grow_lo.append(margin)
            grow_hi.append(margin)
    stock_lo = [p - g for p, g in zip(part_lo, grow_lo)]
    stock_hi = [p + g for p, g in zip(part_hi, grow_hi)]
    frames = {name: Frame(name, r, stock_lo, stock_hi) for name, _, r in SETUPS}

    preferred = list(prefer or [])
    preferred += [rough_from] + [n for n in SETUP_ORDER if n not in preferred and n != rough_from]

    by_setup = {name: [] for name in SETUP_ORDER}
    skipped = []
    covered = []
    for feature in analysis["features"]:
        family = feature.get("family")
        if family == "holes":
            found, reason = _hole_geometry(feature, frames, preferred)
            if found is None:
                skipped.append({"feature": feature["id"], "reason": reason})
            else:
                by_setup[found[0]].append(found[1])
        elif family in TURNED:
            skipped.append({"feature": feature["id"], "reason": "turned feature: make it on a lathe"})
        elif family in NOT_PLANNED:
            if mesh is not None:
                covered.append(feature["id"])
            else:
                skipped.append(
                    {
                        "feature": feature["id"],
                        "reason": f"{NOT_PLANNED[family]}: no operation of its own; "
                        "give the mesh to rough it",
                    }
                )

    setups = []
    for name in [rough_from] + [n for n in SETUP_ORDER if n != rough_from]:
        frame = frames[name]
        holes = by_setup[name]
        ops = []
        if name == rough_from:
            part = _bounds([frame.point(p) for p in _box_corners(part_lo, part_hi)])
            part_top, part_bottom = part[1][2], part[0][2]
            if frame.stock[1][2] - part_top > 1e-6:
                ops.append({"kind": "facing", "floor_z": _r(part_top)})
            if mesh is not None:
                if mesh.get("missing"):
                    skipped.append(
                        {
                            "feature": "roughing",
                            "reason": f"faces {mesh['missing']} did not triangulate: a mesh "
                            "with holes in it would let roughing cut into the part",
                        }
                    )
                else:
                    diagonal = math.dist(part_lo, part_hi)
                    ops.append(
                        {
                            "kind": "roughing",
                            "top_z": _r(frame.stock[1][2]),
                            "floor_z": _r(min(part_bottom + skin, part_top)),
                            # specify-core meshes to 0.1% of the diagonal, then rounds to
                            # 4 decimals.
                            "surface_error_mm": _r(diagonal * 0.001 + 1e-4),
                            "positions": _mesh_for(mesh, frame),
                            "indices": list(mesh["indices"]),
                        }
                    )
        if not ops and not holes:
            continue
        ops += _group(
            holes,
            lambda h: (
                ("diameter", h["diameter"]),
                ("top_z", h["top_z"]),
                ("bore_top_z", h["bore_top_z"]),
                ("bottom_z", h["bottom_z"]),
                ("bottom", h["bottom"]),
            ),
            "holes",
        )
        enlargements = [
            dict(e, center=h["center"], feature=h["feature"]) for h in holes for e in h["enlargements"]
        ]
        ops += _group(
            enlargements,
            lambda e: (("diameter", e["diameter"]), ("top_z", e["top_z"]), ("floor_z", e["floor_z"])),
            "counterbore",
        )
        for h in holes:
            if "csink" in h:
                skipped.append(
                    {"feature": h["feature"], "reason": "countersink: not planned (chamfer it by hand)"}
                )
        setups.append({"id": name, **frame.describe(), "operations": ops})

    return {
        "format": PLAN_FORMAT,
        "version": PLAN_VERSION,
        "units": "mm",
        "source": analysis.get("binding"),
        "part": {"min": _rp(part_lo), "max": _rp(part_hi)},
        "setups": setups,
        "covered_by_roughing": covered,
        "skipped": skipped,
    }


# -------------------------------------------------------------------------------------------
# post: cam plan + tool library -> camlib job -> G-code
# -------------------------------------------------------------------------------------------


class PostError(ValueError):
    """The plan cannot be posted with these tools (no tool fits, bad tool library)."""


def _tool_assembly(camlib, spec):
    t = camlib["tools"]
    kind = spec.get("type", "end_mill")
    dims = dict(
        diameter=spec["diameter"],
        flute_length=spec["flute_length"],
        overall_length=spec["overall_length"],
    )
    if "shank_diameter" in spec:
        dims["shank_diameter"] = spec["shank_diameter"]
    if kind == "end_mill":
        cutter = t.EndMill(spec["id"], **dims)
    elif kind == "drill":
        cutter = t.Drill(spec["id"], point_angle=spec.get("point_angle", 118), **dims)
    else:
        raise PostError(f"tool {spec['id']}: type must be end_mill or drill, not {kind!r}")
    stickout = spec["stickout"]
    holder_length = spec.get("holder_length", 40)
    gauge = stickout + holder_length
    radius = spec.get("holder_diameter", 2 * spec["diameter"]) / 2
    return t.ToolAssembly(
        spec["id"], cutter, stickout_mm=stickout, gauge_length_mm=gauge,
        holders=(t.HolderSection(z_min_mm=stickout, z_max_mm=gauge, radius_min_mm=radius,
                                 radius_max_mm=radius),),
    )


def _cutting(spec, *, plunge=True):
    out = dict(
        feed_mm_min=spec["feed_mm_min"],
        spindle_rpm=spec["spindle_rpm"],
        coolant=spec.get("coolant", "off"),
        tool_number=spec["number"],
        length_register=spec.get("length_register", spec["number"]),
    )
    if plunge:
        out["plunge_mm_min"] = spec.get("plunge_mm_min", spec["feed_mm_min"] / 3)
    return out


class Library:
    """The tool library: which tool cuts what."""

    def __init__(self, data):
        self.data = data
        self.tools = data.get("tools") or []
        if not self.tools:
            raise PostError("the tool library has no tools")
        ids = [t["id"] for t in self.tools]
        if len(set(ids)) != len(ids):
            raise PostError("tool ids must be unique")
        self.drill_tolerance = data.get("drill_tolerance_mm", 0.05)

    def end_mills(self):
        return sorted((t for t in self.tools if t.get("type", "end_mill") == "end_mill"),
                      key=lambda t: t["diameter"])

    def clearing(self, name):
        wanted = self.data.get(f"{name}_tool")
        if wanted:
            return self.by_id(wanted)
        mills = self.end_mills()
        return mills[-1] if mills else None

    def by_id(self, tool_id):
        for t in self.tools:
            if t["id"] == tool_id:
                return t
        raise PostError(f"no tool {tool_id!r} in the library")

    def drill(self, diameter):
        fits = [t for t in self.tools if t.get("type") == "drill"
                and abs(t["diameter"] - diameter) <= self.drill_tolerance]
        return min(fits, key=lambda t: abs(t["diameter"] - diameter)) if fits else None

    def bore_mill(self, diameter):
        """The largest end mill that fits inside the diameter with room to helix."""
        fits = [t for t in self.end_mills() if t["diameter"] < diameter - 0.05]
        return fits[-1] if fits else None


def _drill_tip_length(spec):
    return spec["diameter"] / 2 / math.tan(math.radians(spec.get("point_angle", 118)) / 2)


def operations_for(setup, library, camlib, *, clearance, retract):
    """camlib operations for one plan setup, and notes on what was left out and why."""
    ops_mod = camlib["operations"]
    surface = camlib["surface"]
    ops, tools, notes = [], {}, []
    breakthrough = library.data.get("breakthrough_mm", 1.0)

    def use(spec):
        if spec["id"] not in tools:
            tools[spec["id"]] = _tool_assembly(camlib, spec)
        return tools[spec["id"]]

    for n, op in enumerate(setup["operations"]):
        kind = op["kind"]
        op_id = f"{setup['id']}-{n + 1}-{kind}"
        if kind == "facing":
            spec = library.clearing("facing")
            if spec is None:
                notes.append(f"{op_id}: no end mill to face with")
                continue
            ops.append(ops_mod.Facing(
                op_id, use(spec), floor_z_mm=op["floor_z"], stepdown_mm=spec["stepdown_mm"],
                stepover_mm=min(spec["stepover_mm"], spec["diameter"] / 2),
                clearance_z_mm=clearance, **_cutting(spec)))
        elif kind == "roughing":
            spec = library.clearing("roughing")
            if spec is None:
                notes.append(f"{op_id}: no end mill to rough with")
                continue
            p, idx = op["positions"], op["indices"]
            pts = [tuple(p[3 * i: 3 * i + 3]) for i in range(len(p) // 3)]
            triangles = [(pts[idx[i]], pts[idx[i + 1]], pts[idx[i + 2]]) for i in range(0, len(idx), 3)]
            error = op["surface_error_mm"]
            radial = max(library.data.get("radial_allowance_mm", 0.3), 2 * error)
            bottom = max(library.data.get("bottom_allowance_mm", 0.2), 2 * error)
            ops.append(ops_mod.Roughing(
                op_id, use(spec), surface.SurfaceMesh(triangles, surface_error_mm=error,
                                                      source={"plan": "specify-cam-plan"}),
                top_z_mm=op["top_z"], floor_z_mm=op["floor_z"], stepdown_mm=spec["stepdown_mm"],
                clearance_z_mm=clearance,
                clearing=ops_mod.RasterClearing(min(spec["stepover_mm"], spec["diameter"] / 2),
                                                routing="economical"),
                radial_allowance_mm=radial, bottom_allowance_mm=bottom, omit_empty_cuts=True,
                **_cutting(spec)))
        elif kind == "holes":
            d = op["diameter"]
            through = op["bottom"] == "through"
            drill = library.drill(d)
            if drill is not None:
                if through:
                    depth = op["bottom_z"] - breakthrough
                    allow = breakthrough + _drill_tip_length(drill) + 0.01
                    ref = "full_diameter"
                else:
                    # Never deeper than the model: the tip stops at the hole's bottom.
                    depth, allow, ref = op["bottom_z"], 0, "tip"
                    if op["bottom"] == "flat":
                        notes.append(f"{op_id}: ⌀{d} has a flat bottom; the drill leaves a point")
                extra = {}
                if drill.get("peck_mm"):
                    extra["peck_mm"] = drill["peck_mm"]
                ops.append(ops_mod.Drilling(
                    op_id, use(drill), op["centers"], depth_z_mm=depth, clearance_z_mm=clearance,
                    depth_reference=ref, retract_z_mm=retract, allow_breakthrough_mm=allow,
                    retract_feed_mm_min=drill.get("retract_feed_mm_min", drill["feed_mm_min"]),
                    **_cutting(drill, plunge=False), **extra))
                continue
            mill = library.bore_mill(d)
            if mill is None:
                notes.append(f"{op_id}: no drill of ⌀{d} and no end mill small enough to bore it "
                             f"({', '.join(op['features'])})")
                continue
            ops.append(_bore(ops_mod, op_id, use(mill), mill, op["centers"], d, op["bottom_z"],
                             clearance, retract))
        elif kind == "counterbore":
            d = op["diameter"]
            mill = library.bore_mill(d)
            if mill is None:
                notes.append(f"{op_id}: no end mill small enough for the ⌀{d} counterbore")
                continue
            ops.append(_bore(ops_mod, op_id, use(mill), mill, op["centers"], d, op["floor_z"],
                             clearance, retract))
        else:
            notes.append(f"{op_id}: unknown operation kind {kind!r}")
    return ops, list(tools.values()), notes


def _bore(ops_mod, op_id, tool, spec, centers, diameter, floor, clearance, retract):
    cutting = _cutting(spec, plunge=False)
    return ops_mod.Bore(
        op_id, tool, centers, diameter_mm=diameter, floor_z_mm=floor,
        stepdown_mm=spec["stepdown_mm"], clearance_z_mm=clearance,
        radial_stepover_mm=min(spec["stepover_mm"], spec["diameter"] / 2),
        entry_feed_mm_min=spec.get("plunge_mm_min", spec["feed_mm_min"] / 3),
        retract_feed_mm_min=spec.get("retract_feed_mm_min", spec["feed_mm_min"]),
        retract_z_mm=retract, feed_z_mm=retract, **cutting)


def _import_camlib():
    """stevecad/camlib. It is not on PyPI: the `camlib` there is an unrelated web library,
    which is told apart here by its missing `camlib.jobs`."""
    try:
        import camlib.jobs as jobs
        import camlib.operations as operations
        import camlib.post as post
        import camlib.simulation as simulation
        import camlib.surface as surface
        import camlib.tools as tools
        import camlib.verification as verification
    except ImportError as e:
        raise PostError(
            f"posting needs stevecad/camlib ({e}). It is not on PyPI (`pip install camlib` "
            "gets an unrelated package): install it from GitHub, which builds its Rust engine:\n"
            "    python -m pip install git+https://github.com/stevecad/camlib"
        ) from e
    return dict(jobs=jobs, operations=operations, post=post, simulation=simulation,
                surface=surface, tools=tools, verification=verification)


def post(plan_data, library_data, out_dir, *, stem="part", setups=None, accept=(), log=print):
    """Posts each setup of a plan to `out_dir/<stem>-<setup>.nc`; returns the paths."""
    if plan_data.get("format") != PLAN_FORMAT:
        raise PostError("not a step2cam plan (run `step2cam plan` first)")
    camlib = _import_camlib()
    jobs, post_mod = camlib["jobs"], camlib["post"]
    library = Library(library_data)
    machine_spec = dict(library_data.get("machine") or {})
    if not machine_spec:
        raise PostError("the tool library needs a `machine` (camlib assumes nothing about it)")
    dialect = machine_spec.pop("dialect", "linuxcnc")
    if dialect != "linuxcnc":
        raise PostError("only the linuxcnc dialect is wired up here; see camlib's examples for others")
    machine_name = machine_spec.pop("name", "machine")
    clearance_above = library_data.get("clearance_mm", 10.0)
    retract_above = library_data.get("retract_mm", 2.0)
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    written = []
    for setup in plan_data["setups"]:
        if setups and setup["id"] not in setups:
            continue
        top = setup["stock"]["max"][2]
        clearance = top + clearance_above
        retract = min(top + retract_above, clearance)
        ops, tools, notes = operations_for(setup, library, camlib, clearance=clearance, retract=retract)
        for note in notes:
            log(f"note: {note}")
        if not ops:
            log(f"setup {setup['id']}: nothing to post")
            continue
        lo, hi = setup["stock"]["min"], setup["stock"]["max"]
        cell = library_data.get("cell_size_mm") or max(0.1, round(max(hi[0] - lo[0], hi[1] - lo[1]) / 400, 2))
        verification = camlib["verification"]
        job = jobs.Job(
            f"step2cam-{setup['id']}", "1",
            (jobs.Setup(
                setup["id"],
                stock=camlib["simulation"].StockSpec(min_mm=tuple(lo), max_mm=tuple(hi), cell_size_mm=cell),
                tools=tuple(tools), operations=tuple(ops),
                initial_position_mm=(lo[0], lo[1], clearance),
                verification=verification.StreamChecks(stock_contact=verification.StockContactOptions()),
            ),),
        )
        header = job.compile().setup(setup["id"]).program_header()
        numbers = {t["id"]: t for t in library.tools}
        machine = post_mod.Machine(
            machine_name,
            tuple(
                post_mod.ToolSetting.for_tool(
                    tool, number=numbers[tool.id]["number"],
                    length_register=numbers[tool.id].get("length_register", numbers[tool.id]["number"]),
                    position_after_mm=header.initial_position_mm)
                for tool in header.tools
            ),
            dialect=post_mod.GCodeDialect.linuxcnc(),
            **machine_spec,
        )
        path = out_dir / f"{stem}-{setup['id']}.nc"
        program = job.post_nc(setup["id"], machine=machine, path=str(path), accept=accept,
                              lowering=post_mod.LoweringOptions(native_arcs=True))
        log(f"setup {setup['id']}: {path} ({program.lines} lines, {program.motions} motions; "
            f"unchecked: {program.unchecked})")
        written.append(path)
    return written


# -------------------------------------------------------------------------------------------
# step: STEP -> specify-core (Python library) -> plan
# -------------------------------------------------------------------------------------------


def _specify_core():
    try:
        from specify_core import api
    except ImportError as e:
        raise PlanError(
            f"reading STEP needs specify-core: `python -m pip install specify-core` ({e})"
        ) from e
    return api


def plan_step(step, part=None, *, mesh=True, **options):
    """The cam plan for a STEP file: specify-core analyses (and meshes) it, then `plan`.
    `part` picks one part of an assembly; `options` are `plan`'s."""
    api = _specify_core()
    try:
        loaded = api.load(str(step), part=part)
        analysis = api.analyse(loaded)
        meshed = api.mesh(loaded) if mesh else None
    except PlanError:
        raise
    except Exception as e:  # specify-core's own errors: unreadable file, choose a part, ...
        raise PlanError(f"specify-core: {e}") from e
    return plan(analysis, meshed, **options)


# -------------------------------------------------------------------------------------------
# Command line
# -------------------------------------------------------------------------------------------


def _read_json(name):
    if name == "-":
        text = sys.stdin.read()
    else:
        text = Path(name).read_text(encoding="utf-8")
    try:
        return json.loads(text)
    except json.JSONDecodeError as e:
        raise PlanError(f"{name}: not JSON ({e})") from e


def _write_json(value, output):
    text = json.dumps(value, indent=1) + "\n"
    if output in (None, "-"):
        sys.stdout.write(text)
    else:
        Path(output).write_text(text, encoding="utf-8")


def _add_plan_options(p):
    p.add_argument("--margin", type=float, default=2.0, help="stock around the part's sides, mm (2)")
    p.add_argument("--top-stock", type=float, default=1.0, help="stock above the part, mm (1)")
    p.add_argument("--bottom-stock", type=float, default=0.0, help="stock below the part, mm (0)")
    p.add_argument("--skin", type=float, default=0.5,
                   help="roughing stops this far above the part's bottom, so the part stays held (0.5)")
    p.add_argument("--rough-from", default="top", choices=SETUP_ORDER,
                   help="the setup that faces and roughs; holes prefer it (top)")
    p.add_argument("-o", "--output", default="-", help="plan JSON (- for stdout)")


def _plan_kwargs(args):
    return dict(margin=args.margin, top_stock=args.top_stock, bottom_stock=args.bottom_stock,
                skin=args.skin, rough_from=args.rough_from)


def _summary(p):
    lines = []
    for s in p["setups"]:
        kinds = [o["kind"] + (f" ⌀{o['diameter']}×{len(o['centers'])}" if "centers" in o else "")
                 for o in s["operations"]]
        lines.append(f"{s['id']}: {', '.join(kinds)}")
    for s in p["skipped"]:
        lines.append(f"skipped {s['feature']}: {s['reason']}")
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(prog="step2cam", description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("plan", help="analysis JSON [+ mesh JSON] -> cam plan JSON")
    p.add_argument("analysis", help="specify-core analysis JSON (- for stdin)")
    p.add_argument("--mesh", help="specify-core mesh JSON of the same part, for 3D roughing")
    _add_plan_options(p)

    s = sub.add_parser("step", help="STEP -> cam plan JSON (needs specify-core)")
    s.add_argument("step", help="STEP file")
    s.add_argument("--part", type=int, help="which part of an assembly")
    s.add_argument("--no-mesh", action="store_true", help="no 3D roughing")
    _add_plan_options(s)

    q = sub.add_parser("post", help="cam plan JSON + tool library -> G-code (needs camlib)")
    q.add_argument("plan", help="plan JSON (- for stdin)")
    q.add_argument("--tools", required=True, help="tool library JSON (see examples/tools.example.json)")
    q.add_argument("-o", "--output", default=".", help="directory for the .nc files (.)")
    q.add_argument("--stem", default="part", help="file name stem (part)")
    q.add_argument("--setup", action="append", help="post only this setup (repeatable)")
    q.add_argument("--accept", action="append", default=[], choices=["uncertain", "violation"],
                   help="release a program despite these check findings (look first)")

    args = parser.parse_args(argv)
    log = lambda text: print(text, file=sys.stderr)  # noqa: E731
    try:
        if args.command == "plan":
            analysis = _read_json(args.analysis)
            mesh = _read_json(args.mesh) if args.mesh else None
            result = plan(analysis, mesh, **_plan_kwargs(args))
            log(_summary(result))
            _write_json(result, args.output)
        elif args.command == "step":
            result = plan_step(args.step, args.part, mesh=not args.no_mesh, **_plan_kwargs(args))
            log(_summary(result))
            _write_json(result, args.output)
        else:
            plan_data = _read_json(args.plan)
            library = _read_json(args.tools)
            post(plan_data, library, args.output, stem=args.stem, setups=args.setup,
                 accept=set(args.accept), log=log)
    except (PlanError, PostError, KeyError) as e:
        log(f"error: {e}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
