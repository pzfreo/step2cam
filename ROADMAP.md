# Roadmap

What step2cam is working towards, in order. Each item is an issue; work a phase before the next
unless an item says it can run alongside. Items marked *waits on* depend on another project.
step2cam is an experiment in explainable process planning (README, "Read this first"), so the
order favours what makes the plans correct and honest before what makes them faster.

## Done

- #31 no G-code; #27 machines (the Carvera); #44 GD&T on every fixture; #53 the MachinePlan-10K
  benchmark (`tools/benchmark`); #46 camlib at its main (4034290).
- Phase 1: #67 stale roughing coverage; #36 roughing capped at the tool's reach (the stopgap;
  camlib #122 still open); #35 the tool a left-out feature needs; #34 drills that reach, pecks
  by depth over diameter (deep-hole drilling warned, not planned); #51 ISO 13399 field names.
- #47 in part: modelled 45° chamfers round a whole loop or a hole or slot mouth; single-edge
  chamfers are #80.

## Parked

- PR #28 and #57, the lathe: the focus is making 3-axis milling really good first.

## Phase 1: correct and honest about reach

Done (above). What is left from it: deep-hole drilling (pilot plus deep-hole drill), and
relaxing #36's cap once camlib #122 is fixed.

1. #77 A part with no planar face: plan it, or refuse it with the real reason.

## Phase 2: holes the way a machinist makes them

2. #32 Spot drill before twist drilling.
3. #33 Holes with no matching drill: pilot and enlarge, or an insert drill.
4. #42 Holes with a fit: drill under size and ream.
5. #50 Tapping as an alternative to thread milling.

## Phase 3: features left uncut

6. #80 Chamfers on part of a loop (single straight edges; all of the benchmark's chamfers).
7. #48 Edge rounds, with a corner-rounding end mill.
8. #49 Slots with square corners.
9. #78 Freeform and curved faces finished with camlib's 3D finishing, in order before the
    holes in them, with sloped hole entries spot-faced. *Partly waits on* pzfreo/specify-core#162
    (a finer mesh).

## Phase 4: setups, datums and workholding

10. #43 Model the vise and the clamping, and pass the jaws to camlib as fixtures.
11. #40 Order setups from the datums.
12. #41 Keep features toleranced to the same datum in one setup.
13. #45 Start each later setup from the stock the previous one left.
    *Waits on* stevecad/camlib#121.

## Phase 5: cutting data and the machine

14. #39 Describe the machine.
15. #38 Calculate cutting data from the material group, the tool and the machine.
16. #37 Rough with bounded engagement (camlib's adaptive clearing).
17. Engagement-based feeds, in #38. *Waits on* stevecad/camlib#23.
18. #51 Name the tool library's fields by ISO 13399.

## Phase 6: the lathe through camlib (parked)

19. #57 Hand lathe setups to camlib's turning (after PR #28 and #46).

## Phase 7: G-code in the browser

20. #52 Save G-code from the browser through camlib's Rust posts.
    *Waits on* stevecad/camlib#120.

## Phase 8: agents

Agents propose, the planner and camlib decide (docs/agents.md). Each item can start once what it
needs is in.

21. #58 Supplied decisions: any decision from outside, checked and traced with its source.
22. #59 Agent tools: specify-core, the planner and camlib as a command-line and MCP interface.
23. #60 Repair refusals until camlib accepts.
24. #61 Answer Specify's questions from drawings and notes.
25. #55 Propose the setup order and workholding (after #40, #43, #45).
26. #62 Turn review comments into rule changes, accepted by tests and the benchmark.
27. #63 Build the tool library and machine from catalogues.

## Alongside every phase: evidence

- #53 A benchmark on a fixed MachinePlan-10K sample, run before and after each change.
- #54 A machinist's review of the sample plans, and parts cut on a Carvera.

## From other projects

- camlib: #120 (post G-code from Rust), #121 (start from given stock), #122 (roughing layer
  depth), #123 (roughing refused since #104 on parts it cut before), and our comment on #23 (engagement-based feeds).
- specify-core: `docs/upstream-needs.md` (a schema for the analysis, the material's family, a
  finer mesh: pzfreo/specify-core#162).

## Later, if wanted

- An AP238 (STEP-NC) export, or a FreeCAD CAM export as a second way to make a plan editable
  (docs/capp-plan.md, Planned).
