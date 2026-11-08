"""Write G-code from a camlib job that step2cam saved, with camlib's own post.

step2cam writes no G-code. Its page saves each setup as a camlib job
("Save camlib job": <part>-<setup>.camlib-job.json), the same JSON a camlib
Python job compiles to. This script runs that job in camlib and posts it with
camlib's LinuxCNC post, which checks every block it writes against the plan.

    python examples/post_with_camlib.py plate-top.camlib-job.json plate-top.nc

Needs camlib's Python package (https://github.com/stevecad/camlib). The machine
below is an EXAMPLE: replace it with your controller, using camlib's post for it
(docs/posts.md in camlib). The tool numbers and length registers are the ones
step2cam's tool library gave each operation.
"""

import argparse
import json
import math

from camlib.motion import Program
from camlib.post import GCodeDialect, Machine, ToolSetting, post_events
from camlib.posted import OperationEvents
from camlib.simulation import Simulation
from camlib.tools import ToolAssembly


def machine_for(job):
    """An example three-axis LinuxCNC mill, with the job's tool table."""
    change = job["tool_change"]
    xy = change.get("xy_mm") or job["initial_position_mm"][:2]
    tools = {t["id"]: ToolAssembly.from_dict(t) for t in job["tools"]}
    settings = {}
    for op in job["operations"]:
        tool = tools[op["tool"]]
        settings[tool.id] = ToolSetting.for_tool(
            tool,
            number=op["tool_number"],
            length_register=op["length_register"],
            position_after_mm=(xy[0], xy[1], change["gauge_z_mm"] - tool.gauge_length_mm),
        )
    return Machine(
        "Example three-axis LinuxCNC mill",
        tuple(settings.values()),
        dialect=GCodeDialect.linuxcnc(),
        output_units="mm",
        decimals=3,
        max_feed_mm_min=5000,
        max_spindle_rpm=24000,
        rapid_mm_min=5000,
        planes=("xy", "xz", "yz"),
        arcs=True,
        helices=True,
        max_sweep_rad=math.tau,
        max_tool_number=99,
        max_length_register=99,
        tool_changes=True,
        flood=True,
        mist=False,
        tool_change_position_mode="table",
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("job", help="a <part>-<setup>.camlib-job.json saved by step2cam")
    parser.add_argument("out", help="the G-code file to write")
    args = parser.parse_args()
    with open(args.job) as f:
        job = json.load(f)

    simulation = Simulation(
        job["stock"],
        job["tools"],
        revision=job["run"]["revision"],
        initial_position_mm=job["initial_position_mm"],
        sweep=job["sweep"],
        operations=job["operations"],
        verification=job.get("verification"),
        tool_change=job.get("tool_change"),
        rapid_retracts=job.get("rapid_retracts", True),
    )
    header = Program(
        job["run"]["id"],
        job["run"]["revision"],
        tuple(ToolAssembly.from_dict(t) for t in job["tools"]),
        tuple(job["initial_position_mm"]),
        (),
        tolerance_mm=job["sweep"]["tolerance_mm"],
    )
    findings = []

    def checked(update):
        if update.status not in (None, "clear"):
            findings.append(update)

    source = OperationEvents(simulation, reuse_active_tool=True, on_verification=checked)
    source.prefetch = True
    try:
        text = "".join(chunk.text for chunk in post_events(header, source, machine=machine_for(job)))
    finally:
        source.close()
    if findings:
        raise SystemExit(f"camlib's checks did not clear {len(findings)} moves: no G-code written")
    with open(args.out, "w", encoding="ascii") as f:
        f.write(text)
    print(f"{args.out}: {text.count(chr(10))} lines, posted and checked by camlib")


if __name__ == "__main__":
    main()
