# Roadmap

What step2cam is working towards, in order. Each item is an issue; work a phase before the next
unless an item says it can run alongside. Items marked *waits on* depend on another project.
step2cam is an experiment in explainable process planning (README, "Read this first"), so the
order favours what makes the plans correct and honest before what makes them faster.

## In review

- PR #31: write no G-code; hand camlib the job and show its simulation.
- PR #44: every fixture has a STEP file carrying GD&T.
- PR #27 (machines, a Makera Carvera), #28 (the lathe) and #30 (Carvera research): to rebase onto
  #31.

## Phase 1: correct and honest about reach

What the plan leaves out today, and why, on parts deeper or larger than the demo library.

1. #36 Roughing is dropped entirely when the part is deeper than the end mill reaches.
   *Waits on* stevecad/camlib#122 for the main fix.
2. #35 A feature left out should say what tool would cut it.
3. #46 Update the pinned camlib to its current main.
4. #34 Choose the drilling strategy by depth (and check a drill's flutes reach).

## Phase 2: holes the way a machinist makes them

5. #32 Spot drill before twist drilling.
6. #33 Holes with no matching drill: pilot and enlarge, or an insert drill.
7. #42 Holes with a fit: drill under size and ream.
8. #50 Tapping as an alternative to thread milling.

## Phase 3: features left uncut

9. #47 Large edge chamfers.
10. #48 Edge rounds, with a corner-rounding end mill.
11. #49 Slots with square corners.

## Phase 4: setups, datums and workholding

12. #43 Model the vise and the clamping, and pass the jaws to camlib as fixtures.
13. #40 Order setups from the datums.
14. #41 Keep features toleranced to the same datum in one setup.
15. #45 Start each later setup from the stock the previous one left.
    *Waits on* stevecad/camlib#121.
16. #55 An agent that proposes the setup order and workholding, checked by the planner (after
    #40, #43 and #45).

## Phase 5: cutting data and the machine

17. #39 Describe the machine.
18. #38 Calculate cutting data from the material group, the tool and the machine.
19. #37 Rough with bounded engagement (camlib's adaptive clearing).
20. Engagement-based feeds, in #38. *Waits on* stevecad/camlib#23.
21. #51 Name the tool library's fields by ISO 13399.

## Phase 6: the lathe through camlib

22. #57 Hand lathe setups to camlib's turning (after PR #28 and #46).

## Phase 7: G-code in the browser

23. #52 Save G-code from the browser through camlib's Rust posts.
    *Waits on* stevecad/camlib#120.

## Alongside every phase: evidence

- #53 A benchmark on a fixed MachinePlan-10K sample, run before and after each change.
- #54 A machinist's review of the sample plans, and parts cut on a Carvera.

## From other projects

- camlib: #120 (post G-code from Rust), #121 (start from given stock), #122 (roughing layer
  depth), and our comment on #23 (engagement-based feeds).
- specify-core: `docs/upstream-needs.md` (a schema for the analysis, the material's family, a
  finer mesh).

## Later, if wanted

- An AP238 (STEP-NC) export, or a FreeCAD CAM export as a second way to make a plan editable
  (docs/capp-plan.md, Planned).
