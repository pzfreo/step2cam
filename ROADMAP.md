# Roadmap

What step2cam is working towards, in order. Each item is an issue; work a phase before the next
unless an item says it can run alongside. Items marked *waits on* depend on another project.
step2cam is an experiment in explainable process planning (README, "Read this first"), so the
order favours what makes the plans correct and honest before what makes them faster.

## Done

- #31 no G-code; #27 machines (the Carvera); #44 GD&T on every fixture; #53 the MachinePlan-10K
  benchmark (`tools/benchmark`); #46 camlib at its main (5d78021).
- Phase 1: #67 stale roughing coverage; #36 roughing capped at the tool's reach (cap retained;
  camlib #122 fixed); #35 the tool a left-out feature needs; #34 drills that reach, pecks
  by depth over diameter (deep-hole drilling warned, not planned); #51 ISO 13399 field names.
- #47 in part: modelled 45° chamfers round a whole loop or a hole or slot mouth; single-edge
  chamfers are #80.
- Wave 4: #32 spot drilling before twist drills (a matching spot drill cuts a countersink);
  #49 rectangular slots and pockets, square or radiused, with the corner radius left said;
  #43 the vise and clamping, the jaws passed to camlib as fixtures, side cuts kept above the
  jaws (the flip-and-face step is #86).

## Parked

- PR #28 and #57, the lathe: the focus is making 3-axis milling really good first.

## Goal: make the complex plate on a Carvera

The complex test plate (`tests/fixtures/synthetic/stepped-plate.step`, the page's "Complex plate")
is planned and checked, but can't yet be made as it is. In order:

1. #86 Turn it over onto parallels and face it to thickness (the vise route), **or** #87 hold
   the stock by its margins and cut the outline through with tabs (the clamps route, which
   suits the Carvera's bed). The #86 vise option is implemented; #94 will lay out
   workholding alternatives and choose between them.
2. #88 The Carvera's real tools and aluminium cutting data in the demo library, with a ⌀2 mm
   end mill for the slot's r1.2 corners.
3. The front holes: hold the finished part on its side (the rest of #43), checked against the
   stock the top setup left once #45 can (*waits on* stevecad/camlib#121).
4. #48 The six r0.8 edge rounds, with a corner-rounding end mill (or break them by hand).
5. A Carvera post in camlib, so the page's job becomes G-code the machine runs: *deferred*
   until there is a Carvera to test on (a draft is ready).

Acceptable as they are: the notches' sharp corners left r3 by the ⌀6 (or a smaller tool, or
dog-bone relief if the design allows), and the ⌀3.3 holes 6 × D deep drilled with pecks.

Alongside: #85 warnings as one line with notes on their operations; #90 a real industrial
machine, the Haas VF-2 (Haas NGC), with a common tool set; then a machine selector on the page.

## Phase 1: correct and honest about reach

Done (above). What is left from it: deep-hole drilling (pilot plus deep-hole drill), and
relaxing #36's cap once camlib #122 is fixed.

1. #77 A part with no planar face: plan it, or refuse it with the real reason.

## Phase 2: holes the way a machinist makes them

2. #33 Holes with no matching drill: pilot and enlarge, or an insert drill.
3. #42 Holes with a fit: drill under size and ream.
4. #50 Tapping as an alternative to thread milling.

## Phase 3: features left uncut

5. #80 Chamfers on part of a loop (single straight edges; all of the benchmark's chamfers).
6. #48 Edge rounds, with a corner-rounding end mill.
7. #78 Freeform and curved faces finished with camlib's 3D finishing, in order before the
    holes in them, with sloped hole entries spot-faced. *Partly waits on* pzfreo/specify-core#162
    (a finer mesh).

## Phase 4: setups, datums and workholding

8. #40 Order setups from the datums.
9. #41 Keep features toleranced to the same datum in one setup.
10. #45 Start each later setup from the stock the previous one left.
    *Waits on* stevecad/camlib#121.

## Phase 5: cutting data and the machine

11. #39 Describe the machine.
12. #38 Calculate cutting data from the material group, the tool and the machine.
13. #37 Rough with bounded engagement (camlib's adaptive clearing).
14. Engagement-based feeds, in #38. *Waits on* stevecad/camlib#23.
15. #51 Name the tool library's fields by ISO 13399.

## Phase 6: the lathe through camlib (parked)

16. #57 Hand lathe setups to camlib's turning (after PR #28 and #46).

## Phase 7: G-code in the browser

17. #52 Save G-code from the browser through camlib's Rust posts.
    *Waits on* stevecad/camlib#120.

## Phase 8: agents

Agents propose, the planner and camlib decide (docs/agents.md). Each item can start once what it
needs is in.

18. #58 Supplied decisions: any decision from outside, checked and traced with its source.
19. #59 Agent tools: specify-core, the planner and camlib as a command-line and MCP interface.
20. #60 Repair refusals until camlib accepts.
21. #61 Answer Specify's questions from drawings and notes.
22. #55 Propose the setup order and workholding (after #40, #43, #45).
23. #62 Turn review comments into rule changes, accepted by tests and the benchmark.
24. #63 Build the tool library and machine from catalogues.

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
