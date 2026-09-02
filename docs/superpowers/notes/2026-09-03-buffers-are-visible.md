# Buffers are visible

Stage 3 of `docs/superpowers/specs/2026-09-02-material-convergence-design.md`,
the one declared **a hard gate before the design runs unattended**. This note
says which of the spec's claims about the code held, what a buffer is, how
staleness is handled, and exactly how far the gate is now closed.

**Status: landed and wired.** The planner can see buffer contents, `crates/core`
can fill them, and `goal.plan` fills them before every plan. §8 says how a
reader confirms it actually ran — which matters more than usual here, because
a refresh that silently fails leaves a world indistinguishable from the one
before this change. §11 lists what is still unproven; the short version is that
none of it has met a live game.

## 1. The spec's claims, checked clause by clause

The gate was stated as:

> `PlanState` models no container contents; `FactorioWorld::on_some_entity_updated`
> is a `// TODO` no-op; the only path that can read contents is
> `rcon_inventory_contents_at`, whose only callers are the HTTP handler and the
> Lua binding. So items left in a chest are invisible and the replan re-mines
> them **with the ore already gone from the ground**.

| clause | verdict |
| --- | --- |
| `PlanState` models no container contents | **True.** Every overlay field was `added`/`removed`/`consumed`/`claimed`/`researched`/`reserved`/`characters`/`refused`, and `BotState.inventory` came from `FactorioPlayer.main_inventory` and nothing else. |
| `on_some_entity_updated` is a `// TODO` no-op | **True**, literally: `pub fn on_some_entity_updated(&self, _entity: FactorioEntity) -> Result<()> { // TODO: update entity direction; Ok(()) }`. |
| the only path that can read contents is `rcon_inventory_contents_at` | **True.** |
| whose only callers are the HTTP handler and the Lua binding | **True.** `FactorioRcon::inventory_contents_at` had exactly two callers: `crates/server/src/game/query.rs` and `crates/scripting_lua/src/globals/rcon.rs`. Nothing on any planning path. |

So the gate was real and was stated accurately. Three things the spec did *not*
say, each of which changed the design:

**(a) `on_some_entity_updated` is the wrong lever, and not because it is a
no-op.** The mod raises it from exactly one subscription —
`script.on_event(defines.events.on_player_rotated_entity, on_some_entity_updated)`
in `mods/BotBridge/control.lua`. It fires when a player *turns* an entity and at
no other time. Its payload does carry `output_inventory` and `fuel_inventory`
(`serialize_entity` in `types.lua` includes both for every entity), which is
exactly what makes it look like the right channel. A chest nobody rotates
never produces one. Implementing the TODO would have delivered directions and
no contents at all. It stays a no-op, now with a doc comment saying why it is
not the missing channel.

**(b) The tick-beat sampler is not a channel into `FactorioWorld` either.**
`SAMPLE_BOT_INTERVAL = 60` and the force sampler on a 300-tick beat write to
`script-output/botbridge/samples.jsonl` with `helpers.write_file`. That file is
ingested by `crates/core/src/record/samples.rs` into a run's archive. Nothing
in that path reaches `FactorioWorld`, and adding container contents to it would
be building a second, slower transport for data a single RCON call already
answers on demand.

**(c) The spec's proposed call site does not exist.** §8 says the refresh
belongs in "the pre-plan path (`crates/core/src/plan/planner.rs`, where the
world is sampled before `PlanState::from_world`)". `crates/core/src/plan/planner.rs`
contains no `PlanState::from_world` call and never has — the production callers
are `crates/scripting_lua/src/globals/goal/{plan,mod,recovery}.rs`. The refresh
*function* is in `crates/core/src/plan/planner.rs` (it needs the RCON handle
the `Planner` holds), and the call has to come from `crates/scripting_lua`.

**(d) The spec's storage proposal would have eroded the entity graph.** §8 asks
for "a new `FactorioWorld::update_entity_inventory(...)` that replaces the
stored `FactorioEntity` in the entity graph". Checked rather than trusted, that
cannot be done cheaply on this `EntityGraph`:

* `EntityGraph::add` **refuses** an entity when something is already at that
  position — it warns `failed to add ... blocked by ...` and skips — so a
  refresh cannot re-add. It would have to remove first.
* `EntityGraph::remove` clears every `blocked_tree` box that *intersects* the
  entity's bounding box, not only its own, and every `entity_tree` entry of the
  same name in that box. Paid once, when the game really destroyed something,
  that over-removal is tolerable. Paid on every replan by a refresh loop, it
  would quietly erode the planner's model of which ground is occupied — the
  exact model four separate runs already died on
  (`2026-09-02-placement-refusal*.md`).

So contents live **beside** the graph, in `FactorioWorld::inventories`, not
inside it. The split is the honest one anyway: the graph models *geometry*,
which changes when something is built or destroyed; this models *contents*,
which change continuously. Different refresh rates, different truth horizons.

## 2. What counts as a buffer, and why

**A buffer is an entity of a name this planner builds and unloads itself, whose
contents the game was asked about at plan time.** Today that is exactly one
name: `stone-furnace`.

The decision lives in **one** place, `BUFFER_ENTITIES` in
`crates/core/src/plan/planner.rs`, and it is the set of entities the RCON query
ever names. `crates/planner` believes whatever contents the world holds, so the
query *is* the policy. Putting a second whitelist in the planner would be two
policies able to disagree, and the planner's copy would be the one nobody
updates.

Against the two options I was asked to choose between:

* **"Every container in the world"** is more useful and invites the roster to
  empty a chest a person placed on purpose. Rejected. A furnace's *result* slot
  is also the least ambiguous inventory in the game to help yourself from —
  nobody stores things there, so anything in it was smelted by whoever's plan
  put the ore in. That is as close to "things this plan left behind" as an
  observable world gets.
* **"Only ones this plan put things in"** is smaller and deterministic, and
  across the boundary that matters it is **unimplementable**: a replan builds a
  fresh `PlanState` with no memory of the plan before it, and the stranded
  items this whole exercise exists to recover are stranded by exactly that
  discontinuity. Within one plan the items are not stranded yet.

So the answer is the third thing: *entities of a kind the planner builds*. It
is deterministic (a fixed name list), it covers the actual handover the code
performs today (`smelt_steps` hands over through a furnace), and it cannot
raid a human's chest because a human's chest is never asked about.

Two further narrowings, both local to `crates/planner` because they are about
*consistency* rather than policy, and both pinned by tests:

* **The entity is still there, under the same name.** A reading is keyed by
  tile and a tile can be cleared and rebuilt. `PlanState::from_world` drops a
  reading whose position no longer holds an entity of the name the reading came
  from. `Condition::EntityAt` would catch the first case; only the name check
  catches the second.
* **Output only, never fuel.** Coal in a burning furnace is a machine's
  consumable, not a buffer: taking it out stalls the furnace the plan may still
  be waiting on, and what is recoverable is a partly-burnt slot rather than a
  count anybody planned. The fuel reading is still carried in `crates/core`,
  because it is what the game answered and dropping data at the boundary is
  worse than carrying it.

### No distance bound, and that is the same decision as this morning's

`PlanState::buffers_holding` orders candidates nearest-first and refuses none.
The walk is priced — `Condition::AtPosition` on the `Remove`, `schedule` emits
a `Walk`, `travel_ticks` charges `distance / WALK_TILES_PER_TICK` — so a
refusal would charge the same distance twice, once as ticks and once as a veto.
That is the argument
`2026-09-03-the-plant-walk-is-priced-not-capped.md` makes for the power plant,
and it is *stronger* here: the alternative to withdrawing is making the items
again, and the whole reason this overlay exists is that making them again may
be impossible. **Under-withdrawing strands materials permanently;
over-withdrawing costs a walk.** The asymmetry points one way.

The bound that does exist is `BUFFER_ENTITIES` plus the caller's choice of when
to refresh — which is where the knowledge is.

## 3. Freshness

**What the planner assumes:** that each buffer still holds, at dispatch, what
the world reported at plan time.

**When the assumption breaks, in two ways with two different answers:**

*Another part of the same plan took them.* Prevented, purely, inside the
planner. `PlanState::buffers` is an overlay: `Effect::BufferLose` decrements it
as each `Remove` is emitted, so a second goal asking for the same plates sees
what is really left. `Condition::BufferHas` then re-checks it in `schedule`, so
a plan that got the arithmetic wrong fails at the planner
(`PlannerError::BufferShort`) rather than four minutes later on a bot that has
walked there. `two_goals_cannot_both_spend_the_same_plates` pins it.

The decrement is also what makes a *partial* withdrawal terminate: without it,
the leftover `Have` subgoal `Withdraw` emits comes straight back to `Withdraw`,
finds the buffer still full, and expands into itself until the depth guard
fires. Mutation B2 below shows exactly that.

*Somebody outside the plan took them.* Cannot be prevented, and **already fails
honestly** — this needed no new code, which was worth verifying rather than
assuming. The chain, end to end:

1. `mods/BotBridge/control.lua`'s `rcon_remove_from_inventory` compares what it
   asked for against `inventory.remove`'s return and calls
   `complain("tried to remove "..count.." "..items.name.." but removed " .. real_n)`.
2. `complain` is `rcon.print(text)` — it writes **into the RCON reply body**.
3. `FactorioRcon::judge_transfer_reply` treats a reply that is non-empty once
   the `§tick§` stamp is off as a **failure**. Its own doc comment states the
   mechanism and a test drives the real mod source to prove it.

So a short withdrawal is a failed action carrying the two numbers, not a
silently short one. The freshness story is: bounded by when the caller
refreshes, checked within the plan, and loud at the game when the world moved
anyway.

**Where staleness is smallest:** immediately before planning, which is what
`Planner::refresh_buffers` is for and where its doc says to call it.

## 4. Should `on_some_entity_updated` stop being a no-op?

**No, and the periodic read is not the right shape either. The right shape is a
pull at plan time.**

* `on_some_entity_updated` fires only on rotation (§1a). It cannot deliver
  contents.
* Factorio raises no cheap event for "a container's contents changed", so
  there is no other event to subscribe to.
* A mod-side tick-beat sampler would be a push: it pays a cost proportional to
  the number of containers on every beat, whether or not anything is planning,
  and it delivers data whose age at the moment of use is whatever the beat
  happens to be. The `samples.jsonl` beat also writes to a *file*, not to
  `FactorioWorld` (§1b), so using it would mean building a second transport.
* A pull costs one RCON round trip per plan, names only the entities in
  `BUFFER_ENTITIES` that the graph already knows about (a few dozen furnaces
  over a whole run), and makes staleness bounded by the plan's own dispatch
  delay rather than by a beat.

`on_some_entity_updated` keeps its `// TODO: update entity direction` and gains
a doc comment saying it is deliberately *not* the channel contents arrive on,
so the next reader does not rediscover this.

## 5. What landed

**`crates/core/src/factorio/world.rs`**
* `ObservedInventory { name, position, output, fuel }` — quality summed away
  into `BTreeMap<item, count>`, as `player_changed_main_inventory` already does
  for players.
* `FactorioWorld::inventories: DashMap<Pos, ObservedInventory>`.
* `observe_inventories(Vec<InventoryResponse>)` — takes the RCON reply shape
  verbatim, so nothing between the game and the map can reshape it. **An entity
  the game did not answer for is not overwritten**: the mod skips a position
  where `surface.find_entity` finds nothing, so a missing reply means "not
  found", not "empty". An entity that answers *empty* is recorded as empty,
  which is a real observation and different from silence.
* `observed_inventories() -> Vec<(Pos, ObservedInventory)>`, sorted — the only
  way to read the map, because a `DashMap` iterates in hash order and
  `crates/planner` must be deterministic.
* `forget_inventory`, called from `on_some_entity_deleted`: a mined furnace
  hands its contents to whoever mined it, so a reading left standing would
  report items in a player's pocket as sitting on the ground, and the next
  thing built on that tile would inherit them.
* A clone carries the readings (knowledge, like `placement_refusals`) rather
  than starting blank (a queue, like `teleports`).

**`crates/core/src/plan/planner.rs`**
* `BUFFER_ENTITIES` — the whole "what counts as a buffer" decision, in one
  place, with the reasoning above.
* `Planner::refresh_buffers()` — one RCON round trip, entities deduplicated by
  tile and ordered by it, answers written into the world. `Ok(0)` and no call
  when there is no RCON (`--clients 0`) or nothing to ask about. The error is
  returned rather than swallowed: planning against un-refreshed readings is the
  same staleness one round trip further out, so warn-and-carry-on is a
  defensible caller choice, and making it for everybody here is not.
* `Planner::forget_buffer` for a caller that knows an entity is gone.
* `rcon` loses its `#[allow(dead_code)]`, having acquired its first real user.

**`crates/planner`**
* `Buffer`, `withdraw_slot`, and `PlanState::buffers` — an overlay, read once in
  `from_world` like every other field, so purity holds.
* `PlanState::has_buffers` / `buffered` / `buffers_holding` / `take_from_buffer`.
* `Condition::BufferHas` and `Effect::BufferLose`. **No `BufferGain`**: nothing
  this planner emits puts items into a buffer *and expects a later goal to
  count them* — `smelt_steps` holds both action ids and states its own edge —
  so a `BufferGain` would sit in the enum unpaired with anything. Stage 2's
  chest handover is where it earns its place.
* `Withdraw`, registered ahead of `SharedSmelt`, `Smelt`, `HandCraft` and
  `Mine` in `registry_for`, and ahead of `Smelt` in `default_registry`. It is
  in the single-bot registry (unlike `SharedSmelt`) because picking items up
  out of a furnace is not a multi-bot idea: one bot can leave a smelt
  half-unloaded just as easily as four can, and the cross-bot handover widens
  the window rather than creating it.
* `PlannerError::BufferShort`.

### What `Withdraw` refuses, and why each refusal is narrow

* **`Goal::Produced`.** A withdrawal is not production. A `craft-item` trigger
  fires on the act of producing, so satisfying a `Produced` goal by taking
  finished items out of a chest would plan a technology that never unlocks.
  `Goal::Produced`'s own doc already says possession is not production; this is
  the method that would have broken that promise.
* **`Holder::Anyone`.** A withdrawal is one bot walking to one entity, so it
  needs a bot to measure from and a bot to put the items into, and `Anyone`
  names neither. Nothing is lost in practice: every `Anyone` goal a caller
  states is top-level, `SplitAcrossBots` claims it first, and the
  `Holder::Share` subgoals it emits arrive named.

### Determinism

* `buffers` is a `BTreeMap<Pos, Buffer>` filled from a **sorted** accessor.
  A buffer overlay iterated in hash order would move emission order, which
  fixes `ActionId` allocation, which fixes `schedule`'s `(end, ActionId, BotId)`
  tie-break — a correctness bug, not a style one. Two layers guard it:
  `observed_inventories` sorts, and the destination is ordered anyway.
* `buffers_holding` sorts on `(distance, x, y, name)` with `total_cmp` — a
  total order, the same shape `nearest_supply_anchor` already uses.
* `the_same_world_plans_the_same_withdrawal_twice` renders every action's id,
  label, preconditions and effects and compares eleven expansions of a two-
  buffer world.
* `observed_inventories_come_back_in_tile_order` inserts in an order that is
  neither sorted nor reverse-sorted and reads back twenty-one times.

### Every existing makespan pin passes unchanged

`cargo test -p factorio-bot-planner`: 379 lib tests and every integration file
green, `red_science.rs`, `scheduling.rs`, `smelt_roots.rs`, `seeded_roster.rs`
and `split_capacity.rs` included. `cargo test -p factorio-bot-core --lib`: 405.

The reason a new method registered ahead of `Smelt` and `Mine` moved nothing is
`PlanState::has_buffers`, the cheap first line of `Withdraw::applicable`:
nothing writes to `FactorioWorld::inventories` unless a caller pulls contents
over RCON, so every fixture in the crate answers `false` and the whole
withdrawal path costs one `BTreeMap::is_empty`. That is asserted directly by
`a_world_nobody_has_read_contents_from_has_no_buffers_at_all` rather than
inferred from the pins.

## 6. Red-first

`crates/planner/tests/buffers.rs`, written and run before any planner change:

```
---- plates_standing_in_a_furnace_are_taken_rather_than_smelted_again stdout ----
thread '…' panicked at crates/planner/tests/buffers.rs:103:5:
assertion `left == right` failed: the whole plan is one walk and one take
  left: ["craft 1 stone-furnace", "insert 1 coal into stone-furnace",
         "insert 10 iron-ore into stone-furnace", "mine 1 coal",
         "mine 10 iron-ore", "mine 5 stone", "place stone-furnace",
         "take 10 iron-plate from stone-furnace"]
 right: ["take 10 iron-plate from stone-furnace"]
```

That is the gate, printed: ten plates sitting in a furnace, and the planner
mining ten fresh ore for them.

## 7. Mutation evidence

Each mutation applied to the finished code and then reverted.

| # | mutation | tests that failed | isolated? |
| --- | --- | --- | --- |
| B1 | `Withdraw` not registered in `registry_for` (the gate reopened) | the five behavioural tests | no — this removes the mechanism all five rest on |
| B2 | `Effect::BufferLose` applies nothing | `a_partly_full_buffer_is_emptied_and_the_rest_is_made`, `the_remainder_is_the_difference_and_not_the_difference_twice`, `two_goals_cannot_both_spend_the_same_plates` | no, and instructively: without the decrement a partial withdrawal expands into itself until the depth guard fires, so it breaks termination as well as accounting |
| B3 | `Goal::Produced` allowed to withdraw | `a_produced_goal_is_never_satisfied_by_a_withdrawal` **only** | yes |
| B4 | the reading's name not checked against the entity | `a_reading_for_a_different_entity_is_not_believed` **only** | yes |
| B5 | a reading with no entity behind it is believed | `a_reading_with_no_entity_behind_it_is_not_believed` + B4's test | no — B5 removes the check B4 mutates, so it subsumes it |
| B6 | fuel readings merged into buffer contents | `coal_in_a_furnaces_fuel_slot_is_not_withdrawn` **only** | yes |
| B7 | `buffers_holding` ordered by tile instead of by distance | `two_buffers_are_drained_nearest_first` **only** | yes |
| B8 | `has_buffers` returns `true` unconditionally | the three "not believed" controls + `a_world_nobody_has_read_contents_from_has_no_buffers_at_all` | no — it is the guard all four rest on |
| B9 | `observed_inventories` not sorted | `observed_inventories_come_back_in_tile_order` **only** | yes |
| B10 | `on_some_entity_deleted` keeps the reading | `deleting_an_entity_forgets_its_contents` **only** | yes |
| B11 | an entity that did not answer is recorded as empty | `refreshing_buffers_asks_about_every_known_furnace_and_stores_the_reply` **only** | yes |
| B12 | `BUFFER_ENTITIES` widened to every `container` | `a_container_that_is_not_a_buffer_entity_is_never_asked_about` **only** | yes |

**B12 caught a hole in its own test, which is why it is worth reporting.** The
first version of `a_container_that_is_not_a_buffer_entity_is_never_asked_about`
built a world with a `wooden-chest` and **no RCON**, and asserted the refresh
returned zero. It passed under B12 — because `refresh_buffers` returns `Ok(0)`
at the `rcon.is_none()` guard *before* it ever consults the whitelist, so the
test was asserting the absence of a connection and calling it a policy. Fixed
by giving it a mock with `expect_inventory_contents_at().times(0)`: the
assertion is now that the query is never *made*, which is what the whitelist
actually promises. Without running the mutation this would have shipped as a
green test of nothing.

### Two tests that cannot go red by construction, and what they do pin

* **`a_withdrawal_replays_with_every_precondition_holding`** replays the
  schedule in tick order and checks every precondition at the moment its action
  starts. It cannot be made to fail by mutating `Withdraw` alone, because the
  same `PlanState` machinery that emits the plan is what the replay checks it
  against — a `Withdraw` that mis-counted would have failed at
  `take_from_buffer` during expansion and never reached the replay. What it
  does pin is that `Condition::BufferHas` is *checkable at all*: it is
  reachable from `schedule`, it is evaluated against a forked state, and two
  takes from two furnaces do not overlap in a way that makes either condition
  false when it starts. If `BufferHas` ever stops being wired into
  `Condition::holds`, this is the test that notices.
* **`a_clone_keeps_what_it_last_saw_in_a_buffer`** pins a choice, not a
  mechanism: readings survive `FactorioWorld::clone` like `placement_refusals`
  rather than being dropped like `teleports`. Mutating it red is trivial
  (`inventories: Default::default()` in `Clone`), so it *can* go red — but the
  reason it exists is that the choice is arguable, and a note is the wrong
  place to keep an arguable choice that a struct literal can silently reverse.

## 8. Confirming it ran: what a reader looks for

Every piece above is invisible when it works and invisible when it does not,
and those are not the same thing. **A refresh that found nothing and a refresh
that never happened leave identical worlds and identical plans**:
`FactorioWorld::inventories` is empty either way, `PlanState::has_buffers`
answers `false` either way, `Withdraw` claims nothing either way, and the plan
mines. Only one of them is a defect. So `goal.plan` narrates one line per call,
whatever came back, and **silence means the refresh did not run**.

What a reader sees on stdout, one line per `goal.plan` — which the supervisor
calls once per milestone iteration, so tens of lines over a run:

```
read the contents of 12 buffer(s) before planning
no buffers to read before planning: the world knows of no furnace or chest yet
```

The first says the seam is live and the game answered. The second says the seam
is live and the entity graph knows of no furnace yet — normal before the first
smelt, and a fact worth distinguishing from silence. Neither line appearing at
all, on a run with a game behind it, means `goal.plan` was built with no
refresher: the wiring is not in the binary.

On failure, both channels fire, because a failed refresh is both a thing this
run is about to do wrong and a thing somebody will debug later:

```
stdout (paris, narration):
  could not read what is in this world's buffers; planning as if every furnace
  and chest were empty. Anything a previous plan left in one is invisible to
  this one, so it will plan to make those materials again -- out of ore that
  may already have been mined for them

stderr (tracing, diagnostic):
  buffer refresh failed before goal.plan, planning against an empty buffer map: <error>
```

The narration deliberately names the **consequence** rather than the RPC error.
A failed refresh is not a free failure the way a missed placement pre-check is:
the pre-check narrows a window, while this decides whether a plan re-mines ore
that may be gone — which is the stall the whole exercise exists to prevent, and
is precisely the failure mode that would otherwise survive this change. The
error string is on the diagnostic channel, where it belongs.

`--clients 0` — planning with no game — has no refresher and says nothing.
There was nothing to ask and no run to mislead. Pinned by
`planning_without_a_refresher_is_not_a_failure`.

## 9. Is the gate closed?

**Closed, in code.** The planner can see buffers, `crates/core` can fill them,
and `goal.plan` fills them before every plan.

* The planner can see buffer contents, spend them, and refuse to spend them
  twice.
* A replan that finds plates in a furnace takes them instead of asking the
  world for ore that is gone.
* A partial buffer is emptied and only the difference is made.
* A stale reading — entity gone, or a different entity now standing there — is
  not believed.
* A withdrawal that the world has moved out from under fails loudly at the
  game, by a mechanism that already existed and was verified rather than
  assumed.
* `crates/core` can be told what a container holds, and is told by one function
  that issues one round trip.
* `goal.plan` asks the game, once, before it expands anything, and says out
  loud what came back — so "the buffers were empty" and "nobody asked" are
  distinguishable from a run's output (§8).

**Not closed: none of it has run against a live game.** That is the honest
residual and it is listed in §11, not buried. What this pass can say is that
every mechanism is exercised by a test, that eight of twelve mutations fail
exactly one test each, and that the two tests which cannot go red say so.

## 10. The wiring, as built

`crates/scripting_lua`, with permission, following the shape the pre-check
already set.

**`goal/mod.rs`**
* `BufferRefresher` — `Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<usize, String>> + Send>>>`,
  beside `PlacementChecker`. It returns a **count**, and that is the whole
  reason §8 is possible: nothing else distinguishes "asked, empty" from "never
  asked".
* Built from a third clone of `rcon` and from `plan_world` — the *planning*
  world, for exactly the reason the pre-check captures it: this writes what
  `PlanState::from_world` reads, and refreshing the other one would leave the
  planner reading an empty map while believing it had asked.
* A `Planner` is constructed per call rather than held. `Planner::new` is two
  `Arc` clones, and the two things it holds are the two things the closure
  already captures; keeping one would be caching a struct cheaper to rebuild
  than to reason about the lifetime of.
* Threaded through `create_lua_goal_with` as a sixth parameter, kept separate
  from `placement_checker` rather than bundled with it, so a test can supply
  either alone.

**`goal/plan.rs`**
* `narrate_buffer_refresh` — the one place the narration lives, documented with
  why both outcomes are printed and why the failure line goes to `paris` and
  the error string to `tracing`.
* Awaited **once, before** `plan_verified`'s re-site loop. Not inside it: what
  separates two rounds is the refusal ledger the previous round wrote, and
  nothing in that changes what is standing in a chest. Not after expansion
  either, which is the sharper reason — `PlanState::from_world` reads the
  buffer map at the top of every round and `Withdraw` is what decides whether
  the plan mines or walks, so a refresh afterwards would refresh a fact the
  plan had already been built without.

**Four tests**, in `plan.rs` beside the pre-check's:

| test | what it pins |
| --- | --- |
| `planning_reads_the_buffers_first` | the call exists at all — the failure it guards against *looks exactly like the code working* |
| `the_buffers_are_read_once_however_many_times_the_plan_is_re_sited` | once per `goal.plan`, not once per round. Driven with `Policy::RefuseEverything` so the loop really runs its full budget, and it asserts that first — otherwise the test proves nothing |
| `a_refresh_that_fails_still_hands_back_a_plan` | warn and carry on: a script that caught a raise here would read an unreachable game as an unsatisfiable goal |
| `planning_without_a_refresher_is_not_a_failure` | `--clients 0` is a mode, not an error, and must not narrate a buffer warning to a run with no game |

**Three forced edits elsewhere in that crate**, all compilation-driven: two in
`refusal_for`, whose match over `PlannerError` is exhaustive with no wildcard
(removing `PowerPlantTooFarFromWater`, classifying `BufferShort` as a **fault**
— it says two parts of one expansion counted the same items, not anything about
the world), and `None` added at thirteen test call sites of
`create_lua_goal_with`.

## 11. What remains

**Stage 2 (`HandOff`, the `iron-chest`) is unaffected and unstarted.** When it
lands it needs two lines: `"iron-chest"` added to `BUFFER_ENTITIES`, and
`Effect::BufferGain` beside `BufferLose` if the chest handover wants its
supplier→taker edge inferable rather than stated.

**Unproven, and named rather than left silent:**

* **No live run.** The build hold stood throughout; every claim here is from
  `cargo test` on `factorio-bot-planner` and `factorio-bot-core`, from
  `cargo check`/`cargo clippy` on `factorio-bot-scripting-lua`, from source,
  and from run 32's archive. **The `scripting_lua` tests have been type-checked
  and linted but not executed** — that is the first thing to run when the
  machine is free.
* **`Withdraw` has never been executed.** `crates/executor` needs no change for
  it — `ActionKind::Remove` already carries entity, position and slot, and
  `InventorySlot::FurnaceResult` already resolves to `crafter_output` — but
  "needs no change" is a reading of the code, not an observation of a run.
* **`recover.rs`'s tiers have never seen a withdrawal.** Its note that `Insert`
  is safe to re-run says nothing about a `Remove` from a shared buffer, which
  is now a thing that can appear in a plan.
* **The narration has never been read off a real run.** §8 says what a reader
  should look for; nobody has looked yet.
