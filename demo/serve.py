#!/usr/bin/env python3
"""The step2cam demo's local server: the page in this folder, and one call that takes a STEP
file through specify-core, step2cam.plan and camlib and returns everything the page draws.

    python demo/serve.py            # then open http://127.0.0.1:8765

Needs specify-core and camlib installed (see the README). It listens on localhost only.
"""

from __future__ import annotations

import argparse
import functools
import json
import sys
import tempfile
import traceback
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

DEMO = Path(__file__).resolve().parent
REPO = DEMO.parent
sys.path.insert(0, str(REPO))

import step2cam  # noqa: E402

EXAMPLE = REPO / "tests" / "fixtures" / "plate.step"
TOOLS = REPO / "examples" / "tools.example.json"


def process(step_bytes, name):
    """Everything the page needs for one STEP file, as JSON-ready data. A part that plans but
    does not post still comes back, with `post_error`, so its features can be seen."""
    from specify_core import api

    library = json.loads(TOOLS.read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory() as folder:
        folder = Path(folder)
        step = folder / "part.step"
        step.write_bytes(step_bytes)
        try:
            loaded = api.load(str(step))
            analysis = api.analyse(loaded)
            mesh = api.mesh(loaded)
            plan = step2cam.plan(analysis, mesh)
        except Exception as e:  # say which file, not which temporary path
            raise ValueError(str(e).replace(str(step), name)) from e

        log, ranges, post_error = [], {}, None

        def on_operation(setup_id, update):
            if update.stage == "begin":
                ranges.setdefault(setup_id, []).append(
                    {"id": update.operation, "start": update.completed, "end": update.completed})
            elif update.stage == "end":
                ranges[setup_id][-1]["end"] = update.completed

        def attempt():
            ranges.clear()
            step2cam.post(plan, library, folder, stem="part", accept={"uncertain"}, log=log.append,
                          on_operation=on_operation)

        try:
            try:
                attempt()
            except Exception as e:
                # camlib's roughing can refuse a curved mesh; the holes are still worth showing.
                if not any(op["kind"] == "roughing" for s in plan["setups"] for op in s["operations"]):
                    raise
                for setup in plan["setups"]:
                    setup["operations"] = [op for op in setup["operations"] if op["kind"] != "roughing"]
                plan["warnings"].append(f"Roughing left out: camlib refused it ({str(e)[:160]}…)")
                attempt()
        except Exception as e:  # camlib refusing a program, or a tool that cannot do the job
            post_error = str(e) or type(e).__name__
        programs = [
            {"setup": s["id"], "operations": ranges.get(s["id"], []),
             "nc": (folder / f"part-{s['id']}.nc").read_text(encoding="utf-8")}
            for s in plan["setups"] if (folder / f"part-{s['id']}.nc").exists()
        ]

    # The page has the mesh already; the plan's copy of it per roughing operation is not needed.
    for setup in plan["setups"]:
        for op in setup["operations"]:
            op.pop("positions", None)
            op.pop("indices", None)
    return {
        "name": name,
        "analysis": {k: analysis.get(k) for k in ("features", "faces", "binding")},
        "mesh": mesh,
        "plan": plan,
        "tools": library["tools"],
        "machine": library.get("machine"),
        "programs": programs,
        "log": log,
        "post_error": post_error,
    }


class Handler(SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def _json(self, status, value):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _run(self, step_bytes, name):
        try:
            self._json(200, process(step_bytes, name))
        except Exception as e:  # unreadable file, an assembly with no part chosen, ...
            traceback.print_exc()
            self._json(422, {"error": str(e) or type(e).__name__})

    def do_GET(self):
        if self.path.split("?")[0] == "/api/example":
            self._run(EXAMPLE.read_bytes(), EXAMPLE.name)
        else:
            super().do_GET()

    def do_POST(self):
        if self.path.split("?")[0] != "/api/process":
            self.send_error(404)
            return
        size = int(self.headers.get("Content-Length") or 0)
        name = Path(self.headers.get("X-File-Name") or "part.step").name
        self._run(self.rfile.read(size), name)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()
    handler = functools.partial(Handler, directory=str(DEMO))
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler)
    print(f"step2cam demo: http://127.0.0.1:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
