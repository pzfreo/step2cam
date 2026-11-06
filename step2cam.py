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

Coordinates: each setup is turned so its tool comes down -Z, then moved so X0 Y0 is the minimum
corner and Z0 the top (every cutting Z is negative) of the stock, in the roughing setup, or of
the part's own box, in later setups (roughing has cut the stock away by then). The plan records
which, the rotation and the model point that became the origin, so the setup can be found on
the machine.
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

# Families made on a lathe, not by this bridge.
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

# Mesh points this close to a level count as on it (mm): the mesh is rounded to 1e-4 and flat
# faces are exact, so this only has to absorb rounding.
LEVEL_EPS = 0.01
# Kept between a drill tip and material found beyond a through hole's exit (mm).
EXIT_MARGIN = 0.2


class Frame:
    """One setup's coordinates: model point -> rotate -> translate to the setup's zero.

    `stock` is the box the setup starts from, in model coordinates, and the zero is its minimum
    X and Y corner and its top face: the raw stock for the roughing setup, the part's own box
    for every later one (the part has been cut free of its stock by then)."""

    def __init__(self, name, rows, box_lo, box_hi, zeroed_on):
        self.name = name
        self.rows = rows
        self.zeroed_on = zeroed_on
        lo, hi = _bounds([_rotate(rows, p) for p in _box_corners(box_lo, box_hi)])
        self.shift = [-lo[0], -lo[1], -hi[2]]
        self.stock = _bounds([self.point(p) for p in _box_corners(box_lo, box_hi)])
        # The model point at this setup's zero: rotation rows are orthonormal, so the inverse
        # is the transpose.
        back = [-s for s in self.shift]
        self.origin_model = [sum(rows[i][j] * back[i] for i in range(3)) for j in range(3)]
        self._triangles = None

    def point(self, p):
        return _add(_rotate(self.rows, p), self.shift)

    def describe(self):
        return {
            "zero": self.zeroed_on,
            "rotation": [list(row) for row in self.rows],
            "origin_model": _rp(self.origin_model),
            "stock": {"min": _rp(self.stock[0]), "max": _rp(self.stock[1])},
        }

    def triangles(self, mesh_triangles):
        """The mesh's triangles in this frame (made once)."""
        if self._triangles is None:
            self._triangles = [tuple(self.point(p) for p in t) for t in mesh_triangles]
        return self._triangles


def _mesh_triangles(mesh):
    p, idx = mesh["positions"], mesh["indices"]
    pts = [p[3 * i : 3 * i + 3] for i in range(len(p) // 3)]
    return [(pts[idx[i]], pts[idx[i + 1]], pts[idx[i + 2]]) for i in range(0, len(idx), 3)]


def _disk_meets_triangle(c, r, tri):
    """Whether the XY projection of a triangle comes within `r` of the point `c`."""
    (ax, ay), (bx, by), (cx, cy) = [(p[0], p[1]) for p in tri]
    px, py = c

    def side(x1, y1, x2, y2):
        return (x2 - x1) * (py - y1) - (y2 - y1) * (px - x1)

    s1, s2, s3 = side(ax, ay, bx, by), side(bx, by, cx, cy), side(cx, cy, ax, ay)
    # An edge-on triangle projects to a line: nothing is inside it, and its edges are measured.
    edge_on = abs((bx - ax) * (cy - ay) - (by - ay) * (cx - ax)) < 1e-9
    if not edge_on and ((s1 >= 0 and s2 >= 0 and s3 >= 0) or (s1 <= 0 and s2 <= 0 and s3 <= 0)):
        return True
    for (x1, y1), (x2, y2) in (((ax, ay), (bx, by)), ((bx, by), (cx, cy)), ((cx, cy), (ax, ay))):
        dx, dy = x2 - x1, y2 - y1
        length = dx * dx + dy * dy
        t = 0.0 if not length else max(0.0, min(1.0, ((px - x1) * dx + (py - y1) * dy) / length))
        if math.hypot(px - (x1 + t * dx), py - (y1 + t * dy)) < r:
            return True
    return False


def _blocked_above(triangles, centre, z, radius):
    """Whether any part material lies above `z` within `radius` of `centre` (setup frame):
    a tool coming down to `z` there would cut it."""
    r = radius - LEVEL_EPS
    for tri in triangles:
        if max(p[2] for p in tri) > z + LEVEL_EPS and _disk_meets_triangle(centre, r, tri):
            return True
    return False


def _clear_below(triangles, centre, z, radius):
    """How far below `z` the column of `radius` at `centre` is free of part material (setup
    frame), or None when nothing of the part is below it."""
    r = radius - LEVEL_EPS
    found = None
    for tri in triangles:
        if min(p[2] for p in tri) < z - LEVEL_EPS and _disk_meets_triangle(centre, r, tri):
            gap = max(0.0, z - max(p[2] for p in tri))
            found = gap if found is None else min(found, gap)
    return found


# camlib refuses a roughing mesh whose stated error is larger than this (mm).
CAMLIB_MAX_SURFACE_ERROR = 0.1
# specify-core rounds mesh coordinates to 4 decimals: up to this much error per point (mm).
ROUNDING_ERROR = 1e-4 * math.sqrt(3)


def _point_to_triangle_2d(c, tri):
    """Distance in a plane from the point `c` to the triangle `tri` (0 inside it)."""
    if _disk_meets_triangle(c, 0.0, tri):
        (ax, ay), (bx, by), (cx, cy) = [(p[0], p[1]) for p in tri]
        px, py = c

        def side(x1, y1, x2, y2):
            return (x2 - x1) * (py - y1) - (y2 - y1) * (px - x1)

        s1, s2, s3 = side(ax, ay, bx, by), side(bx, by, cx, cy), side(cx, cy, ax, ay)
        if (s1 >= 0 and s2 >= 0 and s3 >= 0) or (s1 <= 0 and s2 <= 0 and s3 <= 0):
            return 0.0
    best = math.inf
    for a, b in ((tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])):
        dx, dy = b[0] - a[0], b[1] - a[1]
        length = dx * dx + dy * dy
        t = 0.0 if not length else max(0.0, min(1.0, ((c[0] - a[0]) * dx + (c[1] - a[1]) * dy) / length))
        best = min(best, math.hypot(c[0] - (a[0] + t * dx), c[1] - (a[1] + t * dy)))
    return best


def _cylinder_error(triangles, face):
    """How far a cylinder face's flat triangles stray from the cylinder: its radius less the
    closest any triangle comes to the axis. Exact for triangles whose corners are on the
    surface; the triangles lie inside the cylinder's convex section, so this is the most."""
    axis = _unit(face["direction"])
    # Two directions across the axis, to project onto the cylinder's cross-section.
    helper = [1.0, 0.0, 0.0] if abs(axis[0]) < 0.9 else [0.0, 1.0, 0.0]
    u = _unit([axis[1] * helper[2] - axis[2] * helper[1], axis[2] * helper[0] - axis[0] * helper[2],
               axis[0] * helper[1] - axis[1] * helper[0]])
    v = [axis[1] * u[2] - axis[2] * u[1], axis[2] * u[0] - axis[0] * u[2], axis[0] * u[1] - axis[1] * u[0]]
    origin = face["origin"]
    worst = 0.0
    for tri in triangles:
        flat = [(_dot(_add(p, _scale(origin, -1)), u), _dot(_add(p, _scale(origin, -1)), v)) for p in tri]
        worst = max(worst, face["radius"] - _point_to_triangle_2d((0.0, 0.0), flat))
    return worst


def _surface_error(mesh, faces, deflection):
    """The most the mesh strays from the part: none on flat faces, measured on cylinders, and
    specify-core's chordal deflection bound on any other curved face; plus coordinate rounding."""
    triangles = _mesh_triangles(mesh)
    worst = 0.0
    for index, (start, count) in enumerate(mesh.get("faces") or []):
        if not count:
            continue
        face = faces.get(index) or {}
        kind = face.get("kind")
        if kind == "plane":
            continue
        if kind == "cylinder" and {"direction", "origin", "radius"} <= set(face):
            worst = max(worst, _cylinder_error(triangles[start:start + count], face))
        else:
            worst = max(worst, deflection)
    if not mesh.get("faces"):
        worst = deflection
    return worst + ROUNDING_ERROR


def _hole_entries(record):
    """The ways a hole can be cut: (axis, mouth, reversed?) for its own opening and, for a
    plain through hole whose far opening is not partly capped, the other end too."""
    axis = _unit(record["axis"])
    mouth = list(record["location"])
    yield axis, mouth, False
    plain = not any(record.get(k) for k in ("cbore", "spotface", "csink", "outer_steps"))
    if record.get("bottom") == "through" and plain and not record.get("exit_obstructions"):
        yield _scale(axis, -1), _add(mouth, _scale(axis, record["depth"])), True


def _steps(record):
    """The enlargements between a hole's opening and its bore, opening first. quiddity names
    the innermost counterbore `cbore`, the others `outer_steps`, and a shallow one (usually
    the outermost) `spotface`; each is narrower than the one before it, so widest first is the
    order from the opening."""
    steps = list(record.get("outer_steps") or [])
    for key in ("cbore", "spotface"):
        if record.get(key):
            steps.append(record[key])
    return sorted(steps, key=lambda s: -s["diameter"])


# ISO 261 coarse pitches, as specify-core offers tapped sizes (its `PITCH` table); a fine pitch
# is written as specify-core writes it, `M10x1.25`.
COARSE_PITCH = {
    "M1.6": 0.35, "M2": 0.4, "M2.5": 0.45, "M3": 0.5, "M4": 0.7, "M5": 0.8, "M6": 1.0,
    "M8": 1.25, "M10": 1.5, "M12": 1.75, "M16": 2.0, "M20": 2.5, "M24": 3.0,
}


def _thread_size(spec):
    """(major diameter, pitch) of a metric thread spec (`M5`, `M10x1.25`), or None."""
    size, _, fine = spec.partition("x")
    if not size.startswith("M"):
        return None
    try:
        major = float(size[1:])
        pitch = float(fine) if fine else COARSE_PITCH.get(size)
    except ValueError:
        return None
    if pitch is None or not (major > 0 and 0 < pitch < major):
        return None
    return major, pitch


def _full_thread(depth, pitch):
    """specify-core's default minimum full thread in a blind tapped hole of drill depth
    `depth`: the depth less three pitches, rounded down to 0.5 mm, at least one pitch."""
    down = math.floor((depth - 3.0 * pitch) * 2.0) / 2.0
    return down if down > pitch else pitch


def _threads(analysis, warnings):
    """The internal threads asked for on holes, by hole feature id: from Specify's answers
    (`hole.function:<hole or pattern>` = `tapped:M5` or `tapped:M5@5.5`). Threads another tool
    wrote, which state no size, and answers that cannot be used become warnings."""
    features = {f["id"]: f for f in analysis["features"]}
    holes = [f for f in analysis["features"] if f.get("family") == "holes"]
    answers = ((analysis.get("resume") or {}).get("answers")) or {}
    found = {}
    for key, value in answers.items():
        if not key.startswith("hole.function:") or not isinstance(value, str):
            continue
        if not value.startswith("tapped:"):
            continue
        target = key.split(":", 1)[1]
        spec, _, full = value[len("tapped:"):].partition("@")
        size = _thread_size(spec)
        if size is None:
            warnings.append(f"{target}: tapped {spec} is not a metric size this bridge knows; "
                            "not threaded")
            continue
        try:
            full_depth = float(full) if full else None
        except ValueError:
            warnings.append(f"{target}: tapped {value!r} has an unreadable depth; not threaded")
            continue
        feature = features.get(target)
        if feature is None:
            warnings.append(f"{target}: tapped {spec}, but the part has no such feature now; "
                            "not threaded")
            continue
        if feature.get("family") == "holes":
            members = [feature]
        else:  # a hole pattern or set: the holes whose faces it is made of
            own = set(feature.get("faces") or [])
            members = [h for h in holes if h.get("faces") and set(h["faces"]) <= own]
        for hole in members:
            found[hole["id"]] = {"spec": spec, "major": size[0], "pitch": size[1],
                                 "full_depth": full_depth}
    for item in analysis.get("existing") or []:
        if item.get("type") != "internal thread":
            continue
        on = set(item.get("faces") or [])
        for hole in holes:
            if hole["id"] not in found and on & set(hole.get("faces") or []):
                warnings.append(f"{hole['id']}: the file marks an internal thread here but states "
                                "no size; not threaded (answer it in Specify to thread it)")
    return found


def _hole_geometry(feature, frames, preferred, mesh_triangles, part_faces, thread=None):
    """A hole as setup-frame geometry: which setup, its centre, the bore's Z range, its
    enlargements and what is beyond a through hole's exit; or why it is skipped."""
    record = feature["record"]
    options, blocked = [], []
    for axis, mouth, reversed_ in _hole_entries(record):
        found = _setup_for(axis)
        if found is None:
            continue
        name = found[0]
        steps = [] if reversed_ else _steps(record)
        widest = max([record["diameter"]] + [s["diameter"] for s in steps])
        if mesh_triangles is not None:
            frame = frames[name]
            at = frame.point(mouth)
            if _blocked_above(frame.triangles(mesh_triangles), at[:2], at[2], widest / 2):
                blocked.append(name)
                continue
        options.append((name, axis, mouth, reversed_, steps))
    if not options:
        if blocked:
            return None, (f"part material above its opening from {', '.join(blocked)}: "
                          "a three-axis tool cannot reach it")
        return None, f"oblique axis {_rp(record['axis'])}: needs a setup this bridge does not make"
    rank = {name: i for i, name in enumerate(preferred)}
    name, axis, mouth, reversed_, steps = min(options, key=lambda o: rank[o[0]])
    frame = frames[name]

    # quiddity: `location` is the axis point at the opening; `diameter`/`depth` describe the
    # narrowest bore, `depth` measured from the top of that bore to the deep end along
    # cylinders only (a drill point's cone and any shoulder between steps are not in it).
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
    bottom = frame.point(_add(bore_top, _scale(axis, record["depth"])))
    centre = frame.point(mouth)
    hole = {
        "feature": feature["id"],
        "center": [_r(centre[0]), _r(centre[1])],
        "top_z": _r(centre[2]),
        "bore_top_z": _r(frame.point(bore_top)[2]),
        "bottom_z": _r(bottom[2]),
        "diameter": _r(record["diameter"]),
        "bottom": record.get("bottom", "unknown"),
        "enlargements": enlargements,
    }
    warnings = []
    if hole["bottom"] == "through":
        # What is beyond the exit: `exit_clear_mm` is how far the drill may go past it before
        # part material, null when nothing of the part is beyond (only the stock and what
        # holds it). Without a mesh this is known only for an exit on the part's outer face.
        part_bottom = part_faces[name]
        on_face = abs(bottom[2] - part_bottom) <= LEVEL_EPS
        if mesh_triangles is not None:
            clear = _clear_below(frame.triangles(mesh_triangles), bottom[:2], bottom[2],
                                 record["diameter"] / 2)
        else:
            clear = None if on_face else 0.0
        hole["exit_clear_mm"] = None if clear is None else _r(clear)
        if not on_face:
            warnings.append(
                f"{feature['id']}: the through hole ends {_r(bottom[2] - part_bottom)} mm short of "
                f"the part's far face in the {name} setup; anything between (a counterbore or "
                "chamfer at the far end, a shoulder) is not planned"
            )
    if thread is not None:
        # The thread runs down the bore from its top: all of it for a through hole, else the
        # minimum full thread asked for (specify-core's default when none was).
        top, bottom_z = hole["bore_top_z"], hole["bottom_z"]
        length = top - bottom_z
        if hole["bottom"] != "through":
            full = thread["full_depth"] or _full_thread(record["depth"], thread["pitch"])
            if full > length + 1e-9:
                warnings.append(f"{feature['id']}: {thread['spec']} thread {full} mm deep is "
                                f"deeper than the hole ({_r(length)} mm); not threaded")
                thread = None
            else:
                bottom_z = top - full
        if thread is not None:
            hole["thread"] = {
                "spec": thread["spec"], "major": _r(thread["major"]), "pitch": thread["pitch"],
                "top_z": _r(top), "bottom_z": _r(bottom_z),
            }
    if record.get("csink") and not reversed_:
        c = record["csink"]
        hole["csink"] = {
            "major_diameter": _r(c["major_diameter"]),
            "included_angle": _r(c["included_angle"]),
        }
    if reversed_:
        hole["entered_from_exit"] = True
    return (name, hole, warnings), None


def _group(holes, key, kind):
    """Holes with the same key as one operation over their centres."""
    groups = {}
    for hole in holes:
        groups.setdefault(key(hole), []).append(hole)
    ops = []
    for k in sorted(groups, key=lambda k: [(n, -1e18 if v is None else v) for n, v in k]):
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


def _reachable_from(feature, faces, rows):
    """Whether roughing from the setup with these rotation rows can reach a feature: none of
    its flat faces points down in that setup (a floor or ceiling facing away from the tool).
    A heuristic: it reads only the feature's planar faces."""
    for index in feature.get("faces") or []:
        face = faces.get(index)
        if face and face.get("kind") == "plane" and face.get("direction"):
            if _rotate(rows, face["direction"])[2] < -0.01:
                return False
    return True


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
    rows_by_name = {n: r for n, _, r in SETUPS}
    up = list(rows_by_name[rough_from][2])  # model direction of the rough-from setup's +Z
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
    frames = {
        name: Frame(name, r, stock_lo, stock_hi, "stock")
        if name == rough_from
        else Frame(name, r, part_lo, part_hi, "part")
        for name, _, r in SETUPS
    }
    # Each setup's view of the part's far (lowest) face, in its own Z.
    part_faces = {
        name: _bounds([f.point(p) for p in _box_corners(part_lo, part_hi)])[0][2]
        for name, f in frames.items()
    }
    mesh_triangles = None
    if mesh is not None and not mesh.get("missing"):
        mesh_triangles = _mesh_triangles(mesh)

    preferred = list(prefer or [])
    preferred += [rough_from] + [n for n in SETUP_ORDER if n not in preferred and n != rough_from]

    faces = {f["id"]: f for f in analysis.get("faces") or []}
    by_setup = {name: [] for name in SETUP_ORDER}
    skipped, covered, warnings = [], [], []
    threads = _threads(analysis, warnings)
    if mesh is None:
        warnings.append("no mesh: hole approaches and what lies beyond through holes were not "
                        "checked against the part")
    for feature in analysis["features"]:
        family = feature.get("family")
        if family == "holes":
            found, reason = _hole_geometry(feature, frames, preferred, mesh_triangles, part_faces,
                                           threads.get(feature["id"]))
            if found is None:
                skipped.append({"feature": feature["id"], "reason": reason})
                if feature["id"] in threads:
                    skipped.append({"feature": feature["id"],
                                    "reason": f"its {threads[feature['id']]['spec']} thread: the hole "
                                    "is not cut"})
            else:
                by_setup[found[0]].append(found[1])
                warnings.extend(found[2])
        elif family == "hole_patterns":
            continue  # its member holes are planned one by one
        elif family in TURNED:
            skipped.append({"feature": feature["id"], "reason": "turned feature: make it on a lathe"})
        elif mesh_triangles is None:
            skipped.append({"feature": feature["id"],
                            "reason": f"{family}: no operation of its own; give the mesh to rough it"})
        elif _reachable_from(feature, faces, rows_by_name[rough_from]):
            covered.append(feature["id"])
        else:
            skipped.append({"feature": feature["id"],
                            "reason": f"{family}: faces away from the {rough_from} setup, where "
                            "roughing runs; not planned"})

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
            error = None
            if mesh is not None and not mesh.get("missing"):
                # specify-core meshes curved faces to 0.1% of the part's diagonal.
                error = _surface_error(mesh, faces, math.dist(part_lo, part_hi) * 0.001)
            if mesh is not None:
                floor = min(part_bottom + skin, part_top)
                if mesh.get("missing"):
                    skipped.append(
                        {
                            "feature": "roughing",
                            "reason": f"faces {mesh['missing']} did not triangulate: a mesh "
                            "with holes in it would let roughing cut into the part",
                        }
                    )
                elif error > CAMLIB_MAX_SURFACE_ERROR:
                    skipped.append(
                        {
                            "feature": "roughing",
                            "reason": f"the mesh strays up to {_r(error)} mm from the part's "
                            f"curved faces; camlib accepts at most {CAMLIB_MAX_SURFACE_ERROR} mm",
                        }
                    )
                else:
                    ops.append(
                        {
                            "kind": "roughing",
                            "top_z": _r(frame.stock[1][2]),
                            "floor_z": _r(floor),
                            # Roughing clears the margin around the part down to here, so it
                            # leaves a web this thick that holds the part, and cuts where vise
                            # jaws on the stock's sides would be.
                            "web_mm": _r(floor - part_bottom),
                            # Rounded up, so the rounding never understates it.
                            "surface_error_mm": math.ceil(error * 10**DECIMALS) / 10**DECIMALS,
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
                ("exit_clear_mm", h.get("exit_clear_mm", None)),
            ),
            "holes",
        )
        for op in ops:
            if op["kind"] == "holes" and op["bottom"] != "through":
                del op["exit_clear_mm"]
        enlargements = [
            dict(e, center=h["center"], feature=h["feature"]) for h in holes for e in h["enlargements"]
        ]
        ops += _group(
            enlargements,
            lambda e: (("diameter", e["diameter"]), ("top_z", e["top_z"]), ("floor_z", e["floor_z"])),
            "counterbore",
        )
        threaded = [dict(h["thread"], center=h["center"], feature=h["feature"],
                         start_diameter=h["diameter"], through=h["bottom"] == "through")
                    for h in holes if "thread" in h]
        ops += _group(
            threaded,
            lambda t: (("spec", t["spec"]), ("major", t["major"]), ("pitch", t["pitch"]),
                       ("start_diameter", t["start_diameter"]), ("top_z", t["top_z"]),
                       ("bottom_z", t["bottom_z"]), ("through", t["through"])),
            "thread",
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
        "warnings": warnings,
    }


# -------------------------------------------------------------------------------------------
# post: cam plan + tool library -> camlib job -> G-code
# -------------------------------------------------------------------------------------------


class PostError(ValueError):
    """The plan cannot be posted with these tools (no tool fits, bad tool library)."""


# Every tool needs these: no defaults, as holder size and cutting data are specific to the tool,
# material and machine (a holder guessed too small would make camlib's holder checks pass
# where the real one collides).
TOOL_KEYS = ("id", "type", "number", "diameter", "overall_length", "stickout",
             "holder_length", "holder_diameter", "feed_mm_min", "spindle_rpm", "coolant")
TOOL_KEYS_BY_TYPE = {
    "end_mill": ("flute_length", "plunge_mm_min", "stepdown_mm", "stepover_mm"),
    "drill": ("flute_length", "point_angle"),
    # A single-profile thread mill: `diameter` is the tooth's, on a narrower neck.
    "thread_mill": ("tooth_height", "neck_diameter", "neck_length", "plunge_mm_min"),
}
# Thread milling passes, unless the tool says otherwise: the thread depth in two radial steps,
# then one spring pass at full depth.
RADIAL_PASSES = 2
SPRING_PASSES = 1
# camlib proves a thread mill's neck clear of the hole wall beside the groove from its stock
# grid: a cell up to its diagonal away can still count as wall. The grid is made fine enough
# for the neck's clearance, with this much in hand...
NECK_CELL_SAFETY = 1.25
# ...but no finer than a stock grid of this many cells (16 bytes each in camlib, plus a copy).
MAX_STOCK_CELLS = 16_000_000


def _tool_assembly(camlib, spec):
    t = camlib["tools"]
    dims = dict(
        diameter=spec["diameter"],
        flute_length=spec.get("flute_length"),
        overall_length=spec["overall_length"],
    )
    if "shank_diameter" in spec:
        dims["shank_diameter"] = spec["shank_diameter"]
    if spec["type"] == "thread_mill":
        dims.pop("flute_length")
        cutter = t.ThreadMill(spec["id"], tooth_height=spec["tooth_height"],
                              neck_diameter=spec["neck_diameter"], neck_length=spec["neck_length"],
                              **dims)
    elif spec["type"] == "end_mill":
        cutter = t.EndMill(spec["id"], **dims)
    else:
        cutter = t.Drill(spec["id"], point_angle=spec["point_angle"], **dims)
    stickout = spec["stickout"]
    gauge = stickout + spec["holder_length"]
    radius = spec["holder_diameter"] / 2
    return t.ToolAssembly(
        spec["id"], cutter, stickout_mm=stickout, gauge_length_mm=gauge,
        holders=(t.HolderSection(z_min_mm=stickout, z_max_mm=gauge, radius_min_mm=radius,
                                 radius_max_mm=radius),),
    )


def _cutting(spec, *, plunge=True):
    out = dict(
        feed_mm_min=spec["feed_mm_min"],
        spindle_rpm=spec["spindle_rpm"],
        coolant=spec["coolant"],
        tool_number=spec["number"],
        length_register=spec.get("length_register", spec["number"]),
    )
    if plunge:
        out["plunge_mm_min"] = spec["plunge_mm_min"]
    return out


class Library:
    """The tool library: which tool cuts what."""

    def __init__(self, data):
        self.data = data
        self.tools = data.get("tools") or []
        if not self.tools:
            raise PostError("the tool library has no tools")
        for n, tool in enumerate(self.tools):
            name = tool.get("id", f"tools[{n}]")
            if tool.get("type") not in TOOL_KEYS_BY_TYPE:
                raise PostError(f"tool {name}: type must be one of {', '.join(TOOL_KEYS_BY_TYPE)}, "
                                f"not {tool.get('type')!r}")
            missing = [k for k in TOOL_KEYS + TOOL_KEYS_BY_TYPE[tool["type"]] if k not in tool]
            if missing:
                raise PostError(f"tool {name}: missing {', '.join(missing)}")
            if tool["holder_diameter"] < tool.get("shank_diameter", tool["diameter"]):
                raise PostError(f"tool {name}: holder_diameter is smaller than the shank")
            if tool["type"] == "thread_mill" and not tool["neck_diameter"] < tool["diameter"]:
                raise PostError(f"tool {name}: a thread mill's neck must be narrower than its tooth")
        ids = [t["id"] for t in self.tools]
        if len(set(ids)) != len(ids):
            raise PostError("tool ids must be unique")
        self.drill_tolerance = data.get("drill_tolerance_mm", 0.05)
        self.breakthrough = data.get("breakthrough_mm", 1.0)

    def end_mills(self):
        return sorted((t for t in self.tools if t["type"] == "end_mill"), key=lambda t: t["diameter"])

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
        """The drill closest to the diameter and never larger, at most `drill_tolerance_mm`
        under it."""
        fits = [t for t in self.tools if t["type"] == "drill"
                and -1e-9 <= diameter - t["diameter"] <= self.drill_tolerance]
        return max(fits, key=lambda t: t["diameter"]) if fits else None

    def thread_mill(self, op, stock_top):
        """The largest thread mill that can cut this thread: its pitch range (if it states one)
        takes the pitch, it fits the drilled hole, its neck clears the crests and it reaches
        the thread's bottom. With none, the reasons the others were refused."""
        major, start, pitch = op["major"], op["start_diameter"], op["pitch"]
        fits, why = [], []
        for t in self.tools:
            if t["type"] != "thread_mill":
                continue
            r = t["diameter"] / 2
            room = start / 2 - (major / 2 - r)  # from the axis-side path to the drilled wall
            reach = t["tooth_height"] + t["neck_length"]
            if not t.get("min_pitch_mm", 0) <= pitch <= t.get("max_pitch_mm", math.inf):
                why.append(f"{t['id']}: pitch {pitch} outside its range")
            elif not t["diameter"] < start - 0.05:
                why.append(f"{t['id']}: ⌀{t['diameter']} does not fit the ⌀{start} hole")
            elif not t["neck_diameter"] / 2 < room - 0.01:
                why.append(f"{t['id']}: its ⌀{t['neck_diameter']} neck would rub the thread crests")
            elif not reach > stock_top - op["bottom_z"] + 0.01:
                why.append(f"{t['id']}: reaches {reach} mm, the thread ends "
                           f"{_r(stock_top - op['bottom_z'])} mm below the stock top")
            else:
                fits.append(t)
        return (max(fits, key=lambda t: t["diameter"]) if fits else None), why

    def bore_mill(self, diameter):
        """The largest end mill that fits inside the diameter with room to helix."""
        fits = [t for t in self.end_mills() if t["diameter"] < diameter - 0.05]
        return fits[-1] if fits else None


def _drill_tip_length(spec):
    return spec["diameter"] / 2 / math.tan(math.radians(spec["point_angle"]) / 2)


def _drill_depth(op, drill, library, stock_bottom, op_id, notes):
    """(depth_z_mm, depth_reference, allow_breakthrough_mm) for drilling one hole group."""
    d, bottom_z = op["diameter"], op["bottom_z"]
    tip = _drill_tip_length(drill)
    if op["bottom"] == "through":
        clear = op.get("exit_clear_mm")
        want = library.breakthrough
        if clear is None:
            depth, ref, tip_z = bottom_z - want, "full_diameter", bottom_z - want - tip
        else:
            room = clear - EXIT_MARGIN  # how far the tip may go past the exit
            if room >= tip:
                through = min(want, room - tip)
                depth, ref, tip_z = bottom_z - through, "full_diameter", bottom_z - through - tip
                if through < want:
                    notes.append(f"{op_id}: part material {clear} mm beyond the exit: drilled only "
                                 f"{_r(through)} mm past it at full diameter")
            else:
                tip_z = bottom_z - max(0.0, room)
                depth, ref = tip_z, "tip"
                notes.append(f"{op_id}: part material {clear} mm beyond the exit: the ⌀{d} drill "
                             "cannot reach full diameter at the exit without cutting it; finish it "
                             "another way")
        if tip_z < stock_bottom - 1e-9:
            notes.append(f"{op_id}: the drill tip goes {_r(stock_bottom - tip_z)} mm below the "
                         "stock bottom: support the part clear of the holes (spoilboard, parallels)")
            allow = stock_bottom - tip_z + 0.01
        else:
            allow = 0
        return depth, ref, allow
    if op["bottom"] == "drill_point":
        # quiddity's depth ends where the cone starts: full diameter there; the drill's own point
        # then ends where a cone of its angle would.
        notes.append(f"{op_id}: ⌀{d} drill-point holes: full diameter at the model's depth; the "
                     f"point ends {_r(tip)} mm deeper, at the {drill['point_angle']}° drill's angle "
                     "(the model's cone angle is not read)")
        return bottom_z, "full_diameter", 0
    if op["bottom"] == "flat":
        notes.append(f"{op_id}: ⌀{d} flat-bottomed holes: the drill's tip stops at the bottom, so "
                     f"full diameter ends {_r(tip)} mm short of it and a point is left")
    else:
        notes.append(f"{op_id}: ⌀{d} holes with an unknown bottom: the drill's tip stops at the "
                     "model's depth")
    return bottom_z, "tip", 0


def _bore_floor(op, library, stock_bottom, op_id, notes):
    """The Bore floor for a hole group: through holes go past the exit as far as the stock
    (camlib bores no lower) and any part material beyond allow."""
    bottom_z = op["bottom_z"]
    if op["bottom"] != "through":
        return bottom_z
    past = library.breakthrough
    if op.get("exit_clear_mm") is not None:
        past = min(past, max(0.0, op["exit_clear_mm"] - EXIT_MARGIN))
    floor = max(stock_bottom, bottom_z - past)
    if floor > bottom_z - 0.05:
        notes.append(f"{op_id}: ⌀{op['diameter']} through holes are bored to the exit and no "
                     "further (the stock or the part ends there): expect a skin or burr to clean up")
    return floor


def operations_for(setup, library, camlib, *, clearance, retract, needs=None):
    """camlib operations for one plan setup, and notes on what was left out and why. `needs`
    (a dict), when given, gets `cell_mm`: the coarsest stock grid the operations can be
    checked on, if one of them needs a finer grid than usual."""
    needs = {} if needs is None else needs
    ops_mod = camlib["operations"]
    surface = camlib["surface"]
    ops, tools, notes = [], {}, []
    stock_bottom = setup["stock"]["min"][2]
    stock_top = setup["stock"]["max"][2]
    # How deep each hole is open at its full diameter, once cut: a thread must not go lower.
    full_diameter_z = {}

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
            notes.append(f"{op_id}: clears {radial} mm from the walls and leaves a "
                         f"{op.get('web_mm', '?')} mm web around the part; it cuts the margin where "
                         "vise jaws on the stock would be, so hold the stock from below or by the web")
        elif kind == "holes":
            d = op["diameter"]
            drill = library.drill(d)
            if drill is not None:
                depth, ref, allow = _drill_depth(op, drill, library, stock_bottom, op_id, notes)
                extra = {}
                if drill.get("peck_mm"):
                    extra["peck_mm"] = drill["peck_mm"]
                open_z = depth if ref == "full_diameter" else depth + _drill_tip_length(drill)
                full_diameter_z.update(dict.fromkeys(op["features"], open_z))
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
            floor = _bore_floor(op, library, stock_bottom, op_id, notes)
            full_diameter_z.update(dict.fromkeys(op["features"], floor))
            ops.append(_bore(ops_mod, op_id, use(mill), mill, op["centers"], d, floor,
                             clearance, retract))
        elif kind == "counterbore":
            d = op["diameter"]
            mill = library.bore_mill(d)
            if mill is None:
                notes.append(f"{op_id}: no end mill small enough for the ⌀{d} counterbore")
                continue
            ops.append(_bore(ops_mod, op_id, use(mill), mill, op["centers"], d, op["floor_z"],
                             clearance, retract))
        elif kind == "thread":
            thread = f"{op['spec']} ({', '.join(op['features'])})"
            uncut = [f for f in op["features"] if f not in full_diameter_z]
            if uncut:
                notes.append(f"{op_id}: {thread}: the hole is not cut, so it is not threaded")
                continue
            open_z = max(full_diameter_z[f] for f in op["features"])
            if op["bottom_z"] < open_z - 1e-6:
                notes.append(f"{op_id}: {thread}: the thread would end {_r(open_z - op['bottom_z'])} "
                             "mm below where the hole reaches full diameter; not threaded")
                continue
            mill, why = library.thread_mill(op, stock_top)
            if mill is None:
                notes.append(f"{op_id}: {thread}: no thread mill can cut it"
                             + (f" ({'; '.join(why)})" if why else ""))
                continue
            # The neck's clearance from the drilled wall, on the thread's final path circle.
            neck_room = (op["start_diameter"] / 2 - (op["major"] / 2 - mill["diameter"] / 2)
                         - mill["neck_diameter"] / 2)
            cell = neck_room / (math.sqrt(2) * NECK_CELL_SAFETY)
            lo, hi = setup["stock"]["min"], setup["stock"]["max"]
            cells = (hi[0] - lo[0]) / cell * (hi[1] - lo[1]) / cell
            if cells > MAX_STOCK_CELLS:
                notes.append(f"{op_id}: {thread}: {mill['id']}'s neck passes {_r(neck_room)} mm from "
                             f"the hole wall; proving that needs a {_r(cell)} mm stock grid, too fine "
                             "for this part (use a thread mill with a narrower neck)")
                continue
            needs["cell_mm"] = min(needs.get("cell_mm", math.inf), cell)
            ops.append(ops_mod.ThreadMilling(
                op_id, use(mill), op["centers"], internal=True, diameter_mm=op["major"],
                start_diameter_mm=op["start_diameter"], pitch_mm=op["pitch"],
                top_z_mm=op["top_z"], bottom_z_mm=op["bottom_z"], clearance_z_mm=clearance,
                radial_passes=mill.get("radial_passes", RADIAL_PASSES),
                spring_passes=mill.get("spring_passes", SPRING_PASSES),
                retract_z_mm=retract, feed_z_mm=retract, **_cutting(mill)))
            notes.append(f"{op_id}: {thread}: thread milled to the ⌀{op['major']} major diameter "
                         f"(pitch {op['pitch']}, right hand) with {mill['id']}; check the fit "
                         "with a gauge or screw before taking the part off")
        else:
            notes.append(f"{op_id}: unknown operation kind {kind!r}")
    return ops, list(tools.values()), notes


def _bore(ops_mod, op_id, tool, spec, centers, diameter, floor, clearance, retract):
    cutting = _cutting(spec, plunge=False)
    return ops_mod.Bore(
        op_id, tool, centers, diameter_mm=diameter, floor_z_mm=floor,
        stepdown_mm=spec["stepdown_mm"], clearance_z_mm=clearance,
        radial_stepover_mm=min(spec["stepover_mm"], spec["diameter"] / 2),
        entry_feed_mm_min=spec["plunge_mm_min"],
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
    library = Library(library_data)
    machine_spec = dict(library_data.get("machine") or {})
    if not machine_spec:
        raise PostError("the tool library needs a `machine` (camlib assumes nothing about it)")
    if machine_spec.get("tool_change_position_mode") != "table":
        # Other modes need a camlib ToolChangePosition matching the machine, not wired up here.
        raise PostError('machine.tool_change_position_mode must be "table" (the only mode wired up)')
    dialect = machine_spec.pop("dialect", "linuxcnc")
    if dialect != "linuxcnc":
        raise PostError("only the linuxcnc dialect is wired up here; see camlib's examples for others")
    machine_name = machine_spec.pop("name", "machine")
    camlib = _import_camlib()
    jobs, post_mod = camlib["jobs"], camlib["post"]
    clearance_above = library_data.get("clearance_mm", 10.0)
    retract_above = library_data.get("retract_mm", 2.0)
    for warning in plan_data.get("warnings", []):
        log(f"warning: {warning}")
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    written = []
    for setup in plan_data["setups"]:
        if setups and setup["id"] not in setups:
            continue
        top = setup["stock"]["max"][2]
        clearance = top + clearance_above
        retract = min(top + retract_above, clearance)
        needs = {}
        ops, tools, notes = operations_for(setup, library, camlib, clearance=clearance,
                                           retract=retract, needs=needs)
        log(f"setup {setup['id']}: zero X0 Y0 on the {setup.get('zero', 'stock')}'s minimum corner, "
            f"Z0 on its top (model point {setup['origin_model']})")
        for note in notes:
            log(f"note: {note}")
        if not ops:
            log(f"setup {setup['id']}: nothing to post")
            continue
        lo, hi = setup["stock"]["min"], setup["stock"]["max"]
        cell = library_data.get("cell_size_mm") or max(0.1, round(max(hi[0] - lo[0], hi[1] - lo[1]) / 400, 2))
        if needs.get("cell_mm", math.inf) < cell:
            # Rounded down to a whole micrometre, so the grid is never coarser than needed.
            cell = math.floor(needs["cell_mm"] * 1000) / 1000
            log(f"setup {setup['id']}: stock grid {cell} mm, fine enough to prove the thread "
                "mill's neck clear of the hole wall")
        verification = camlib["verification"]
        # Where the machine leaves each new tool: the spindle's gauge line at `gauge_z` over
        # X Y, in this setup's work coordinates. The planner (Setup.tool_change) and the post
        # (each tool's position_after_mm in the table) are told the same, and the machine must
        # do it (an M6 that moves there).
        gauges = {t.id: t.gauge_length_mm for t in tools}
        lowest = clearance + max(gauges.values())
        gauge_z = library_data.get("tool_change_gauge_z_mm")
        if gauge_z is None:
            gauge_z = lowest
        elif gauge_z < lowest:
            raise PostError(f"tool_change_gauge_z_mm {gauge_z} puts a tool tip below the clearance "
                            f"height in setup {setup['id']}: it must be at least {_r(lowest)}")
        change_xy = tuple(library_data.get("tool_change_xy_mm") or (lo[0], lo[1]))
        tip_after = {tid: (change_xy[0], change_xy[1], gauge_z - g) for tid, g in gauges.items()}
        log(f"setup {setup['id']}: after each tool change the spindle gauge line must be at "
            f"X{_r(change_xy[0])} Y{_r(change_xy[1])} Z{_r(gauge_z)} (work coordinates)")
        first_tool = getattr(ops[0], "tool", None)
        first_id = getattr(first_tool, "id", None) or tools[0].id
        job = jobs.Job(
            f"step2cam-{setup['id']}", "1",
            (jobs.Setup(
                setup["id"],
                stock=camlib["simulation"].StockSpec(min_mm=tuple(lo), max_mm=tuple(hi), cell_size_mm=cell),
                tools=tuple(tools), operations=tuple(ops),
                initial_position_mm=tip_after[first_id],
                verification=verification.StreamChecks(stock_contact=verification.StockContactOptions()),
                tool_change=camlib["simulation"].ToolChangePosition(gauge_z_mm=gauge_z, xy_mm=change_xy),
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
                    position_after_mm=tip_after[tool.id])
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
        kinds = []
        for o in s["operations"]:
            if o["kind"] == "thread":
                kinds.append(f"thread {o['spec']}×{len(o['centers'])}")
            elif "centers" in o:
                kinds.append(f"{o['kind']} ⌀{o['diameter']}×{len(o['centers'])}")
            else:
                kinds.append(o["kind"])
        lines.append(f"{s['id']}: {', '.join(kinds)}")
    for s in p["skipped"]:
        lines.append(f"skipped {s['feature']}: {s['reason']}")
    for w in p.get("warnings", []):
        lines.append(f"warning: {w}")
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
