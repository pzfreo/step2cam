"""Tests of the step2cam bridge.

    python3 -m unittest discover -s tests

Standard library only by default: camlib is replaced by a stand-in that records what the bridge
asks of it. With camlib's Python sources on the path (CAMLIB_PYTHON=<camlib checkout>/python),
the operations are also built from camlib's real classes; with camlib installed (its engine
built), a whole plan is posted to G-code.
"""

import copy
import io
import json
import math
import os
import sys
import tempfile
import types
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(ROOT))

import step2cam  # noqa: E402

FIXTURES = HERE / "fixtures"


def fixture(name):
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def det(rows):
    (a, b, c), (d, e, f), (g, h, i) = rows
    return a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)


BINDING = {"source_sha256": "x", "face_count": 1, "loader_version": 2, "part": 0}
BOX = ((-20, -20, -10), (20, 20, 0))


def synthetic(holes, bounds=BOX, features=(), faces=()):
    """An analysis with these hole records (and other features and faces)."""
    return {
        "binding": BINDING,
        "faces": list(faces),
        "features": [
            {"id": f"holes:{n}", "family": "holes", "record": r, "faces": []}
            for n, r in enumerate(holes)
        ] + list(features),
        "dimensioning": {"bounds": [list(bounds[0]), list(bounds[1])]},
    }


def tapped(answers, analysis=None):
    """An analysis (the plate's by default) with these Specify answers stored in it."""
    a = copy.deepcopy(analysis or fixture("plate.analysis.json"))
    a["resume"] = {"answers": dict(answers)}
    return a


def thread_ops(plan, setup=0):
    return [o for o in plan["setups"][setup]["operations"] if o["kind"] == "thread"]


def mesh_of(triangles, bounds=BOX):
    """A mesh of just these triangles, for the synthetic analysis."""
    positions = [x for tri in triangles for p in tri for x in p]
    return {"format": "pmi-assist-mesh", "version": 2, "binding": BINDING,
            "bbox": [list(bounds[0]), list(bounds[1])], "positions": positions,
            "indices": list(range(3 * len(triangles))), "faces": [], "missing": [], "edges": []}


def square(z, half=5, cx=0, cy=0):
    """Two triangles: a level square at `z`."""
    a, b, c, d = ((cx - half, cy - half, z), (cx + half, cy - half, z), (cx + half, cy + half, z),
                  (cx - half, cy + half, z))
    return [(a, b, c), (a, c, d)]


def hole(axis, location, diameter, depth, bottom="through", **extra):
    record = {"axis": axis, "location": location, "diameter": diameter, "depth": depth,
              "bottom": bottom, "cbore": None, "spotface": None, "csink": None}
    record.update(extra)
    return record


class Rotations(unittest.TestCase):
    def test_every_setup_is_a_proper_rotation_taking_its_axis_down(self):
        for name, axis, rows in step2cam.SETUPS:
            with self.subTest(name):
                self.assertEqual(det(rows), 1)
                self.assertEqual(step2cam._rotate(rows, axis), [0, 0, -1])
                for i in range(3):
                    for j in range(3):
                        self.assertEqual(step2cam._dot(rows[i], rows[j]), int(i == j))


class Plan(unittest.TestCase):
    def test_plate_from_the_top(self):
        plan = step2cam.plan(fixture("plate.analysis.json"))
        self.assertEqual(plan["format"], "specify-cam-plan")
        self.assertEqual([s["id"] for s in plan["setups"]], ["top"])
        top = plan["setups"][0]
        # Part 100 x 60 x 10, 2 mm margin, 1 mm on top: Z0 is the stock top.
        self.assertEqual(top["stock"], {"min": [0, 0, -11], "max": [104, 64, 0]})
        kinds = [o["kind"] for o in top["operations"]]
        self.assertEqual(kinds, ["facing", "holes", "holes", "holes"])
        self.assertEqual(top["operations"][0]["floor_z"], -1)
        by_d = {o["diameter"]: o for o in top["operations"] if o["kind"] == "holes"}
        self.assertEqual(sorted(by_d), [4.2, 6.6, 8.0])
        self.assertEqual(len(by_d[6.6]["centers"]), 6)
        for op in by_d.values():
            self.assertEqual((op["top_z"], op["bottom_z"], op["bottom"]), (-1, -11, "through"))
        self.assertEqual(by_d[8.0]["centers"], [[52, 32]])
        self.assertEqual(plan["skipped"], [])
        self.assertEqual(top["zero"], "stock")

    def test_the_mesh_adds_roughing_in_the_setup_frame(self):
        plan = step2cam.plan(fixture("plate.analysis.json"), fixture("plate.mesh.json"))
        top = plan["setups"][0]
        # The mesh's exact box and the analysis's frame-carried box agree.
        self.assertEqual(top["stock"], step2cam.plan(fixture("plate.analysis.json"))["setups"][0]["stock"])
        rough = [o for o in top["operations"] if o["kind"] == "roughing"][0]
        self.assertEqual((rough["top_z"], rough["floor_z"]), (0, -10.5))  # 0.5 mm skin
        p = rough["positions"]
        zs, xs = p[2::3], p[0::3]
        self.assertAlmostEqual(max(zs), -1)
        self.assertAlmostEqual(min(zs), -11)
        self.assertAlmostEqual(min(xs), 2)
        self.assertEqual(len(rough["indices"]) % 3, 0)

    def test_a_mesh_of_another_part_is_refused(self):
        mesh = fixture("plate.mesh.json")
        mesh["binding"] = dict(mesh["binding"], source_sha256="0" * 64)
        with self.assertRaises(step2cam.PlanError):
            step2cam.plan(fixture("plate.analysis.json"), mesh)

    def test_a_mesh_with_untriangulated_faces_is_not_roughed(self):
        mesh = fixture("plate.mesh.json")
        mesh["missing"] = [3]
        plan = step2cam.plan(fixture("plate.analysis.json"), mesh)
        self.assertNotIn("roughing", [o["kind"] for o in plan["setups"][0]["operations"]])
        self.assertEqual(plan["skipped"][0]["feature"], "roughing")

    def test_side_holes_get_their_own_setup_inside_the_stock(self):
        plan = step2cam.plan(fixture("spindle.analysis.json"))
        ids = [s["id"] for s in plan["setups"]]
        self.assertEqual(ids[0], "top")
        self.assertGreater(len(ids), 1)
        for setup in plan["setups"]:
            lo, hi = setup["stock"]["min"], setup["stock"]["max"]
            self.assertEqual(hi[2], 0)
            for op in setup["operations"]:
                for x, y in op.get("centers", []):
                    self.assertTrue(lo[0] <= x <= hi[0] and lo[1] <= y <= hi[1], (setup["id"], x, y))
                if op["kind"] == "holes":
                    self.assertTrue(lo[2] <= op["bottom_z"] < op["top_z"] <= 0, op)
        reasons = {s["reason"] for s in plan["skipped"]}
        self.assertIn("turned feature: make it on a lathe", reasons)

    def test_a_through_hole_is_entered_from_the_preferred_side(self):
        # quiddity may present a through hole from either mouth: here from the bottom.
        a = synthetic([hole([0, 0, 1], [5, 0, -10], 6, 10)])
        plan = step2cam.plan(a, top_stock=0)
        self.assertEqual([s["id"] for s in plan["setups"]], ["top"])
        op = plan["setups"][0]["operations"][0]
        self.assertEqual((op["top_z"], op["bottom_z"]), (0, -10))
        self.assertEqual(op["centers"], [[27, 22]])

    def test_a_blind_hole_from_below_needs_the_bottom_setup_zeroed_on_the_part(self):
        a = synthetic([hole([0, 0, 1], [5, 3, -10], 5, 4, bottom="flat")])
        plan = step2cam.plan(a, top_stock=1)
        bottom = [s for s in plan["setups"] if s["id"] == "bottom"][0]
        # By the time the part is turned over, the stock around it has been roughed away: the
        # setup is zeroed on the part's own box, not on the stock's.
        self.assertEqual(bottom["zero"], "part")
        self.assertEqual(bottom["stock"], {"min": [0, 0, -10], "max": [40, 40, 0]})
        op = bottom["operations"][0]
        self.assertEqual((op["top_z"], op["bottom_z"], op["bottom"]), (0, -4, "flat"))
        # Turned over about X: model y=3 lands at y = 20 - 3.
        self.assertEqual(op["centers"], [[25, 17]])
        # The origin recorded for the bottom setup maps back to the model point.
        self.assertEqual(bottom["origin_model"], [-20, 20, -10])

    def test_counterbores_become_their_own_operation_above_the_bore(self):
        a = synthetic([
            hole([0, 0, -1], [0, 0, 0], 6.6, 6.5, cbore={"diameter": 11, "depth": 3.5}),
            hole([0, 0, -1], [10, 0, 0], 6.6, 6.5, cbore={"diameter": 11, "depth": 3.5}),
        ])
        plan = step2cam.plan(a, top_stock=0)
        ops = plan["setups"][0]["operations"]
        holes = [o for o in ops if o["kind"] == "holes"]
        cbores = [o for o in ops if o["kind"] == "counterbore"]
        self.assertEqual(len(holes), 1)
        self.assertEqual((holes[0]["bore_top_z"], holes[0]["bottom_z"]), (-3.5, -10))
        self.assertEqual(len(cbores), 1)
        self.assertEqual((cbores[0]["diameter"], cbores[0]["top_z"], cbores[0]["floor_z"]), (11, 0, -3.5))
        self.assertEqual(len(cbores[0]["centers"]), 2)

    def test_a_spotface_is_the_outermost_step_above_a_counterbore(self):
        # quiddity: steps narrow inwards; the shallow spotface is usually the outermost.
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 6.6, 3.5, cbore={"diameter": 11, "depth": 6},
                            spotface={"diameter": 20, "depth": 0.5})])
        ops = step2cam.plan(a, top_stock=0)["setups"][0]["operations"]
        cbores = sorted(((o["diameter"], o["top_z"], o["floor_z"]) for o in ops
                         if o["kind"] == "counterbore"), reverse=True)
        self.assertEqual(cbores, [(20, 0, -0.5), (11, -0.5, -6.5)])
        holes = [o for o in ops if o["kind"] == "holes"][0]
        self.assertEqual((holes["bore_top_z"], holes["bottom_z"]), (-6.5, -10))

    def test_outer_steps_come_before_the_counterbore(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 5, 5, cbore={"diameter": 9, "depth": 3},
                            outer_steps=[{"diameter": 16, "depth": 2}])])
        ops = step2cam.plan(a, top_stock=0)["setups"][0]["operations"]
        cbores = sorted(((o["diameter"], o["top_z"], o["floor_z"]) for o in ops
                         if o["kind"] == "counterbore"), reverse=True)
        self.assertEqual(cbores, [(16, 0, -2), (9, -2, -5)])

    def test_a_through_hole_with_a_capped_exit_is_not_entered_from_it(self):
        a = synthetic([hole([0, 0, 1], [5, 0, -10], 6, 10, exit_obstructions=[
            {"location": [5, 0, 0], "faces": [3], "covered_fraction": 0.4}])])
        setups = step2cam.plan(a, top_stock=0)["setups"]
        self.assertEqual([s["id"] for s in setups], ["bottom"])
        self.assertNotIn("entered_from_exit", json.dumps(setups))

    def test_material_above_the_opening_sends_the_hole_to_its_other_end(self):
        # A ledge 5 mm above the top opening of a plain through hole.
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 6, 10)], bounds=((-20, -20, -10), (20, 20, 5)))
        mesh = mesh_of(square(5), bounds=((-20, -20, -10), (20, 20, 5)))
        plan = step2cam.plan(a, mesh, top_stock=0)
        bottom = [s for s in plan["setups"] if s["id"] == "bottom"]
        self.assertEqual(len(bottom), 1)
        self.assertEqual(bottom[0]["operations"][0]["features"], ["holes:0"])
        self.assertNotIn("holes", [o["kind"] for o in plan["setups"][0]["operations"]])

    def test_a_blind_hole_under_material_is_skipped(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 6, 4, bottom="flat")],
                      bounds=((-20, -20, -10), (20, 20, 5)))
        plan = step2cam.plan(a, mesh_of(square(5), bounds=((-20, -20, -10), (20, 20, 5))))
        reason = plan["skipped"][0]["reason"]
        self.assertTrue(reason.startswith("part material above its opening from top"), reason)

    def test_material_beyond_a_through_exit_is_measured(self):
        # A through hole that breaks into a cavity, with its far wall 2 mm below the exit.
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 6.6, 6)])
        plan = step2cam.plan(a, mesh_of(square(-8)), top_stock=0)
        op = [o for o in plan["setups"][0]["operations"] if o["kind"] == "holes"][0]
        self.assertEqual((op["bottom_z"], op["exit_clear_mm"]), (-6, 2))
        self.assertTrue(any("ends 4.0 mm short of the part's far face" in w for w in plan["warnings"]))

    def test_an_exit_on_the_parts_face_has_nothing_beyond_it(self):
        plan = step2cam.plan(fixture("plate.analysis.json"), fixture("plate.mesh.json"))
        for op in plan["setups"][0]["operations"]:
            if op["kind"] == "holes":
                self.assertIsNone(op["exit_clear_mm"])
        self.assertEqual(plan["warnings"], [])

    def test_without_a_mesh_an_inner_exit_counts_as_blocked(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 6.6, 6)])
        plan = step2cam.plan(a, top_stock=0)
        op = plan["setups"][0]["operations"][0]
        self.assertEqual(op["exit_clear_mm"], 0)
        self.assertTrue(plan["warnings"][0].startswith("no mesh"))

    def test_roughing_covers_only_features_it_can_reach(self):
        faces = [{"id": 0, "kind": "plane", "direction": [0, 0, 1]},
                 {"id": 1, "kind": "plane", "direction": [0, 0, -1]},
                 {"id": 2, "kind": "plane", "direction": [1, 0, 0]}]
        features = [
            {"id": "pockets:0", "family": "pockets", "record": {}, "faces": [0, 2]},  # opens up
            {"id": "pockets:1", "family": "pockets", "record": {}, "faces": [1, 2]},  # opens down
            {"id": "fillets:3", "family": "fillets", "record": {}, "faces": [2]},
            {"id": "hole_patterns:4", "family": "hole_patterns", "record": {}, "faces": []},
        ]
        a = synthetic([], features=features, faces=faces)
        plan = step2cam.plan(a, mesh_of(square(-10, half=20)))
        self.assertEqual(plan["covered_by_roughing"], ["pockets:0", "fillets:3"])
        self.assertEqual([s["feature"] for s in plan["skipped"]], ["pockets:1"])
        # Without the mesh nothing is roughed, and every family is listed.
        plan = step2cam.plan(a)
        self.assertEqual([s["feature"] for s in plan["skipped"]],
                         ["pockets:0", "pockets:1", "fillets:3"])

    def test_the_mesh_error_is_measured_on_flats_and_cylinders(self):
        # The plate is flat faces and holes: its triangles stray only by the holes' chords,
        # far less than specify-core's 0.1%-of-diagonal bound (0.117 mm here), which camlib
        # (at most 0.1 mm) would refuse.
        plan = step2cam.plan(fixture("plate.analysis.json"), fixture("plate.mesh.json"))
        rough = [o for o in plan["setups"][0]["operations"] if o["kind"] == "roughing"][0]
        self.assertTrue(0.01 < rough["surface_error_mm"] < 0.02, rough["surface_error_mm"])
        # Checked by sampling each cylinder triangle against its cylinder.
        analysis, mesh = fixture("plate.analysis.json"), fixture("plate.mesh.json")
        faces = {f["id"]: f for f in analysis["faces"]}
        tris = step2cam._mesh_triangles(mesh)
        sampled = 0.0
        for index, (start, count) in enumerate(mesh["faces"]):
            face = faces[index]
            if face["kind"] != "cylinder":
                continue
            axis, origin = step2cam._unit(face["direction"]), face["origin"]
            for t in tris[start:start + count]:
                for i in range(6):
                    for j in range(6 - i):
                        w = (i / 5, j / 5, 1 - i / 5 - j / 5)
                        q = [sum(w[k] * t[k][n] for k in range(3)) - origin[n] for n in range(3)]
                        along = step2cam._dot(q, axis)
                        radial = math.sqrt(max(0.0, step2cam._dot(q, q) - along * along))
                        sampled = max(sampled, abs(face["radius"] - radial))
        self.assertLessEqual(sampled, rough["surface_error_mm"])

    def test_a_mesh_too_coarse_for_camlib_is_not_roughed(self):
        # A curved face of a kind not measured here takes specify-core's bound: 0.1% of a
        # 200 mm diagonal is over camlib's 0.1 mm.
        bounds = ((0, 0, -10), (150, 130, 0))
        faces = [{"id": 0, "kind": "bspline"}]
        a = synthetic([], bounds=bounds, faces=faces)
        mesh = mesh_of(square(-5, half=10, cx=50, cy=50), bounds=bounds)
        mesh["faces"] = [[0, 2]]
        plan = step2cam.plan(a, mesh)
        self.assertNotIn("roughing", [o["kind"] for o in plan["setups"][0]["operations"]])
        reason = [s["reason"] for s in plan["skipped"] if s["feature"] == "roughing"][0]
        self.assertIn("camlib accepts at most 0.1 mm", reason)

    def test_roughing_says_what_web_it_leaves(self):
        plan = step2cam.plan(fixture("plate.analysis.json"), fixture("plate.mesh.json"))
        rough = [o for o in plan["setups"][0]["operations"] if o["kind"] == "roughing"][0]
        self.assertEqual(rough["web_mm"], 0.5)

    def test_roughing_from_another_side(self):
        plan = step2cam.plan(fixture("plate.analysis.json"), fixture("plate.mesh.json"),
                             rough_from="bottom")
        first = plan["setups"][0]
        self.assertEqual((first["id"], first["zero"]), ("bottom", "stock"))
        # The plate's through holes are plain, so they are drilled from the roughing side.
        self.assertEqual(sum(len(o["centers"]) for o in first["operations"] if o["kind"] == "holes"), 8)
        self.assertEqual(len(plan["setups"]), 1)

    def test_a_tapped_answer_threads_its_hole(self):
        plan = step2cam.plan(tapped({"hole.function:holes:12": "tapped:M5"}))
        (op,) = thread_ops(plan)
        self.assertEqual(
            {k: op[k] for k in ("spec", "major", "pitch", "start_diameter", "top_z", "bottom_z",
                                "through", "centers", "features")},
            {"spec": "M5", "major": 5, "pitch": 0.8, "start_diameter": 4.2, "top_z": -1,
             "bottom_z": -11, "through": True, "centers": [[97, 32]], "features": ["holes:12"]})
        kinds = [o["kind"] for o in plan["setups"][0]["operations"]]
        self.assertGreater(kinds.index("thread"), max(i for i, k in enumerate(kinds) if k == "holes"))

    def test_a_pattern_answer_threads_every_member(self):
        pattern = "hole_patterns.grid:6.7.8.10.11.13"
        plan = step2cam.plan(tapped({f"hole.function:{pattern}": "tapped:M8"}))
        (op,) = thread_ops(plan)
        self.assertEqual(sorted(op["features"]),
                         ["holes:10", "holes:11", "holes:13", "holes:6", "holes:7", "holes:8"])
        self.assertEqual((op["major"], op["pitch"]), (8, 1.25))

    def test_a_fine_pitch_is_read(self):
        plan = step2cam.plan(tapped({"hole.function:holes:9": "tapped:M10x1.25"}))
        self.assertEqual([(o["major"], o["pitch"]) for o in thread_ops(plan)], [(10, 1.25)])

    def test_a_blind_thread_is_as_deep_as_asked_or_specify_cores_default(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 4.2, 10, bottom="flat"),
                       hole([0, 0, -1], [9, 0, 0], 4.2, 10, bottom="flat")])
        a["features"][0]["faces"], a["features"][1]["faces"] = [1], [2]
        a = tapped({"hole.function:holes:0": "tapped:M5", "hole.function:holes:1": "tapped:M5@5.5"}, a)
        ops = thread_ops(step2cam.plan(a, top_stock=0))
        # Default: depth less three pitches, down to 0.5 mm: 10 - 2.4 = 7.6 -> 7.5.
        self.assertEqual(sorted((o["top_z"], o["bottom_z"]) for o in ops), [(0, -7.5), (0, -5.5)])

    def test_a_thread_deeper_than_its_hole_is_a_warning(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 4.2, 6, bottom="flat")])
        a["features"][0]["faces"] = [1]
        plan = step2cam.plan(tapped({"hole.function:holes:0": "tapped:M5@8"}, a), top_stock=0)
        self.assertEqual(thread_ops(plan), [])
        self.assertTrue(any("deeper than the hole" in w for w in plan["warnings"]))

    def test_answers_that_cannot_be_threaded_are_warnings(self):
        plan = step2cam.plan(tapped({"hole.function:holes:12": "tapped:M7",
                                     "hole.function:holes:99": "tapped:M5",
                                     "hole.function:holes:9": "clearance:M8"}))
        self.assertEqual(thread_ops(plan), [])
        text = " ".join(plan["warnings"])
        self.assertIn("M7 is not a metric size", text)
        self.assertIn("holes:99: tapped M5, but the part has no such feature", text)

    def test_a_thread_another_tool_wrote_is_a_warning(self):
        a = fixture("plate.analysis.json")
        a["existing"] = [{"kind": "note", "type": "internal thread", "faces": [12]}]
        plan = step2cam.plan(a)
        self.assertEqual(thread_ops(plan), [])
        self.assertTrue(any(w.startswith("holes:12: the file marks an internal thread")
                            for w in plan["warnings"]))

    def test_oblique_holes_and_countersinks_are_listed_not_cut(self):
        a = synthetic([
            hole([0.6, 0, -0.8], [0, 0, 0], 4, 5, bottom="flat"),
            hole([0, 0, -1], [5, 5, 0], 4, 10, csink={"major_diameter": 8, "included_angle": 90,
                                                     "axis": [0, 0, -1], "location": [5, 5, 0],
                                                     "drill_diameter": 4, "depth": 2}),
        ])
        plan = step2cam.plan(a)
        reasons = [s["reason"] for s in plan["skipped"]]
        self.assertTrue(any(r.startswith("oblique axis") for r in reasons))
        self.assertTrue(any(r.startswith("countersink") for r in reasons))

    def test_not_an_analysis(self):
        with self.assertRaises(step2cam.PlanError):
            step2cam.plan({"hello": 1})


try:
    import specify_core  # noqa: F401

    HAVE_SPECIFY_CORE = True
except ImportError:
    HAVE_SPECIFY_CORE = False


@unittest.skipUnless(HAVE_SPECIFY_CORE, "pip install specify-core")
class FromStep(unittest.TestCase):
    def test_plan_step_calls_specify_core_and_matches_its_json(self):
        got = step2cam.plan_step(FIXTURES / "plate.step")
        want = step2cam.plan(fixture("plate.analysis.json"), fixture("plate.mesh.json"))
        self.assertEqual(got, want)

    def test_without_the_mesh_there_is_no_roughing(self):
        got = step2cam.plan_step(FIXTURES / "plate.step", mesh=False)
        self.assertEqual(got, step2cam.plan(fixture("plate.analysis.json")))

    def test_a_thread_written_by_specify_is_milled(self):
        from specify_core import api

        src = str(FIXTURES / "plate.step")
        analysis = api.analyse(src)
        answers = api.export_answers(src, analysis, {"part.material": "6082-T6",
                                                     "hole.function:holes:12": "tapped:M5"})
        with tempfile.TemporaryDirectory() as tmp:
            out = str(Path(tmp) / "plate-m5.step")
            api.write(src, api.apply(analysis, answers), out, answers=answers)
            plan = step2cam.plan_step(out)
        (op,) = thread_ops(plan)
        self.assertEqual((op["spec"], op["features"], op["through"]), ("M5", ["holes:12"], True))

    def test_an_unreadable_file_is_a_plan_error(self):
        with self.assertRaises(step2cam.PlanError):
            step2cam.plan_step(FIXTURES / "plate.analysis.json")


class CommandLine(unittest.TestCase):
    def test_plan_reads_stdin_and_writes_stdout(self):
        text = (FIXTURES / "plate.analysis.json").read_text(encoding="utf-8")
        out, err = io.StringIO(), io.StringIO()
        saved = sys.stdin
        sys.stdin = io.StringIO(text)
        try:
            with redirect_stdout(out), redirect_stderr(err):
                code = step2cam.main(["plan", "-"])
        finally:
            sys.stdin = saved
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out.getvalue())["format"], "specify-cam-plan")
        self.assertIn("top: facing", err.getvalue())

    def test_the_summary_names_threads(self):
        path = Path(tempfile.mkdtemp()) / "a.json"
        path.write_text(json.dumps(tapped({"hole.function:holes:12": "tapped:M5"})))
        err = io.StringIO()
        with redirect_stdout(io.StringIO()), redirect_stderr(err):
            code = step2cam.main(["plan", str(path)])
        self.assertEqual(code, 0)
        self.assertIn("thread M5×1", err.getvalue())

    def test_a_bad_input_exits_one(self):
        err = io.StringIO()
        with redirect_stderr(err):
            code = step2cam.main(["plan", str(FIXTURES / "plate.mesh.json")])
        self.assertEqual(code, 1)
        self.assertIn("not a specify-core analysis", err.getvalue())


# -------------------------------------------------------------------------------------------
# post, against a stand-in camlib
# -------------------------------------------------------------------------------------------


class Recorded:
    """Any camlib constructor: keeps its arguments."""

    def __init__(self, kind, *args, **kwargs):
        self.kind, self.args, self.kwargs = kind, args, kwargs
        self.id = args[0] if args else kwargs.get("id")


def stand_in():
    def maker(kind):
        return lambda *a, **k: Recorded(kind, *a, **k)

    names = {
        "tools": ["EndMill", "Drill", "ThreadMill", "ToolAssembly", "HolderSection"],
        "operations": ["Facing", "Roughing", "Drilling", "Bore", "RasterClearing", "ThreadMilling"],
        "surface": ["SurfaceMesh"],
    }
    return {mod: types.SimpleNamespace(**{n: maker(n) for n in ns}) for mod, ns in names.items()}


def example_tools():
    return json.loads((ROOT / "examples" / "tools.example.json").read_text(encoding="utf-8"))


TIP_66 = 6.6 / 2 / math.tan(math.radians(59))
TIP_42 = 4.2 / 2 / math.tan(math.radians(59))


class Post(unittest.TestCase):
    camlib = staticmethod(stand_in)

    def setUp(self):
        self.library = step2cam.Library(example_tools())

    def ops(self, plan, setup=0):
        return step2cam.operations_for(plan["setups"][setup], self.library, self.camlib(),
                                       clearance=10, retract=2)

    def test_plate_drills_what_has_a_drill_and_bores_the_rest(self):
        plan = step2cam.plan(fixture("plate.analysis.json"), fixture("plate.mesh.json"))
        ops, tools, notes = self.ops(plan)
        self.assertEqual([o.kind for o in ops], ["Facing", "Roughing", "Drilling", "Drilling", "Bore"])
        facing, rough, d42, d66, bore = ops
        self.assertEqual(facing.args[1].id, "em6")
        self.assertEqual(facing.kwargs["floor_z_mm"], -1)
        mesh = rough.args[2]
        self.assertEqual(len(mesh.args[0]), 1312)
        # The allowance covers the mesh's own error twice over.
        self.assertGreaterEqual(rough.kwargs["radial_allowance_mm"], 2 * mesh.kwargs["surface_error_mm"])
        self.assertEqual(d42.args[1].id, "drill4.2")
        self.assertEqual(d66.args[1].id, "drill6.6")
        self.assertEqual(len(d66.args[2]), 6)
        # Through, onto the part's face: full diameter 1 mm past it, the tip below the stock
        # bottom (-11) allowed and said.
        self.assertEqual(d66.kwargs["depth_reference"], "full_diameter")
        self.assertEqual(d66.kwargs["depth_z_mm"], -12)
        self.assertAlmostEqual(d66.kwargs["allow_breakthrough_mm"], 1 + TIP_66 + 0.01)
        self.assertTrue(any("below the stock bottom" in n for n in notes))
        self.assertTrue(any("web" in n and "vise" in n for n in notes))
        self.assertEqual(d66.kwargs["peck_mm"], 4)
        # No 8 mm drill: helical bore with the largest end mill that fits, to the stock bottom,
        # which it cannot pass: a skin is expected and said.
        self.assertEqual((bore.args[1].id, bore.kwargs["diameter_mm"], bore.kwargs["floor_z_mm"]),
                         ("em6", 8.0, -11))
        self.assertTrue(any("skin or burr" in n for n in notes))
        self.assertEqual(sorted(t.id for t in tools), ["drill4.2", "drill6.6", "em6"])
        for tool in tools:
            self.assertGreater(tool.kwargs["gauge_length_mm"], tool.kwargs["stickout_mm"])

    def test_stock_below_lets_a_bore_break_through(self):
        plan = step2cam.plan(fixture("plate.analysis.json"), bottom_stock=2)
        ops, _, notes = self.ops(plan)
        bore = [o for o in ops if o.kind == "Bore"][0]
        self.assertEqual(bore.kwargs["floor_z_mm"], -12)
        self.assertFalse(any("skin or burr" in n for n in notes))

    def test_blind_flat_holes_stop_at_the_bottom_with_the_tip(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 4.2, 6, bottom="flat")])
        ops, _, notes = self.ops(step2cam.plan(a, top_stock=0))
        drill = ops[0]
        self.assertEqual((drill.kwargs["depth_reference"], drill.kwargs["depth_z_mm"]), ("tip", -6))
        self.assertEqual(drill.kwargs["allow_breakthrough_mm"], 0)
        self.assertTrue(any("full diameter ends" in n and "short" in n for n in notes))

    def test_drill_point_holes_reach_full_diameter_at_the_models_depth(self):
        # quiddity's depth stops where the cone starts.
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 4.2, 6, bottom="drill_point")])
        ops, _, notes = self.ops(step2cam.plan(a, top_stock=0))
        drill = ops[0]
        self.assertEqual((drill.kwargs["depth_reference"], drill.kwargs["depth_z_mm"]),
                         ("full_diameter", -6))
        self.assertTrue(any("drill-point" in n for n in notes))

    def test_part_material_beyond_the_exit_limits_the_drill(self):
        # The far wall of a cavity 2 mm below the exit (EXIT_MARGIN 0.2 kept from it).
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 6.6, 6), hole([0, 0, -1], [10, 0, 0], 4.2, 6)])
        mesh = mesh_of(square(-8, half=15, cx=5))
        ops, _, notes = self.ops(step2cam.plan(a, mesh, top_stock=0))
        ops = [o for o in ops if o.kind == "Drilling"]
        by_tool = {o.args[1].id: o for o in ops}
        # ⌀4.2: its tip fits in the 1.8 mm, so full diameter goes 1.8 - tip past the exit.
        d42 = by_tool["drill4.2"]
        self.assertEqual(d42.kwargs["depth_reference"], "full_diameter")
        self.assertAlmostEqual(d42.kwargs["depth_z_mm"], -6 - (1.8 - TIP_42))
        self.assertAlmostEqual(d42.kwargs["depth_z_mm"] - TIP_42, -7.8)
        # ⌀6.6: its tip is longer than the room: tip at -7.8, full diameter never reaches the
        # exit, and the note says so.
        d66 = by_tool["drill6.6"]
        self.assertEqual((d66.kwargs["depth_reference"], d66.kwargs["depth_z_mm"]), ("tip", -7.8))
        self.assertTrue(any("cannot reach full diameter" in n for n in notes))
        for op in ops:
            self.assertEqual(op.kwargs["allow_breakthrough_mm"], 0)

    def test_a_hole_no_tool_fits_is_a_note_not_a_guess(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 2.5, 6)])
        ops, _, notes = self.ops(step2cam.plan(a, top_stock=0))
        self.assertEqual(ops, [])
        self.assertTrue(any("no drill of ⌀2.5" in n for n in notes))

    def test_a_drill_is_never_larger_than_the_hole(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 6.58, 10), hole([0, 0, -1], [9, 0, 0], 6.64, 10)])
        ops, _, _ = self.ops(step2cam.plan(a, top_stock=0))
        chosen = {o.kwargs.get("diameter_mm") or o.args[1].id for o in ops}
        self.assertEqual(chosen, {6.58, "drill6.6"})  # 6.58: bored; 6.64: the 6.6 drill

    def test_a_thread_is_milled_after_its_hole(self):
        plan = step2cam.plan(tapped({"hole.function:holes:12": "tapped:M5"}))
        ops, tools, notes = self.ops(plan)
        kinds = [o.kind for o in ops]
        self.assertEqual(kinds[-1], "ThreadMilling")
        thread = ops[-1]
        self.assertEqual(thread.args[1].id, "tm3")
        self.assertEqual(
            {k: thread.kwargs[k] for k in ("internal", "diameter_mm", "start_diameter_mm",
                                           "pitch_mm", "top_z_mm", "bottom_z_mm", "radial_passes",
                                           "spring_passes")},
            {"internal": True, "diameter_mm": 5, "start_diameter_mm": 4.2, "pitch_mm": 0.8,
             "top_z_mm": -1, "bottom_z_mm": -11, "radial_passes": 2, "spring_passes": 1})
        self.assertTrue(any("thread milled" in n for n in notes))

    def test_a_thread_below_the_drills_full_diameter_is_not_cut(self):
        # A flat-bottomed hole: the drill's tip stops at the bottom, so full diameter ends a tip
        # length short; a thread asked all the way down cannot be milled there.
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 4.2, 8, bottom="flat")])
        a["features"][0]["faces"] = [1]
        ops, _, notes = self.ops(step2cam.plan(tapped({"hole.function:holes:0": "tapped:M5@8"}, a),
                                               top_stock=0))
        self.assertNotIn("ThreadMilling", [o.kind for o in ops])
        self.assertTrue(any("below where the hole reaches full diameter" in n for n in notes))

    def test_a_thread_no_mill_fits_says_why(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 2.5, 10)])  # no 2.5 drill: bored... no mill
        a["features"][0]["faces"] = [1]
        ops, _, notes = self.ops(step2cam.plan(tapped({"hole.function:holes:0": "tapped:M3"}, a),
                                               top_stock=0))
        self.assertEqual(ops, [])
        self.assertTrue(any("is not cut, so it is not threaded" in n for n in notes))
        data = example_tools()
        data["tools"].append(dict(data["tools"][0], id="drill2.5", type="drill", number=9,
                                  diameter=2.5, flute_length=20, point_angle=118))
        self.library = step2cam.Library(data)
        ops, _, notes = self.ops(step2cam.plan(tapped({"hole.function:holes:0": "tapped:M3"}, a),
                                               top_stock=0))
        self.assertEqual([o.kind for o in ops], ["Drilling"])
        self.assertTrue(any("no thread mill can cut it (tm3: ⌀3 does not fit the ⌀2.5 hole)" in n
                            for n in notes), notes)

    def test_counterbores_are_bored_with_an_end_mill(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 6.6, 6.5, cbore={"diameter": 11, "depth": 3.5})])
        ops, _, _ = self.ops(step2cam.plan(a, top_stock=0))
        cbore = [o for o in ops if o.kind == "Bore"][0]
        self.assertEqual((cbore.kwargs["diameter_mm"], cbore.kwargs["floor_z_mm"]), (11, -3.5))

    def test_the_plan_is_not_changed_by_posting(self):
        plan = step2cam.plan(fixture("plate.analysis.json"))
        before = copy.deepcopy(plan)
        self.ops(plan)
        self.assertEqual(plan, before)


class Library(unittest.TestCase):
    def test_holder_and_cutting_data_have_no_defaults(self):
        for key in ("holder_diameter", "holder_length", "coolant", "point_angle", "plunge_mm_min"):
            data = example_tools()
            for tool in data["tools"]:
                tool.pop(key, None)
            with self.subTest(key), self.assertRaises(step2cam.PostError) as caught:
                step2cam.Library(data)
            self.assertIn(key, str(caught.exception))

    def test_a_holder_narrower_than_the_shank_is_refused(self):
        data = example_tools()
        data["tools"][0]["holder_diameter"] = 3
        with self.assertRaises(step2cam.PostError):
            step2cam.Library(data)

    def test_a_thread_mill_needs_its_tooth_and_neck(self):
        data = example_tools()
        tm = [t for t in data["tools"] if t["type"] == "thread_mill"][0]
        del tm["neck_diameter"]
        with self.assertRaises(step2cam.PostError) as caught:
            step2cam.Library(data)
        self.assertIn("neck_diameter", str(caught.exception))
        tm["neck_diameter"] = tm["diameter"]
        with self.assertRaises(step2cam.PostError):
            step2cam.Library(data)

    def test_only_table_tool_changes_are_posted(self):
        data = example_tools()
        data["machine"]["tool_change_position_mode"] = "z_home"
        plan = step2cam.plan(fixture("plate.analysis.json"))
        with tempfile.TemporaryDirectory() as out, self.assertRaises(step2cam.PostError) as caught:
            step2cam.post(plan, data, out, log=lambda _: None)
        self.assertIn("table", str(caught.exception))

    def test_the_wrong_camlib_is_named(self):
        saved = {k: v for k, v in sys.modules.items() if k == "camlib" or k.startswith("camlib.")}
        for k in saved:
            del sys.modules[k]
        sys.modules["camlib"] = types.ModuleType("camlib")  # PyPI's unrelated camlib: no jobs
        try:
            with self.assertRaises(step2cam.PostError) as caught:
                step2cam._import_camlib()
            self.assertIn("git+https://github.com/stevecad/camlib", str(caught.exception))
        finally:
            del sys.modules["camlib"]
            sys.modules.update(saved)


# -------------------------------------------------------------------------------------------
# post, against camlib itself
# -------------------------------------------------------------------------------------------

if os.environ.get("CAMLIB_PYTHON"):
    sys.path.insert(0, os.environ["CAMLIB_PYTHON"])


def _camlib_classes():
    try:
        import camlib.operations as operations
        import camlib.surface as surface
        import camlib.tools as tools
    except ImportError:
        return None
    if not hasattr(operations, "Drilling"):
        return None
    return dict(operations=operations, surface=surface, tools=tools)


def _camlib_engine():
    try:
        import camlib.jobs  # noqa: F401  (needs camlib's compiled engine)
    except ImportError:
        return False
    return True


@unittest.skipUnless(_camlib_classes(), "camlib's Python sources (CAMLIB_PYTHON=<checkout>/python)")
class PostWithCamlibClasses(Post):
    """The same cases, built from camlib's real classes: a wrong argument name fails here."""

    camlib = staticmethod(_camlib_classes)

    def ops(self, plan, setup=0):
        ops, tools, notes = super().ops(plan, setup)
        wrapped = [Real(o) for o in ops]
        for o in ops + tools:
            o.to_dict()
        return wrapped, [Real(t) for t in tools], notes


class Real:
    """A camlib object read through the stand-in's `kind`/`args`/`kwargs`/`id`."""

    def __init__(self, obj):
        self.obj = obj
        self.kind = type(obj).__name__
        self.id = getattr(obj, "id", None)
        fields = list(getattr(obj, "__dataclass_fields__", {}))
        self.kwargs = {f: getattr(obj, f) for f in fields}
        self.args = tuple(Real(getattr(obj, f)) if hasattr(getattr(obj, f), "__dataclass_fields__")
                          else getattr(obj, f) for f in fields[:3])
        if self.kind == "SurfaceMesh":
            self.args = (obj.triangles,)


@unittest.skipUnless(_camlib_engine(), "camlib installed with its engine")
class PostToGcode(unittest.TestCase):
    def test_the_plate_posts(self):
        # With its ⌀4.2 hole tapped M5, so the thread mill runs too.
        plan = step2cam.plan(tapped({"hole.function:holes:12": "tapped:M5"}),
                             fixture("plate.mesh.json"))
        with tempfile.TemporaryDirectory() as out:
            log = []
            written = step2cam.post(plan, example_tools(), out, stem="plate", log=log.append,
                                    accept={"uncertain"})
            self.assertEqual([p.name for p in written], ["plate-top.nc"])
            text = written[0].read_text()
            self.assertIn("G", text)
            self.assertIn("T5", text)  # the thread mill
            self.assertTrue(any("thread milled" in line for line in log), log)
            self.assertTrue(any(line.startswith("setup top:") for line in log), log)


if __name__ == "__main__":
    unittest.main()
