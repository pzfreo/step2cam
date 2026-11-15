# Shelved: bounded-engagement (adaptive) roughing, issue #37

Shelved 2026-11-15 by the maintainer's decision. Not merged. Supersedes the
earlier parked branch `cx/adaptive-37` (pre-refactor design).

## Why shelved
With the safeguards the review required, adaptive applies to 1 of the 32
planset plans (Haas mounting plate, 6082-T6). On that part roughing is one
~9 mm pass either way; adaptive saves ~17 s of cutting (388 → 20 moves,
26.4 → 9.4 s est.) and drops the worst measured roughing force 430 → 124 N
on a vise that allows 1,454 N. ~1,500 lines for that gain was not worth
carrying yet. It pays off when a library has sourced, material-qualified
cutting data for deep pockets or tougher materials.

## What the branch does (as of fec47e9, reviewed)
- Rule `process.roughing.bounded_engagement@1`: adaptive clearing with a
  sourced radial engagement, flute-based stepdown (reach cap kept), chip-
  thinning feed under the machine ceiling.
- Qualification: needs a policy, cutting data declared for the part's
  material group, and a force reference; otherwise raster with a traced
  reason. Only Haas aluminium (ISO N) qualifies in shipped libraries.
  Carvera policy removed (Makera data: 0.2 mm stepdown on em6; 0.2 kW spindle).
- Spindle-power cap: Kienzle tangential force × cutting speed (shared with
  the grip check, `capp::grip_force`), limited to 60% of `spindle.power_kw`
  (a step2cam judgement), depth floored to 0.05 mm, before/after traced.
- Checked raster retry when camlib refuses adaptive, exhausts its work
  budget, or emits no moves; tool-selection evidence kept.
- Memoised adaptive results/refusals across exclusion iterations and
  workholding candidates; replays are re-verified. Benchmark CPU main/HEAD
  681/684 s; outcomes 475/385/368/15 unchanged; all 32 plans' feature
  outcomes unchanged.
- Grip check (#165) measures adaptive moves.

## Open review findings (re-review of fec47e9)
fb50a72 and c758d26 are Codex's fix round for these, committed but
NOT reviewed or fully checked (the round was stopped when the branch was
shelved). Re-review them before relying on them.
1. Major: the memo key (`rust/src/simulate.rs`, ~466) omits the stock's
   committed sweep history; jobs with different earlier operations but equal
   cells could share a memoised refusal. Fix: key on initial stock spec +
   preceding operations and tools.
2. Remembered raster fallbacks (`rust/src/lib.rs` ~50) scoped to the planned
   setup stock box, not the stock earlier setups actually leave.
3. Force reference chosen by substring "aluminium" in the material name;
   also require ISO group N ("aluminium bronze").
4. Dependent-flat depth limit now applies after raster roughing too; confirm
   no outcome change.
5. Retry exclusion matches error message text for gouge/stock-contact/scene.
6. Plan record shows raster stepover 8 mm on the adaptive op beside the 5 mm
   engagement actually used; setup sheet must show 5 mm.
7. Nits: depth floored even when the cap doesn't bind (15 → 14.95 by float
   error); stepup looked up from the setup's first roughing op.

## Other known items
- camlib (pinned 6810078) adaptive sometimes accepts a job and emits no
  moves (3 planset cases fall back to raster). Minimal reproduction was
  written to a scratch dir and not kept; regenerate from the Haas plate
  roughing job if reporting it to camlib.
- The consolidated pipeline test binary goes 120 → 132 s; the adaptive
  module is ~24 s.

## To resume
Merge origin/main, re-review fb50a72/c758d26 against the list above, rerun
planset/plancmp and the benchmark, and add qualified data for a case that
matters (e.g. a deep aluminium pocket on the Haas).
