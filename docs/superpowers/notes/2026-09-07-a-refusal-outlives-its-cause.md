# A refusal outlives its cause

2026-09-07. Closes the question
`2026-09-06-a-refusal-that-names-what-it-found.md` left open in as many words:
*"Expiring a refusal, or re-asking the game about one before a replan, is the
open question this leaves behind."*

## The defect

`FactorioSurface::placement_refusals` is never expired, and
`PlanState::from_world` read all of it. A footprint the game refused once was
excluded for the rest of the run whatever later happened to the ground — so a
`BuildBlock` that lost one placement could never be finished by a replan, which
is the property `goal.built` exists for:

```
pass 1: done=true failed=1 pending=0
replan REFUSES: cannot build burner-mining-drill at tile (-16,-14)
```

## What was chosen: evidence-based expiry, at the read point, in one mode

`PlacementRefusal::names_only_transient_blockers`
(`crates/core/src/factorio/world.rs`) reads the evidence the game itself
attached to the refusal. `PlanState::from_world` drops the refusals it answers
`true` for; nothing else changes.

| `blockers` | `tile` | reading | kept? |
|---|---|---|---|
| only names in `TRANSIENT_BLOCKERS` | any | the cause has walked off | **no** |
| any other name | any | something is standing there | yes |
| empty | `Some` | the game looked, found no entity: the ground is the answer | yes |
| empty | `None` | the mod appended nothing — *not asked* | yes |

`TRANSIENT_BLOCKERS` is `character`, `entity-ghost`, `tile-ghost` — three
names, each already established as a non-obstacle elsewhere in the tree rather
than asserted here. A character is what both *write* sites already refuse to
record (`report_character_in_footprint` in the mod, and
`PlacementVerdict::is_durable_refusal` for the pre-check). A ghost is what
`GHOST_ENTITY_TYPES` already keeps out of `blocked_tree` and what both of
`occupant_of`'s entity loops already skip by name.

**The bottom row is the one that must not drift.** An empty blocker list with
no tile is an absence of observation, and this repo's rule for that shape —
`EntityGraph::resource_fingerprint`, `runMatch.ts` — is that equal means equal
and different means *unknown*. Reading "we did not look" as "nothing was there"
would expire every refusal recorded before the mod started naming anything, on
no evidence, and re-open the four-run failure the ledger was built for. The
existing `refusal_memory.rs` fixture is exactly that shape, and it stays green:
that suite is now also the regression test for this row.

### Why not re-ask the game

Authoritative, and rejected for two reasons that both outlast the round-trip
cost. **It works only live.** `factorio-bot plan --world <dump>` and
`score-map` have no game to ask, so the same ledger would mean two different
things depending on whether Factorio was attached — a rule "discovered by
somebody at 2am", and the offline loop is the loop this project iterates in.
And it needs the plan path (`goal/plan.rs`), not the three files this branch
owns. The evidence rule is a pure function of fields the refusal already
carries, so it runs identically in both modes and stays inside the planner's
purity contract.

### Why not a TTL

A number nobody can justify, in a week spent deleting numbers that were true
when written.

### Why the read point and not a drain

The ledger stays whole and `record.refusals()` still reports every entry. "The
game said no here and here is what it saw" is worth recording whatever a
planner later decides about it; a ledger that forgot would also forget the
evidence explaining why it forgot.

## How often `blockers` is actually populated: **it has never been populated**

Asked, because the design would be a guess otherwise. The answer is a flat
zero, and it is the finding that most shapes what to believe about this change.

- **0 `placement_refused` events across all 21 archived runs.** The event kind
  does not appear once. The ledger has never been written in any run this
  checkout still holds.
- **0 `placement_refusals` in both offline dumps** —
  `workspace/scripts/map.json` (t=0) and `map-31337-explored.json`.
- **6 of 6 archived placement failures were a character**, every one of them
  `cannot place item '<x>' because a character is standing in the footprint`
  — the wording deliberately outside the `said 'no'` family, so the mod
  correctly learned nothing durable from any of them.

Two consequences, stated plainly rather than smoothed over.

**The rule's behaviour under the unmeasurable case is "change nothing".** With
no field data on how often `blockers` is filled or right, the only defensible
design is one that expires *only* on a positive, named observation and keeps
everything else. That is what this is; it cannot mis-expire a refusal that
carries no evidence, because it never looks at one.

**And this is defence in depth on the live paths, not a repair of one.** Both
write sites already filter characters, so no *current* mod-and-binary
combination can put a character-only entry in the ledger. What can is a dumped
world, a `--resume-from` savepoint, a mod older than either filter, or a third
write site added later — and the read point is the one place all four pass
through. The 6-of-6 above says the transient this guards against is
overwhelmingly the one that actually happens.

## What is still open, and it is not this

**The (-16,-14) refusal itself remains unexplained.** This branch makes a
transiently-refused footprint rebuildable and proves it with a test; it does
not establish that the run which motivated it was refused by a transient. The
previous note retired the tree hypothesis and this one does not replace it with
a second story. Whoever picks it up wants the thing that note already names:
**nothing in the Lua bindings enumerates `blocked_tree`**, so the anonymous
rectangle that refused that tile cannot be read from a script. That is the mod's
and it belongs to its owner.

## Handed over

One request, for whoever owns `mods/BotBridge/`. Nothing here needs it and
nothing here was blocked by it, but it would turn the remaining unknown into a
reading:

> **A binding that enumerates the planner's `blocked_tree` boxes in an area**,
> so `world.*` can be compared against `rcon.find_entities_filtered` for the
> same rectangle. The 2026-09-06 note's one unexplained observation is a box
> that no source the planner had could name; today there is no way to ask what
> is in that tree from a script.

## Verification

- **All four offline baselines byte-identical**, re-measured on this branch's
  own release binary (`fe4d387f` + this change, `--no-default-features
  --features cli,lua`), seed 31337, default settings:

  | goal | world | actions / ticks |
  |---|---|---|
  | `researched:automation` | `map.json` | 176 / 21,784 |
  | `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 |
  | `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 |
  | `gathered:crude-oil` | `map-31337-explored.json` | 2,115 / 317,283 |

  **And they could not have moved**, which is worth more than the four
  matches: both dumps carry `placement_refusals: []`, so the filter had
  nothing to filter on either world. Checked before the plans were run, not
  inferred from them agreeing.

- **Seven falsifications, one break at a time, each substitution asserted to
  have matched exactly once before the result was read.** Every one landed on
  the tests it should and on no others:

  | break | red |
  |---|---|
  | the read-point filter removed | 5 (both character tests, ghosts, the search, both block tests) |
  | `!blockers.is_empty()` → `true` | 2 (both "named nothing" rows) |
  | `.all()` → `.any()` | 1 (transient beside a durable blocker) |
  | `character` out of `TRANSIENT_BLOCKERS` | 4 |
  | ghosts out of `TRANSIENT_BLOCKERS` | 1 |
  | the block cut to one entity | 4 (proves the count is derived, not a constant) |
  | the durable block refusal's blocker → `character` | 1 |

  **The guard fired once and was right to.** The `tree-01` substitution matched
  **zero** times on the first attempt — `rustfmt` had wrapped the call across
  four lines — and would have reported a green run over a mutation that never
  applied, which is the exact trap
  `2026-09-06-fixtures-agree-with-their-code.md` names. Retried line-targeted.

- `nix develop -c cargo test --workspace` green (exit 0 captured from the
  command, not from a pipeline; 0 `test result: FAILED` lines) and
  `cargo clippy --workspace --all-features --all-targets -- --deny warnings`
  clean. Nothing under `app/` touched, so no `pnpm lint` was owed.

- **The acceptance test is a test, not a claim.**
  `crates/planner/tests/refusal_expiry.rs::a_block_refused_by_a_transient_blocker_is_completable_on_replan`
  builds a `Goal::Built` whose footprint carries a character-only refusal, and
  asserts the replan plans **both** placements — with
  `the_replan_aims_at_the_tile_the_transient_refusal_named` pinning that it aims
  at the refused tile itself rather than quietly siting elsewhere, and
  `a_durable_refusal_on_a_block_tile_still_refuses_the_block` as the control
  that the block is not simply ignoring the ledger.
