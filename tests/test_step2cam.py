"""Tests of the step2cam bridge. Standard library only: camlib is replaced by a stand-in that
records what the bridge asks of it, so the tool choices are checked without the CAM engine.

    python3 -m unittest discover -s tests
"""

import copy
import io
import json
import types
import sys
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


def synthetic(holes, bounds=((-20, -20, -10), (20, 20, 0))):
    """An analysis with just these hole records."""
    return {
        "binding": {"source_sha256": "x", "face_count": 1, "loader_version": 2, "part": 0},
        "features": [
            {"id": f"holes:{n}", "family": "holes", "record": r, "faces": []}
            for n, r in enumerate(holes)
        ],
        "dimensioning": {"bounds": [list(bounds[0]), list(bounds[1])]},
    }


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

    def test_a_blind_hole_from_below_needs_the_bottom_setup(self):
        a = synthetic([hole([0, 0, 1], [5, 3, -10], 5, 4, bottom="flat")])
        plan = step2cam.plan(a, top_stock=0)
        bottom = [s for s in plan["setups"] if s["id"] == "bottom"][0]
        op = bottom["operations"][0]
        self.assertEqual((op["top_z"], op["bottom_z"], op["bottom"]), (0, -4, "flat"))
        # Turned over about X: model y=3 lands at stock y = 20 - 3 + 2 margin.
        self.assertEqual(op["centers"], [[27, 19]])
        # The origin recorded for the bottom setup maps back to the model point.
        self.assertEqual(bottom["origin_model"], [-22, 22, -10])

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
        "tools": ["EndMill", "Drill", "ToolAssembly", "HolderSection"],
        "operations": ["Facing", "Roughing", "Drilling", "Bore", "RasterClearing"],
        "surface": ["SurfaceMesh"],
    }
    return {mod: types.SimpleNamespace(**{n: maker(n) for n in ns}) for mod, ns in names.items()}


class Post(unittest.TestCase):
    def setUp(self):
        self.library = step2cam.Library(json.loads((ROOT / "examples" / "tools.example.json").read_text()))

    def ops(self, plan, setup=0):
        return step2cam.operations_for(plan["setups"][setup], self.library, stand_in(),
                                       clearance=10, retract=2)

    def test_plate_drills_what_has_a_drill_and_bores_the_rest(self):
        plan = step2cam.plan(fixture("plate.analysis.json"), fixture("plate.mesh.json"))
        ops, tools, notes = self.ops(plan)
        self.assertEqual([o.kind for o in ops], ["Facing", "Roughing", "Drilling", "Drilling", "Bore"])
        self.assertEqual(notes, [])
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
        # Through: full diameter 1 mm past the bottom, the point allowed below the stock.
        self.assertEqual(d66.kwargs["depth_reference"], "full_diameter")
        self.assertEqual(d66.kwargs["depth_z_mm"], -12)
        self.assertGreater(d66.kwargs["allow_breakthrough_mm"], 1 + 6.6 / 2 / 1.7)
        self.assertEqual(d66.kwargs["peck_mm"], 4)
        # No 8 mm drill: helical bore with the largest end mill that fits.
        self.assertEqual((bore.args[1].id, bore.kwargs["diameter_mm"], bore.kwargs["floor_z_mm"]),
                         ("em6", 8.0, -11))
        self.assertEqual(sorted(t.id for t in tools), ["drill4.2", "drill6.6", "em6"])
        for tool in tools:
            gauge = tool.kwargs["gauge_length_mm"]
            self.assertGreater(gauge, tool.kwargs["stickout_mm"])

    def test_blind_holes_stop_at_the_bottom_with_the_tip(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 4.2, 6, bottom="flat")])
        ops, _, notes = self.ops(step2cam.plan(a, top_stock=0))
        drill = ops[0]
        self.assertEqual((drill.kwargs["depth_reference"], drill.kwargs["depth_z_mm"]), ("tip", -6))
        self.assertEqual(drill.kwargs["allow_breakthrough_mm"], 0)
        self.assertTrue(any("flat bottom" in n for n in notes))

    def test_a_hole_no_tool_fits_is_a_note_not_a_guess(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 2.5, 6)])
        ops, _, notes = self.ops(step2cam.plan(a, top_stock=0))
        self.assertEqual(ops, [])
        self.assertTrue(any("no drill of ⌀2.5" in n for n in notes))

    def test_counterbores_are_bored_with_an_end_mill(self):
        a = synthetic([hole([0, 0, -1], [0, 0, 0], 6.6, 6.5, cbore={"diameter": 11, "depth": 3.5})])
        ops, _, _ = self.ops(step2cam.plan(a, top_stock=0))
        cbore = [o for o in ops if o.kind == "Bore"][0]
        self.assertEqual((cbore.kwargs["diameter_mm"], cbore.kwargs["floor_z_mm"]), (11, -3.5))

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

    def test_the_plan_is_not_changed_by_posting(self):
        plan = step2cam.plan(fixture("plate.analysis.json"))
        before = copy.deepcopy(plan)
        self.ops(plan)
        self.assertEqual(plan, before)


if __name__ == "__main__":
    unittest.main()
