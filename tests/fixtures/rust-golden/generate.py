"""Writes what the Python path gives, for rust/tests/parity.rs. Needs camlib.

    python tests/fixtures/rust-golden/generate.py
"""
import json
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT))

import camlib.jobs as jobs  # noqa: E402

import step2cam  # noqa: E402

fixture = lambda name: json.loads((ROOT / "tests/fixtures" / name).read_text())  # noqa: E731
plate = step2cam.plan(fixture("plate.analysis.json"), fixture("plate.mesh.json"))
(HERE / "plate.plan.json").write_text(json.dumps(plate))
(HERE / "spindle.plan.json").write_text(json.dumps(step2cam.plan(fixture("spindle.analysis.json"))))

captured = {}
original = jobs.Simulation


class Capture(original):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        captured["manifest"] = self.state.manifest


jobs.Simulation = Capture
library = json.loads((ROOT / "examples/tools.example.json").read_text())
with TemporaryDirectory() as out:
    written = step2cam.post(plate, library, out, stem="plate", accept={"uncertain"}, log=lambda _: None)
    (HERE / "plate-top.nc").write_text(written[0].read_text())
(HERE / "plate.manifest.json").write_text(json.dumps(captured["manifest"]))

# The same plate with its ⌀4.2 hole answered as tapped M5.
analysis = fixture("plate.analysis.json")
tapped = next(f["id"] for f in analysis["features"] if f["family"] == "holes" and f["record"]["diameter"] == 4.2)
analysis["resume"] = {"answers": {f"hole.function:{tapped}": "tapped:M5"}}
threaded = step2cam.plan(analysis, fixture("plate.mesh.json"))
(HERE / "plate-thread.plan.json").write_text(json.dumps(threaded))
with TemporaryDirectory() as out:
    written = step2cam.post(threaded, library, out, stem="plate", accept={"uncertain"}, log=lambda _: None)
    (HERE / "plate-thread-top.nc").write_text(written[0].read_text())
(HERE / "plate-thread.manifest.json").write_text(json.dumps(captured["manifest"]))
