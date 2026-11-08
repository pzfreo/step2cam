# Roadmap

What step2cam is working towards, in order. Each item is an issue; work a phase before the next
unless an item says it can run alongside. Items marked *waits on* depend on another project.
step2cam is an experiment in explainable process planning (README, "Read this first"), so the
order favours what makes the plans correct and honest before what makes them faster.

## Done

- #31 no G-code; #27 machines (the Carvera); #44 GD&T on every fixture; #53 the MachinePlan-10K
  benchmark (`tools/benchmark`); #46 camlib at its main (4034290).

## Parked

- PR #28 and #57, the lathe: the focus is making 3-axis milling really good first.

## Phase 1: correct and honest about reach

What the plan leaves out today, and why, on parts deeper or larger than the demo library.

1. #67 Features still "covered by roughing" after the roughing is dropped.
2. #36 Roughing is dropped entirely when the part is deeper than the end mill reaches: rough
   to what the tool reaches meanwhile. *Waits on* stevecad/camlib#122 for the main fix.
3. #35 A feature left out should say what tool would cut it.
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

## Phase 5: cutting data and the machine

16. #39 Describe the machine.
17. #38 Calculate cutting data from the material group, the tool and the machine.
18. #37 Rough with bounded engagement (camlib's adaptive clearing).
19. Engagement-based feeds, in #38. *Waits on* stevecad/camlib#23.
20. #51 Name the tool library's fields by ISO 13399.

## Phase 6: the lathe through camlib (parked)

21. #57 Hand lathe setups to camlib's turning (after PR #28 and #46).

## Phase 7: G-code in the browser

22. #52 Save G-code from the browser through camlib's Rust posts.
    *Waits on* stevecad/camlib#120.

## Phase 8: agents

Agents propose, the planner and camlib decide (docs/agents.md). Each item can start once what it
needs is in.

23. #58 Supplied decisions: any decision from outside, checked and traced with its source.
24. #59 Agent tools: specify-core, the planner and camlib as a command-line and MCP interface.
25. #60 Repair refusals until camlib accepts.
26. #61 Answer Specify's questions from drawings and notes.
27. #55 Propose the setup order and workholding (after #40, #43, #45).
28. #62 Turn review comments into rule changes, accepted by tests and the benchmark.
29. #63 Build the tool library and machine from catalogues.

## Alongside every phase: evidence

- #53 A benchmark on a fixed MachinePlan-10K sample, run before and after each change.
- #54 A machinist's review of the sample plans, and parts cut on a Carvera.

## From other projects

- camlib: #120 (post G-code from Rust), #121 (start from given stock), #122 (roughing layer
  depth), #123 (roughing refused since #104 on parts it cut before), and our comment on #23 (engagement-based feeds).
- specify-core: `docs/upstream-needs.md` (a schema for the analysis, the material's family, a
  finer mesh).

## Later, if wanted

- An AP238 (STEP-NC) export, or a FreeCAD CAM export as a second way to make a plan editable
  (docs/capp-plan.md, Planned).
