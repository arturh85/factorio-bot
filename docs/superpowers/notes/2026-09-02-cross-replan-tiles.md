# Cross-replan tiles — the premise was false, the blocker was not

## Status

Done. `ee0ce843`. All gates green (`cargo fmt --check`, `clippy --deny
warnings`, `cargo test --workspace --all-features`: 1225 passed, 0 failed).
Note not committed.

**The brief's cause is not this run's cause.** No cross-replan tile ledger was
built, and building one would have fixed nothing here. What was built is the
fix for what actually failed.

## 1. What the run says, before anything is inferred

`workspace/runs/run-1788334911-41961/events.jsonl`, read directly:

* **Every plan in the run was scheduled onto one bot.** All fifteen
  `plan_created` events carry `bots: [2]`. Bots 1, 3 and 4 got no step in any
  milestone.
* **The six iron mines went to six different tiles.** In dispatch order:
  `[-19.5,-59.5]`, `[-35.5,-56.5]`, `[-34.5,-56.5]`, `[-35.5,-57.5]`,
  `[-34.5,-57.5]`, `[-34.5,-58.5]`, `[-33.5,-59.5]`. No tile is used twice, by
  the same plan or by a later one.
* **Five of them failed, each after a whole number of ore.** Mining is 120
  ticks per ore (`planned_duration / count` = 2640/22). The failures elapsed
  1815, 726, 1695, 1210 and 242 ticks — 15, 6, 14, 10 and 2 ore — against asks
  of 22, 7, 50, 36 and 26.
* Statuses across the run: 69 `success`, 6 `failed`, **0 `lost`**. Nothing was
  outstanding when any run ended.

So: one bot, six distinct tiles, no in-flight anything. "Two successive plans
hand out the same resource tile" did not happen in this run and could not have
caused it.

## 2. What actually happened

`mods/BotBridge/control.lua`, the `on_tick` mining branch: when
`storage.p[idx].mining.entity` stops being `valid` while `mining.left > 0`, it
reports

```
ERROR: the target <ore> was gone before mining finished -- something else mined it first
```

A resource entity becomes invalid for exactly one reason: it emptied and the
game destroyed it. `on_mined_entity` decrements `mining.left` by what each
swing actually delivered, so a bot asked for 50 out of a tile holding 14 mines
it dry on swing 14 and meets that branch on the next tick. **The message names
a cause it cannot know.** There was no something else; the miner was the only
actor. That wording is what sent this investigation — and, reading the earlier
notes, at least one previous one — at a concurrency bug.

The planner asked for 50 because it believed the tile held 500.
`PlanState::resource_available` returned `DEFAULT_RESOURCE_PER_TILE` for every
tile of every ore on every map.

`docs/superpowers/notes/2026-09-02-tile-reservation.md` predicted this exactly,
in its own concerns section, including the discriminator:

> a plan that asks one bot for 400 stone will emit one action against one tile
> that may really hold far less … **If the run's milestone-1 `iron-ore` failure
> recurs *with a single bot*, this is why.**

It recurred with a single bot.

## 3. The premise was checkable and false; here is what is actually true

The brief warned that both previous cross-boundary fixes rested on a false
premise. This is the third. Four claims, checked:

* **"A plan is dispatched while the previous plan's mining may still be in
  flight."** False. `scripts/supervisor.lua` is a state machine —
  `planning` → `running` → `planning` — and the `running` arm is one blocking
  `goal.run(plan)` call. `run_into` (`crates/executor/src/run.rs`) `join_all`s
  every bot's future before returning, and `LoseTrackOnDrop` marks anything
  outstanding `Lost` on every exit including the abnormal ones. Nothing plans
  while something runs.
* **"Rung 6 replans often."** True — seven plans — but each replan follows a
  completed run, not an overlapping one.
* **"The world it observes has not yet lost the ore the earlier plan is
  consuming."** Not the failing mechanism. Every tile failed on its *first*
  use in the whole run. What the world had not learned was not "this run mined
  it" but "twenty earlier runs mined it, and the game has been telling you the
  remainder all along".
* **"The `EntityGraph` structurally cannot hold this."** False, and this is the
  load-bearing one. `serialize_entity` (`mods/BotBridge/types.lua`) has always
  sent `record.amount = entity.amount` for `type == "resource"`;
  `FactorioEntity::amount` (`crates/core/src/types.rs`) has always deserialised
  it; `crates/core/tests/live_2_1_payloads.rs` already asserts
  `ore[0].amount == Some(13)` off a live 2.1.17 capture. Step 1 of the
  tile-reservation note's four-step plan was already done and nobody had
  noticed. `EntityGraph::add` threw the field away because `resources` was a
  `BTreeSet<Pos>`.

## 4. The fix

`resources: DashMap<String, BTreeSet<Pos>>` → `DashMap<String, BTreeMap<Pos,
Option<u32>>>`, plus `EntityGraph::resource_amount(name, &pos) -> Option<u32>`,
plus one line in `PlanState::resource_available`:

```rust
let capacity = self.base.entity_graph
    .resource_amount(item, &key)
    .unwrap_or(DEFAULT_RESOURCE_PER_TILE);
capacity.saturating_sub(self.consumed.get(&key).copied().unwrap_or(0))
```

Three properties worth stating:

* **`None` is "nobody said", not zero.** A live tile can never be `Some(0)` —
  the game destroys an emptied resource and `on_resource_depleted` removes it
  from the map. `None` is a hand-built fixture (`FactorioEntity::new_resource`
  sets no amount) or a blueprint import. That is why `DEFAULT_RESOURCE_PER_TILE`
  survives as a fallback and why **no existing test moved**: every fixture in
  the workspace is silent, so every fixture still reads 500.
* **The substitution happens in exactly one place.** A second substitution site
  is how the seats count and the tile walk would start disagreeing about what a
  patch can hold.
* **A repeat delivery refreshes the amount.** Resource tiles arrive at
  `EntityGraph::add` more than once by design of the transport, and the repeat
  carries a *newer* reading than the stored one. Dropping it because the tile is
  not new would be the same mistake as never reading it. A delivery carrying no
  amount leaves a known one alone — silence is not a report of zero.

Purity, determinism and the tile-centre convention are untouched:
`resource_amount` is another read of the same immutable snapshot
`resource_contains` already read, keyed through the same flooring `Pos`, and
nothing converts a position anywhere in the change.

## 5. The four selectors still agree

They agree because they were never changed. All four —
`resource_tiles_for`, `nearest_resource_tile`, `resource_supply_at_least`,
`resource_seats` — reach capacity through `resource_unclaimed` →
`resource_available`, and only that one function learned anything.

What changes and what does not:

* **`resource_seats` is unchanged by amounts.** It counts *tiles* that are
  non-empty and mutually spaced; a tile holding 13 is as non-empty as one
  holding 500. `Method::concurrency` therefore sizes splits exactly as before.
  `seats_and_selection_agree_on_reported_amounts` asserts the two worlds return
  the same seat count.
* **`resource_tiles_for` is not.** It counts *ore*, so a fifty-ore goal that
  used to be one action on one tile is now four actions on four tiles, each
  capped at what its tile holds. The same test asserts selection can still
  deliver `seats * REPORTED` — i.e. that seats never promises capacity
  selection refuses.
* **`resource_supply_at_least` becomes honest rather than generous.** A patch of
  `n` tiles holding 13 each supplies `13n` and refuses `13n + 1`; before, it
  claimed `500n`. Its documented status as an upper bound above the spacing
  limit is unchanged.

## 6. The brief's three questions, answered for the record

**1. What the next plan can actually know.** Everything it needed, already, from
the world snapshot — it just was not reading it. No new carrier was required,
and `FactorioWorld::placement_refusals` was the wrong precedent to follow here:
a refusal is a verdict the model *cannot* represent, while an ore amount is
ordinary observed world state that the model can represent and was discarding.

**2. When a commitment ends.** No commitment was added, so nothing has to
expire. Deliberately: the brief's own standard is that "a commitment that never
clears is worse than the bug", and a commitment built for a race that cannot
occur has no signal to clear on that is not invented. If the driver ever becomes
concurrent — a second script, an executor that plans while running — this
becomes a live question again, and the honest end signal at that point is the
run's completion, not the action's, because that is the only boundary the Lua
loop actually observes.

**3. Whether the executor already knows.** It does know which tile it is mining
(`ActionKind::Mine`'s baked `pos`) and `ExecutionLog` tracks the attempt. It
does **not** know the one thing that would have mattered: how much ore the tile
had. It cannot — the game reports delivered items per swing to the *mod*, and
the mod's `action_completed`/`action_failed` reply carries no remaining count.
So this was not plumbing an existing fact; the fact existed one layer further
out, in the entity payload nobody was reading.

## 7. Tests

**11 new, no existing test moved.**

`crates/planner/tests/tile_capacity.rs` (7):

* `a_tile_offers_the_amount_the_game_reported` — the defect at its smallest.
* `a_tile_with_no_reported_amount_falls_back_to_the_modelled_one` — control;
  the fallback's remaining job, and why nothing else moved.
* `one_bot_never_asks_one_tile_for_more_ore_than_it_holds` — **the run's
  shape**. One bot, `Have{iron-ore, 50}`, tiles holding 13: at least four mining
  actions, none asking a tile for more than it holds, distinct tiles, total
  still 50.
* `the_same_goal_is_one_action_when_nobody_reports_an_amount` — the
  falsification side by side; the same goal on the same map is one `mine 50`
  when the amount is unknown, which is the run's action id 13 verbatim.
* `a_second_plan_sees_the_whole_patch_again` — **the control that the
  commitment clears.** Claims are per-`PlanState`; a fresh state built from the
  same world offers every tile the first plan committed to, at full amount, and
  plans identically. This is the test that would fail if a cross-replan ledger
  were added without an expiry.
* `seats_and_selection_agree_on_reported_amounts` — the seats/selection
  agreement, plus the supply test refusing `13n + 1`.
* `capacity_aware_tile_assignment_is_deterministic` — two expansions, identical
  tiles and takes. (`expansion_is_deterministic` compares labels, which carry
  the count but not the tile.)

`crates/core/src/graph/entity_graph.rs` (4): a reported amount is kept; a tile
nobody reported an amount for reads as unknown (not zero, not full); an unknown
tile has no amount; a second delivery refreshes the amount without duplicating
the tile; a delivery without an amount does not erase a known one.

**Confirmed non-vacuous.** With `resource_amount`'s result stubbed to `None`,
4 of the 7 planner tests fail and the 3 controls pass — which is what controls
are for. With the amount refresh in `add` stubbed out, 1 of the 4 core tests
fails.

## 8. Reported, not fixed

1. **The mod's message is a diagnosis it is not entitled to make.** "something
   else mined it first" is one of two possible causes and the *less* likely one
   now that within-plan exclusivity exists. It cost this investigation its first
   several hours and, reading the note trail, is what pointed the previous one
   at concurrency too. `mining.left` and the entity's last known amount are both
   in hand at that branch; "delivered N of M before the tile emptied" would be
   both true and diagnostic. Not touched here because I cannot run Factorio to
   verify a mod change (and `workspace/mods/` shadows the repo copy, so an
   unverified edit is worse than none).
2. **Amounts are as of the last chunk writeout, and nothing refreshes them
   mid-run.** Full depletion is handled — `on_resource_depleted` →
   `on_some_entity_deleted` → `EntityGraph::remove` — and
   `on_player_mined_entity` also routes to `on_some_entity_deleted`, so a tile
   leaves the model early rather than late. The residual is a tile partially
   mined in *this* run reading at its run-start amount. It did not cause this
   run (every failing tile failed on its first use) and it errs in the
   pessimistic direction, but it is the next thing to look at if the same error
   recurs on a tile the same run already touched. Adjacent to the "nothing
   retires a mined-out tile" gap the brief reserved; not touched.
3. **`DEFAULT_RESOURCE_PER_TILE` is now unreachable on any live path** and lives
   only for fixtures. If a live tile ever reads 500 exactly, that is a
   transport bug, not a rich patch.
4. **The scheduler put every one of fifteen plans on bot 2 alone**, with three
   idle bots in the roster, across six milestones. That is not this defect and
   did not cause it — but it is the reason the run had no concurrency for a
   concurrency fix to help, and it is a large amount of unused capacity. It is
   the same single-bot concentration `run-1788322836-81715` and
   `run-1788329146-40305` were noted for. Worth its own pass.
