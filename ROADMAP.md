# Roadmap

What step2cam is working towards, in order. Each item is an issue; work a phase before the next
unless an item says it can run alongside. Items marked *waits on* depend on another project.
step2cam is an experiment in explainable process planning (README, "Read this first"), so the
order favours what makes the plans correct and honest before what makes them faster.

## Done

- #31 no G-code; #27 machines (the Carvera); #44 GD&T on every fixture; #53 the MachinePlan-10K
  benchmark (`tools/benchmark`); #46 camlib at its main (c6383a5).
- Phase 1: #67 stale roughing coverage; #36 roughing capped at the tool's reach (cap retained;
  camlib #122 fixed); #35 the tool a left-out feature needs; #34 drills that reach, pecks
  by depth over diameter (deep-hole drilling warned, not planned); #51 ISO 13399 field names.
- #47 in part: modelled 45° chamfers round a whole loop or a hole or slot mouth; single-edge
  chamfers are #80.
- Wave 4: #32 spot drilling before twist drills (a matching spot drill cuts a countersink);
  #49 rectangular slots and pockets, square or radiused, with the corner radius left said;
  #43 the vise and clamping, the jaws passed to camlib as fixtures, side cuts kept above the
  jaws (the flip-and-face step is #86).
- Wave 5: #86 flip onto parallels and face to thickness, as a vise workholding option; #85
  warnings as one line with notes on their operations; #90 the Haas VF-2 (Haas NGC) with a
  sourced common tool set; #58 supplied decisions (tool and clamping choices from a person or
  an agent, checked and traced with their source).
- #111 in part: pocket and drill reach judged by camlib, its refusal relayed in plain words
  with the tool that would do; typed reach refusals and local stock-contact obstruction data
  give the holder/shank/neck verdict (#129, camlib #144).
- The page: #96 a machine menu (Carvera, Haas VF-2, LinuxCNC mill with placeholder tools and limits); #99 a phone
  layout with a menu and bottom tabs; #102 a compact feature list; #104 the two linked 3D views
  side by side above one tabbed panel; #106 features in columns; #110 the Plan tab as a list
  beside the selected operation's detail; #115 its Tool choice and plain Why in place of Full
  view; on phones, Toolpaths shows the current move, not the list. Anonymous usage counts
  (GoatCounter): return visits, own-file outcomes, which explanations are read.
- #94 workholding options (vise ordinary and flip and face) chosen by rule or by a person; #88
  real Carvera and Haas tool libraries; #50 tapping as an alternative to thread milling.

## Parked

- PR #28 and #57, the lathe: the focus is making 3-axis milling really good first.

## Goal: make the complex plate on a Carvera

The complex test plate (`tests/fixtures/synthetic/stepped-plate.step`, the page's "Complex plate")
is planned and checked, but can't yet be made as it is. In order:

1. #94 Workholding options: done for the vise. The planner plans and simulates each candidate,
   picks by rule (finishes the part, then fewest left out, setups, extra stock, time) and lets a
   person or an agent choose; the complex plate defaults to flip and face. Still to come: #87
   margins and tabs (suits the Carvera's bed), and a fixture plate screwed through the part's
   own holes, a design to work through first (screw-head and tool clearance, seating, the
   stock the first setup leaves).
2. #88 The Carvera's real tools: done (Makera's kit and aluminium presets, the 1/4" collet).
   The notches' r1.2 corners are left r2 by the ⌀4 (smaller tool or rest pass: planner work),
   and times are long under Makera's conservative presets until #37 and #38.
3. The front holes: hold the finished part on its side (the rest of #43), checked against the
   stock the top setup left once #45 can (*waits on* stevecad/camlib#121).
4. #48 The six r0.8 edge rounds, with a corner-rounding end mill (or break them by hand).
5. A Carvera post in camlib, so the page's job becomes G-code the machine runs: *deferred*
   until there is a Carvera to test on (a draft is ready).

Acceptable as they are: the notches' sharp corners left r3 by the ⌀6 (or a smaller tool, or
dog-bone relief if the design allows), and the ⌀3.3 holes 6 × D deep drilled with pecks.

## Phase 1: correct and honest about reach

Done (above). What is left from it:

1. camlib bump to main (c6383a5), with fixes for camlib #122 and #123: done. #36's roughing
   cap stays until camlib can prove shank clearance inside a pocket it has already cleared;
   relaxing the cap still caused roughing refusals in the deep-part experiment.
2. #111's holder verdict: done for independent operations. Camlib judges the assembly against
   remaining stock; after roughing exposes a pocket mouth, its local top can be admitted by
   camlib. Features with no operation of their own still depend on roughing.
3. #115's remainder: record the tools considered for flats, spot drills and wall finishing, so
   Tool choice can show the alternatives for every operation.
4. Deep-hole drilling (pilot plus deep-hole drill).
5. #77 A part with no planar face: plan it, or refuse it with the real reason.

## Phase 2: holes the way a machinist makes them

6. #95 Graded tool choices: the best tool for the part and tool materials, falling back to
   what the library has, with a warning, and asking approval for a suboptimal step.
7. #33 Holes with no matching drill: pilot and enlarge, or an insert drill.
8. #42 Holes with a fit: drill under size and ream.
9. #50 Tapping as an alternative to thread milling: done (cut taps M3–M6 on the Haas).

## Phase 3: features left uncut

10. #80 Chamfers on part of a loop (single straight edges; all of the benchmark's chamfers).
11. #48 Edge rounds, with a corner-rounding end mill.
12. #78 Freeform and curved faces finished with camlib's 3D finishing, in order before the
    holes in them, with sloped hole entries spot-faced. *Partly waits on* pzfreo/specify-core#162
    (a finer mesh).
13. Pockets with floor fillets (the bracket's base pockets), recognised as pockets.
    *Waits on* pzfreo/specify-core#163.

## Phase 4: setups, datums and workholding

14. #40 Order setups from the datums.
15. #41 Keep features toleranced to the same datum in one setup.
16. #45 Start each later setup from the stock the previous one left.
    *Waits on* stevecad/camlib#121 (in progress there).
17. #94 Workholding options: the fixture plate and #87 margins and tabs (see the goal above).

## Phase 5: cutting data and the machine

18. #39 Describe the machine.
19. #38 Calculate cutting data from the material group, the tool and the machine.
20. #37 Rough with bounded engagement (camlib's adaptive clearing).
21. Engagement-based feeds, in #38. *Waits on* stevecad/camlib#23.

## Phase 6: the lathe through camlib (parked)

22. #57 Hand lathe setups to camlib's turning (after PR #28 and #46).

## Phase 7: G-code in the browser

23. #52 Save G-code from the browser through camlib's Rust posts.
    *Waits on* stevecad/camlib#120 (in progress there).

## Phase 8: agents

Agents propose, the planner and camlib decide (docs/agents.md). Each item can start once what it
needs is in.

#58 supplied decisions is done (above).

24. #59 Agent tools: specify-core, the planner and camlib as a command-line and MCP interface,
    with #97 a command-line tool and Python API (reports as Markdown or PDF, open questions
    answered as JSON).
25. #60 Repair refusals until camlib accepts.
26. #61 Answer Specify's questions from drawings and notes.
27. #55 Propose the setup order and workholding (after #40, #45, #94).
28. #62 Turn review comments into rule changes, accepted by tests and the benchmark.
29. #63 Build the tool library and machine from catalogues.

## Alongside every phase: evidence

- #53 A benchmark on a fixed MachinePlan-10K sample, run before and after each change.
- #54 A machinist's review of the sample plans, and parts cut on a Carvera.

## From other projects

- camlib: fixed on its main: #122 (roughing layer depth), #123 (roughing refused since #104),
  #64 (which mesh check failed), #66 (undercut-neck clearance); being worked on there: #120
  (post G-code from Rust), #121 (start from given stock); #129 is fixed (whole-tool reach
  and typed refusals); and our comment on #23 (engagement-based feeds).
- specify-core: `docs/upstream-needs.md` (a schema for the analysis, the material's family);
  #162 (a finer mesh, and dimensioning for parts with no planar face); #163 (a pocket with a
  floor fillet recognised as a slot).

## Later, if wanted

- An AP238 (STEP-NC) export, or a FreeCAD CAM export as a second way to make a plan editable
  (docs/capp-plan.md, Planned).
