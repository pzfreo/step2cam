"""Tall wall beside an r3 blind pocket, millimetres; no GD&T answers needed."""
from pathlib import Path
from build123d import Align, Box, Pos, Plane, RectangleRounded, extrude, export_step
B = (Align.CENTER, Align.CENTER, Align.MIN)
part = Box(60, 40, 10, align=B)
part += Pos(0, 15, 10) * Box(60, 10, 60, align=B)
# Wall face Y10; pocket back wall Y2 leaves an 8 mm gap.
part -= extrude(Plane.XY.offset(6) * Pos(0, -2) * RectangleRounded(20, 8, 3), amount=5)
assert len(part.solids()) == 1
export_step(part, str(Path(__file__).with_suffix('.step')))
