"""Square and r3 blind pockets plus an edge-open square slot, in millimetres.

Run with build123d, then freeze analysis AND mesh with specify-core WASM 0.1.12.
No GD&T answers: these fixtures test geometry and corner deviations only.
"""
from pathlib import Path
from build123d import Align, Box, Pos, RectangleRounded, Plane, extrude, export_step

B = (Align.CENTER, Align.CENTER, Align.MIN)
part = Box(80, 50, 12, align=B)
part -= Pos(-20, 5, 6) * Box(24, 10, 7, align=B)
part -= Pos(30, -10, 7) * Box(22, 8, 6, align=B)
part -= extrude(Plane.XY.offset(6) * Pos(15, 12) * RectangleRounded(20, 8, 3), amount=7)
assert abs(part.bounding_box().size.X - 80) < 1e-6
assert len(part.solids()) == 1
export_step(part, str(Path(__file__).with_name("square-slots.step")))
