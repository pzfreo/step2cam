"""Writes the small parts that rust/tests/walls.rs finishes: each is a shape an outline alone would
cut wrongly. Then specify-core's analysis and mesh of each, frozen beside it.

    uv run --no-project --with build123d python tests/fixtures/synthetic/walls/make.py
"""
from pathlib import Path
from build123d import Align, Box, Cylinder, Pos, export_step
B=(Align.CENTER,Align.CENTER,Align.MIN)
out=Path(__file__).parent
parts={}
# A stem under a cap that overhangs it: a contour on the stem would go through the cap.
parts["overhang"]=Box(100,100,10,align=B)+Pos(0,0,10)*Box(30,30,30,align=B)+Pos(0,0,40)*Box(70,70,6,align=B)
# An island in a pocket, 11.7 mm from the pocket's long walls: too little for a 12 mm cutter.
block=Box(120,80,20,align=B)-Pos(0,0,5)*Box(90,35.4,15,align=B)
parts["island"]=block+Pos(0,0,5)*Box(20,12,15,align=B)
# A channel a 12 mm cutter just fits but roughing, with its allowances, does not enter.
parts["channel"]=Box(120,64,8,align=B)+Pos(0,-16.15,8)*Box(120,20,20,align=B)+Pos(0,16.15,8)*Box(120,20,20,align=B)
# A round boss on a plate: a full circle.
parts["round-boss"]=Box(100,100,10,align=B)+Pos(0,0,10)*Cylinder(20,15,align=B)
for n,p in parts.items():
    export_step(p,str(out/f"{n}.step"))
    print(n,round(p.volume))
