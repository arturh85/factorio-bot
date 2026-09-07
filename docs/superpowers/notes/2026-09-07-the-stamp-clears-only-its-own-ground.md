# The stamp clears only its own ground

*2026-09-07. The fix for
`2026-09-07-the-stamp-mines-the-pole-run.md`, which diagnosed two defects in
`FactorioRcon::place_blueprint` and stopped at its lane boundary. Live
confirmation headless, 10x, seed 31337, debug binary, `workspace/headless-c`.*

## Two defects, two fixes, and they close different halves

The note offered three candidate fixes and warned that they are not obviously
independent. They are not, and the pairing is not the obvious one.

| | fix | closes |
|---|---|---|
| 1 | offset the build area instead of re-centring it | **defect 2** only |
| 2 | do not mine at all under `only_ghosts` | **defect 1** only |

**Fix 1 alone would not have saved the pole.** The mined pole sat at
(3.5, −2.5); the block's entities span x[3, 11] y[−4, 6] at anchor (0, 0), so
the pole is inside the **correctly offset** rectangle too. A correct sweep
would have mined it just as reliably as the mis-centred one did — sooner, in
fact, since the pole run terminates at the block by construction. Offsetting
changes *which* ground is cleared, not *whether* the plan's own poles are on it.

**Fix 2 alone would have left a real build clearing the wrong ground.** The
executor only ever passes `only_ghosts = true`, so nothing it dispatches would
notice — but `rcon.place_blueprint` from a Lua script passes `false`, and that
path mines. Both fixes landed.

Both are in `crates/core/src/factorio/rcon.rs`, with the two decisions lifted
into named, testable functions in `crates/core/src/factorio/util.rs`:
`blueprint_build_area_at` (the area translated onto the ground the block will
occupy) and `entities_to_clear` (what a real build's sweep mines out of what
the query returned).

## The counters: eliminated rather than reported

`done=true failed=0` over a mined pole is a true statement about placement and
a false statement about the outcome, and the brief asked whether that deserves
to be visible.

**It has nothing left to report.** After fix 2 the ghost stamp issues no area
query and no mine, so no entity the plan placed can be mined by the plan's own
stamp — the failure mode the counter would have had to describe cannot occur.
The remaining `only_ghosts = false` path mines deliberately, to clear its own
footprint, and now clears exactly that. A reporting channel for an impossible
case is dead code, and this project has enough of those. What was added instead
is the honest doc: `RconActuator::stamp_ghosts` used to *argue* the hazard away
via `method::blueprint`'s `is_fresh_site` gate; that argument covers the
block's own entities and nothing else, and it now says so and says what
replaced it.

## Reproduced before fixing, and each defect kills exactly one test

`crates/core/tests/the_stamp_clears_only_its_own_ground.rs`, six tests against
a fake RCON server that records every command, so what is asserted is what the
game was actually asked to do. Each defect was re-introduced as a one-line
mutation with the substitution asserted to match exactly once:

| mutation | test killed |
|---|---|
| `if !only_ghosts` → `if true` | `a_ghost_stamp_asks_the_game_to_mine_nothing` |
| translated rect → re-centred rect | `a_real_build_asks_the_game_about_the_ground_it_will_occupy` |
| `blueprint_build_area_at` drops the position | `the_swept_area_is_the_ground_the_block_will_occupy` |
| character exclusion dropped | `the_sweep_spares_characters_and_resources_and_nothing_else` |
| electric poles excluded from clearing | `the_pole_the_live_run_lost_is_inside_the_area_the_sweep_considers` |
| `build_area.contains` dropped | `the_sweep_judges_by_position_not_by_what_the_query_returned` |

Six mutations, six tests, one each — no mutation left the suite green and none
killed two. Restored with `touch` rather than `cp -p`, because a preserved
mtime makes cargo re-run the *mutated* binary against restored source.

**Every "nothing was mined" assertion is paired with a non-accidental one from
the same computation**, because a stamp that never ran would satisfy it too:

* the ghost-stamp test also asserts the `place_blueprint` command *was*
  dispatched, exactly once;
* `the_pole_...` is its control — it shows that the very same predicate,
  handed the same area at the same anchor, *does* mark an ordinary entity for
  clearing. So the pole's survival is fix 2 doing its job, not the pole sitting
  where the sweep never looked;
* the two-marker geometry test asserts the **old** rectangle differs from the
  new one at the fixture's anchor, so the marker positions are about the fix
  and not about an arbitrary geometry.

## Live

`scripts/false_power_sweep.lua`, the note's own two-marker probe. Marker A is
6.5 tiles clear of the block, marker B is inside the block's own footprint.

```
markers before the build: A=true  B=true
build: done=true failed=0 lost=0 pending=0
markers after the build:  A=true  B=true
RESULT: neither mined -- no sweep reached either marker.
```

Against the diagnosing run's `A=false  B=true`. Zero occurrences of
`mining entity in build area` in the whole log.

`scripts/false_power_probe.lua`, the same 28-entity `ElectricOreToPlate` block
with no siting hint and no cheated power, beside the run that found the defect:

| | before | after |
|---|---|---|
| plan kept at the planned tile | **40 of 41** | **41 of 41** |
| `MOVED small-electric-pole planned (3.50, −2.50)` | present | absent |
| poles standing | 9 | **10** |
| pole components carrying a generator | 1 | 1 |
| consumers lit / dark | 6 / 0 | 6 / 0 |

The pole the two diagnosing runs both lost is standing. **The lit verdict is
not the evidence here** — the block was lit before the fix too, by luck, and
saying so is the point: a surviving run pole happened to fall inside wire
reach. What changed is the thing luck was covering for, and 41 of 41 is what
says so.

## Fix 3 is still available and no longer load-bearing

`method::blueprint` still drops `Powering::powered` at `blueprint.rs:1768`
where `method::extract` states it (`extract.rs:393`). Unchanged — that file
belongs to a peer session.

It is **no longer needed to close this defect**: the mechanism by which the
plan's own power evaporated between `ensure_powered`'s `Some` and the standing
entities is gone, so there is nothing left for the condition to catch here. It
remains a cheap loud refusal against any *other* way a block's power could be
gone at dispatch, and the note's argument for it — a loud refusal beats a
silent dead factory — stands on its own. Defence in depth, not the fix.

## Baselines

Four offline baselines on one binary, before and after, all byte-identical —
as expected for an executor/RCON change:

| goal | world | actions / ticks |
|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,115 / 317,283 |

`gathered:crude-oil` refuses on `map.json` (uncharted ground beyond 256 tiles),
which is correct. `cargo test --workspace --no-fail-fast`: 3,008 passed, 0
failed. Clippy clean at `--all-features --all-targets --deny warnings`.
