# How step2cam holds the part

This page explains, in plain terms, how step2cam decides how a part is held while it is cut, how
it keeps the cutter away from the vise, and what it does not check. The exact rules, numbers and
sources are in [the planning documentation](capp-plan.md); the vise and parallels each machine
has are in [libraries/README.md](../libraries/README.md).

## A vise, not clamps

step2cam plans for one kind of workholding: a machine vise. Two jaws grip opposite sides of the
stock, and two parallels (hardened bars) sit under it to set its height. It does not place strap
clamps, toe clamps or fixtures. Two other routes are listed on the Plan tab with the reason they
are not offered yet: screwing the part to a fixture plate through its own holes, and leaving tabs
in a margin of stock.

So the question is not *where can a clamp go?* but *how deep can the jaws grip, and what must the
cuts stay away from?* When a feature and the grip collide, step2cam does not move the vise. It
changes the cuts, changes the order of setups, or reports what cannot be cut.

## Where the jaws go

- **Which sides.** The jaws close across the stock's narrower width, gripping its two long sides.
- **How deep.** The grip is at least 3.175 mm (1/8") or a fifth of the stock's height, whichever is
  more, and at most half its height. These are step2cam's own judgement: we found no published
  minimum grip for a vise.
- **Parallels.** The parallels set the grip: grip = jaw height − parallel height. step2cam picks a
  height the machine's library lists, or a standard size (a cited set of ½–1⅝" parallels) when
  the library lists none, and says which it used.
- **Stock.** The starting bar is rounded up to a standard flat-bar size, so the grip is planned on
  the stock you would actually buy.

The jaws and parallels become solid boxes in the setup's coordinates. The setup sheet's clamping
diagrams are drawn from those boxes.

## Keeping the cutter off the vise

Two things keep the tool away from the jaws and parallels:

1. **Cuts stop above the jaws.** Cutting down the outside of the part stops at least 1 mm above
   the jaw tops. Anything that would need the cutter lower (a wall, a chamfer, roughing) is cut
   short, and the plan reports what was left for another setup.
2. **camlib checks every move.** Each jaw and parallel is handed to
   [camlib](https://github.com/stevecad/camlib) as a solid body. camlib checks every move of the
   whole tool, including its holder, against them, so a drill breaking through into a parallel is
   caught too. An operation with a move that would hit one is left out and reported. The check is
   cautious on purpose: each jaw is treated as being as thick as it is tall.

## Flip and face

Gripping the side of a part hides the bottom of that side, so a single setup can't finish the
whole outline. For that reason the sample plates use a *flip and face* route:

1. **First setup.** Extra stock is left under the part, and the jaws grip only that extra. The
   whole outside of the part, and its top side, can then be cut to size.
2. **Flip.** The part is turned over and gripped by the finished walls it now has. Its bottom is
   then faced off, removing the extra stock.

Gripping finished walls is where the features matter. A wall with holes, notches or chamfers in it
gives the jaws less to hold, so step2cam measures it rather than assuming it:

- It takes the flat faces of the part's model inside each jaw's grip band, and subtracts what
  earlier drilling, threading and edge breaking removed.
- Each jaw needs at least 60% of its band in real contact, and that contact must run at least
  3.175 mm along the jaw. A side with no certain contact means the setup is withheld, with the
  reason.
- Each parallel needs three quarters of its footprint resting on a flat face. If an edge chamfer is
  in the way, the parallels are moved inward, in small steps up to 16 mm, until they sit flat.
- The deepest grip that passes is chosen, and the plan advises soft jaws, because the jaws are now
  on finished faces.
- A grip is only trusted once the operations that finish those faces are confirmed in the plan. If
  one of them is refused or can't be cut, step2cam replans without the flip and says why.

## Is the grip strong enough?

Where the vise maker publishes a clamping force (the Haas library's Kurt DX6 does), step2cam
compares the grip's friction with the cutting force it estimates for each cut. Facing is stepped
down if that helps; other cuts that exceed it get a warning with a remedy. Where no force is
published (the Carvera's vise), each setup warns that the grip was not checked against the cut.

## What it does not do

- It doesn't choose where along the jaws the part sits, turn the part to suit a feature, or use
  step jaws, soft jaws cut to shape, strap clamps or fixture plates.
- The force check is friction against an estimated cutting force. It doesn't check that the part
  can't lift or twist in the jaws, or the contact pressure on a finished face.
- After the flip, the second setup starts from a plain rectangular block (the part's size plus
  its extra stock), rather than the exact shape the first setup left. That waits on camlib being able to carry
  stock from one setup to the next.
- None of this has been checked on a real machine. Have someone who knows machining check the
  setup before cutting.

Questions, or a case it gets wrong? [Open an issue](https://github.com/pzfreo/step2cam/issues).
