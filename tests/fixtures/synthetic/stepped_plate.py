"""Writes stepped-plate.step and stepped-plate-chamfered.step: a synthetic part with the mix of features of NIST's CTC 01 test
part (a plate with a raised boss, through and side holes, slots, notches, a V-channel, chamfers
and rounds), at about 1/8 of its size and with its own dimensions. Not derived from NIST's model.

    uv run --no-project --with build123d python tests/fixtures/synthetic/stepped_plate.py
    then, for each NAME: specify-core analyse -o NAME.analysis.json NAME.step
                          specify-core mesh -o NAME.mesh.json NAME.step

The second has the plate's top edge chamfered. With its rounded corners and boss, camlib's roughing
currently refuses that.
"""
from pathlib import Path

from build123d import (Align, Axis, Box, Cone, Cylinder, Plane, Polygon, Pos, RectangleRounded,
                       RegularPolygon, Rot, SlotOverall, chamfer, export_step, extrude, fillet)

LENGTH, WIDTH, PLATE, BOSS = 100, 56, 12, 6
BOTTOM = (Align.CENTER, Align.CENTER, Align.MIN)


def build(chamfered):
    part = Box(LENGTH, WIDTH, PLATE, align=BOTTOM)
    part = fillet(part.edges().filter_by(Axis.Z), radius=3)  # upright corners, which roughing follows
    if chamfered:
        part = chamfer(part.faces().sort_by(Axis.Z)[-1].edges(), length=1)  # the plate's top edge

    # The raised boss, with a rounded top edge.
    boss = extrude(Plane.XY.offset(PLATE) * RegularPolygon(16, 6), amount=BOSS)
    boss = fillet(boss.faces().sort_by(Axis.Z)[-1].edges(), radius=0.8)
    part += boss

    # Through holes: corners, around the boss, and a bore through the boss.
    for x, y, d in [(sx * 42, sy * 22, 4.4) for sx in (-1, 1) for sy in (-1, 1)] + [
        (sx * 20, sy * 20, 3.3) for sx in (-1, 1) for sy in (-1, 1)
    ] + [(0, 0, 10)]:
        part -= Pos(x, y, -1) * Cylinder(d / 2, PLATE + BOSS + 2, align=BOTTOM)

    # A round-ended through slot and a square-ended one with radiused corners.
    part -= extrude(Plane.XY.offset(-1) * Pos(-34, 0) * Rot(0, 0, 90) * SlotOverall(15, 5), amount=PLATE + 2)
    part -= extrude(Plane.XY.offset(-1) * Pos(34, 0) * RectangleRounded(6, 12, 1.2), amount=PLATE + 2)

    # Through notches in the long edges.
    for y in (-WIDTH / 2, WIDTH / 2):
        part -= Pos(0, y, -1) * Box(8, 8, PLATE + 2, align=BOTTOM)

    # A V-channel across the top of the plate.
    v = Polygon((25.5, PLATE + 1), (28.5, PLATE + 1), (27, PLATE - 1.5), align=None)
    part -= extrude(Plane.XZ.offset(WIDTH / 2 + 1) * v, amount=WIDTH + 2)

    # Blind drilled holes in the front face, with drill points.
    for x in (-10, 10):
        part -= Pos(x, -WIDTH / 2 - 1, 6) * Rot(-90, 0, 0) * Cylinder(1.25, 11, align=BOTTOM)
        part -= Pos(x, -WIDTH / 2 + 10, 6) * Rot(-90, 0, 0) * Cone(1.25, 0, 0.72, align=BOTTOM)
    return part


for name, chamfered in (("stepped-plate", False), ("stepped-plate-chamfered", True)):
    export_step(build(chamfered), str(Path(__file__).with_name(f"{name}.step")))
