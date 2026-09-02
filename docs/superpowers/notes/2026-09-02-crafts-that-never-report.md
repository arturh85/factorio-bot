# Crafts that never report

Run `run-1788347034-00981`, 2026-09-02. Every action the run lost was a craft,
and every one of those crafts *happened*. Notes on what the record shows, what
the mod was doing, what changed, and what is still unestablished.

## 1. The evidence, re-derived

`workspace/runs/run-1788347034-00981/events.jsonl` at the time of writing holds
**195 `action_dispatched` and 184 `action_settled`**. (The brief said 179/170;
the file grew by two more replans between the two readings. The shape is the
same and the conclusion is unchanged.) Pairing dispatches against settles
*within each plan* — ids are only unique inside a plan — leaves eleven
unanswered:

| plan | milestone | id | bot | action |
|---|---|---|---|---|
| 8 | 6 | 7 | 1 | craft 1 stone-furnace |
| 9 | 7 | 0, 1, 2, 3 | 1, 2, 3, 4 | craft 2–3 automation-science-pack |
| 10 | 7 | 13 | 1 | craft 1 stone-furnace |
| 11 | 7 | 21 | 1 | craft 1 stone-furnace |
| 12 | 7 | 12 | 1 | craft 1 stone-furnace |
| 13 | 7 | 19 | 1 | craft 1 stone-furnace |
| 14 | 7 | 12 | 1 | craft 1 stone-furnace |
| 15 | 7 | 19 | 1 | craft 1 stone-furnace |

**All eleven are crafts.** No mine, no walk, no place, no insert. The two the
brief did not list are the same action in two later replans.

Each cost exactly one `ACTION_RESULT_DEADLINE`. Dispatch-to-next-dispatch gaps:
50177 → 71783, 71783 → 93392, 122037 → 143649, 170844 → 192451 — 21606, 21609,
21612, 21607 ticks, against 360 s × 60 tps = 21600. The run spent its last
164 000 ticks doing nothing but waiting out crafts.

### The crafts all succeeded

This is the part that settles the diagnosis. `samples.jsonl` records every
bot's inventory on a 1 s beat:

* Bot 1, dispatch at tick 50177. At 50160 it holds 19 stone and 0
  stone-furnace; at **50220** it holds 15 stone and **1 stone-furnace**. The
  furnace was made, 43 ticks after the request, and the action never settled.
* All four bots, dispatch at 71783. Their `automation-science-pack` counts go
  0 → 3, 0 → 3, 0 → 2, 0 → 2 between ticks 72120 and 72720 — **exactly the
  counts requested**, at exactly 5 s of craft time each. None settled.

So the game did the work and raised its event; the mod failed to join that
event back to the action. This is not a Factorio failure and not an executor
failure. It is the join.

Two things the record also rules out. The server log for that run
(`workspace/server/factorio-previous.log`, started 13:02:11) contains **no Lua
error and no complaint line** — so no handler raised, and `begin_crafting` was
never seen to start fewer crafts than asked. And it contains no map *load*
after startup, only autosaves — so nothing reset a module local mid-run.

## 2. What the mod was doing

`rcon_action_start_crafting` called `player.begin_crafting{}` — the game's own
crafting queue does the durative work — and pushed `count` entries onto a
per-player list, only the last carrying the action id.
`on_player_crafted_item` compared the **head** of that list to the crafted
recipe:

```lua
if queue[1].recipe == event.recipe.name then
    ... action_completed ...   -- and pop the head
else
    -- ignored: "probably an intermediate product"
end
```

`on_player_crafted_item` carries the player and a `LuaRecipe` and nothing else
(`workspace/factorio-api-docs/runtime-api.json`, Factorio 2.1.17): no request
id, no queue position. So a join has to be built, and this one had four
properties worth naming:

1. **It head-of-line blocks, permanently.** A crafted item that does not match
   the head is *ignored and the head stays*. One entry that will never be
   crafted therefore stops **every later craft for that player, forever** — no
   timeout, no error, no log line. Nothing in the mod could ever remove an
   entry except a matching craft.
2. **A partial start left residue.** `begin_crafting` returns "the count that
   was actually started crafting". When that was less than asked, the old code
   complained — which refused the action, so nobody was waiting — **and then
   pushed all `count` entries anyway**. Every surplus entry was a permanent
   block.
3. **A cancellation left residue and no verdict.**
   `on_player_cancelled_crafting` was not registered at all. A cancelled craft
   answered nothing and its entries stayed forever.
4. **The list was a module local**, not `storage`, and `on_load` rebuilt
   nothing, so anything in flight across a save/load was lost.

## 3. What is *not* established

**Which of those routes this particular run took.** Nothing in the retained
record identifies it. Every craft before tick 46418 settled with the exact
expected latency (2 gears → 61 ticks, 5 gears → 154, 10 circuits → 310, 1 lab
→ 120 — each `crafts × recipe energy × 60` plus a round trip), so the list
drained cleanly right up to the last successful craft; the very next craft, and
every craft after it on every bot, never settled. A per-player residue explains
bot 1 and does not explain bots 2, 3 and 4, whose lists were provably empty
since tick 14456.

The one artefact that would say is the mod's stdout for that run, and it was
not retained — the run record captures parsed events, not the raw writeout
stream. Reconstructing it would need a re-run, and a live run was using
`workspace/` while this was written.

So this note claims what the record supports: **the join was the failure, the
join had several silent ways to get stuck, and all of them are now gone.** It
does not claim to know which one fired. The design open question the spec left
(`docs/superpowers/specs/2026-09-02-mod-side-actions-design.md` §8 item 5:
"whether the game ever reorders, coalesces or interleaves intermediate crafts
in a way that breaks positional matching was not established") is *still* not
established — it is now irrelevant, because nothing positional is left.

## 4. What changed

Modelled on the research fix (`f7b60f1c`), which had the same shape of problem.

### The join

`storage.craft_actions[player_index][recipe_name]` is an array of
`{ id, remaining }`, oldest first.

* **Keyed by `(player, recipe)`** because that is all the event carries — the
  same reasoning that made research key by technology name.
* **Bucketed per recipe**, so a stuck bucket cannot block a different recipe.
  This is the direct answer to failure 1: bot 1's stone furnace settles now
  whatever else is outstanding.
* **Counted, not positionally matched.** Each craft of R decrements the oldest
  request for R still owed one; that request settles when it reaches zero.
  Two requests for the same recipe on the same bot are indistinguishable to
  the event, so attributing crafts FIFO is the strongest claim the event
  supports — and it is an attribution, not a measurement, which the code says
  in place.
* **In `storage`**, so it survives a save/load.
* Buckets are pruned when empty, so the registry does not accumulate one
  permanent entry per recipe ever crafted into the save file.

### Intermediates

A craft nobody is waiting on settles nothing and consumes nothing. That is the
normal case for every intermediate `begin_crafting` queues on its own way to
the requested item, and it now costs nothing rather than depending on the head
happening not to match.

### Refusals fail fast

Three refusals, all answered in the RCON reply body — which is where
`player_craft_timed` reads a refusal, so each costs one round trip instead of
360 seconds:

* an unknown recipe, checked against `player.force.recipes` **before**
  `begin_crafting`, which *raises* on an unknown name;
* a recipe the force has not unlocked;
* a partial start — `begin_crafting` returning anything other than the count
  asked for.

The partial-start case **registers nothing**, which is the fix for failure 2.
What it does not do is undo the crafts the game did start: those complete, find
no waiter, and are ignored. That under-claims — the bot ends up holding items
the executor was told it did not get — in the direction this codebase chooses
everywhere else. Cancelling them instead would need a queue index and, per
`LuaControl.cancel_crafting`, can cascade into unrelated crafts.

One consequence worth knowing: if the executor retries a partially-started
craft, a leftover craft from the refused attempt can decrement the retry's
waiter and settle it a few seconds early. The item genuinely exists and was
genuinely crafted by that bot, so counting it is defensible; it is recorded
here so nobody rediscovers it as a mystery.

### Cancellations settle

`on_player_cancelled_crafting` is registered. `cancel_count` crafts of a recipe
are taken **from the back** of that bucket: the game's crafting queue is FIFO,
so the earliest request is nearest completion and the latest is what a
cancellation reaches first.

A request that loses any of its crafts **fails**. It asked for a count and that
count will not arrive; reporting the partial yield as a success would tell the
executor a bot holds items it does not have. A craft that is half done when the
rest is cancelled therefore fails, not succeeds — `a_partially_completed_craft_that_is_then_cancelled_fails`.

### Rust side

`player_craft_timed` now classifies a reply-body refusal as `Dispatch::Refused`
with the dispatch tick the game stamped, instead of letting it fall through
`?` to `Dispatch::NotDispatched` with `ActionTicks::UNKNOWN`. Both still reach
the executor as `Rejected` and both still fail fast; the difference is that the
run record keeps a real tick and the mod's own sentence instead of a
`Debug`-formatted `Vec<String>`. The now-unused `action_start_crafting` wrapper
was removed rather than left as a second spelling of the same wire call.

## 5. Tests

`crates/core/tests/botbridge_craft_action.rs` — 16 tests, loading the real
`mods/BotBridge/control.lua` into Lua 5.4 on a stub game and raising the
crafting events at the handlers. **9 of the 16 failed before the change**
(head-of-line blocking, storage, unknown recipe, partial start, the four
cancellation cases, and the reply-body refusal); the other 7 pin behaviour the
old code already had right, so that it stays right.

`crates/core/tests/craft_is_awaited.rs` — 3 tests over the real RCON wire
protocol against a fake server: a craft returns only when a completion for its
own action id arrives, another action's completion does not release it, and a
refusal fails at once as `Dispatch::Refused`. The third failed before the
change (`NotDispatched`).

What neither can prove: that Factorio raises `on_player_crafted_item` exactly
once per craft with the shape assumed here, or that
`on_player_cancelled_crafting` reports `cancel_count` the way this reads it.
Both are taken from `runtime-api.json` for 2.1.17. No Factorio process was
started for this change — a live run was using `workspace/`.
