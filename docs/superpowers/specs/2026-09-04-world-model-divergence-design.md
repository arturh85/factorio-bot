# The planner does not know how much fits in a slot

**Status:** design only. Nothing below is built. No production code was written
for this document. Every claim is read off the source, measured from
`workspace/runs/run-1788517971-48257/`, reproduced offline with
`factorio-bot plan --world workspace/scripts/map.json`, or measured directly
against a live Factorio 2.1.17 instance over `factorio-bot rcon`.

**The short version.** `tried to remove 141 iron-plate but removed 100` is
neither staleness nor double-counting. **A stone furnace's output is one
inventory slot and holds exactly one stack**, and `iron-plate` stacks to 100, so
141 was never physically possible — at plan time, at dispatch time, or at any
moment in between. The planner has no notion of slot capacity anywhere: it sizes
every transfer from *demand*. Four distinct surfaces over-promise, and all four
can be produced offline from the committed baseline map. The failure is not
detected late; it is **created** at expansion.

Recommendation, in one line: **teach the planner one function —
`slot_capacity(slot, item)` — then fix the four sizing sites behind it, cheapest
first (`produce::PlaceDrill`'s take, which is what stops green science), and add
a plan-time refusal only once all four are sized so it can never fire.** The
smallest useful change is about half a day; the whole family is about a week and
is decomposed in §9. **Red does not move** — and not because it was checked once,
but because every transfer red plans is already inside the caps (§6).

---

## 0. Corrections to the brief

Every agent on this work has found something wrong in its brief. Five here.

1. **"Four of five action failures in that run were this class."** They carry the
   same *message* and they are **two different defects**. One of the five —
   `take 141 iron-plate from the cell`, asked 141, got 100 — is the capacity
   defect. The other three (`take 2 iron-plate`→0, `take 3 copper-plate`→0,
   `take 2 copper-plate`→0) ask counts nowhere near any cap and get **zero**:
   those are genuine model-versus-game divergence, where the machine had not
   produced yet. Fixing capacity fixes one of the four. §8 says what the other
   three need, and it is not the same change.

2. **"Is it staleness or accounting?"** Neither. It is a missing physical
   constraint. The furnace at `(-7, -31)` never held more than 100 iron-plate at
   any point in its recorded life (596 machine samples), and at plan time it did
   not exist at all — it is placed *by the same plan* that plans to empty it.
   That last point is decisive for the design (§5.1): there is nothing to ask the
   game about, so no amount of divergence detection could have caught this.

3. **"The same number, 141, in both failures" (with `3b79eb20`).** Coincidence.
   `3b79eb20` was two claims on one bot's stock; here 141 is
   `Demand::need` for a `Have iron-plate` share, and nothing in the world held
   141 of anything. Independently: at plan time the *entire world* held 58
   iron-plate across every container and every bot.

4. **`get_insertable_count` is not a usable oracle for input capacity.** The
   brief and the follow-up both suggest asking the game. Measured on the bench,
   on one entity in one call: `get_insertable_count("iron-ore")` answers **50**
   while `insert{count=1000}` accepts **70**. It reports the stack size, not the
   capacity. Anything built on it would under-fill by 29%.

5. **`crates/planner/src/method/assemble.rs:214` says "the mod does not send
   `fuel_value` or `stack_size`", and that is false.** `serialize_item_prototype`
   (`mods/BotBridge/types.lua:89`) sends both, and
   `workspace/scripts/map.json` carries them for all 342 item prototypes —
   `iron-plate` 100, `coal` 50 with `fuel_value` 4,000,000. Two constants
   (`COAL_KJ`, `COAL_STACK`) are hardcoded on the strength of that claim. This
   matters here because the whole fix depends on `stack_size` being readable, and
   **it is**.

---

## 1. What actually happened, from the record

`run-1788517971-48257`, seed 31337, milestone 3 (green science).

The plan created at tick 73,454 (402 steps) contains

```json
{"id":73,"bot":1,"action":"take 141 iron-plate from the cell",
 "deps":[70,71,72],"planned_start":113518,"planned_duration":10}
```

The cell's furnace (unit id 1000, `stone-furnace` at `(-7, -31)`) was placed at
tick 81,851 — **8,397 ticks after the plan that plans to empty it was written.**
`samples.jsonl` then tells the whole story on its 300-tick beat:

```
(176700, 'working',      93,  0)
(178200, 'no_ingredients', 100,  0)
(178500, 'full_output',  100,  1)
(179100, 'full_output',  100,  3)
...
(187800, 'full_output',  100, 39)     <- 9,600 ticks of this
(188100, 'working',        1, 40)     <- the take fired at 188,081
```

The furnace reached exactly 100 iron-plate, went to **`full_output`, and stopped
smelting for 9,600 ticks** while its input backed up from 1 to 39 ore. The take
was dispatched at 188,081, removed the 100 that were there, and was judged a
failure. Nothing else ever touched that container: every one of the nine actions
dispatched against `(-7, -31)` across the whole run belonged to bot 1.

Two costs, and the second is the larger one:

* the action failed, its dependants were abandoned, and milestone 3 burned an
  iteration;
* **the cell was dead for 9,600 ticks before that**, with a bot idle beside it
  (the plan's own listing shows 36,240 ticks of idle in front of this take), and
  32 ore stranded in a slot the furnace could not empty.

The items were not lost — `rcon_remove_from_inventory` inserts what it removed
into the player and *then* complains, so bot 1's inventory went 190 → 290 at that
tick. The plan simply does not know that.

## 2. Why 141 was impossible: the capacity table, measured

Measured on a live 2.1.17 instance, entities created and destroyed in the same
command, via `factorio-bot rcon -s localhost -- '/c ...'`. Note the 2.0 define
names: `crafter_input` / `crafter_output`; `furnace_source` and `furnace_result`
do not exist and return nothing at all rather than erroring.

| slot | rule | measurements |
| --- | --- | --- |
| `crafter_output` | **1 slot, exactly `stack_size`** | stone-furnace/iron-plate **100**; assembling-machine-1/iron-gear-wheel **100**; assembling-machine-1/electronic-circuit **200** |
| `crafter_input` | **`stack_size + 20`**, per ingredient | iron-ore **70**, copper-ore **70**, stone **70** (stack 50); iron-plate **120** (stack 100); copper-cable **220** (stack 200); solid-fuel **70**; low-density-structure **70** |
| `fuel` | **1 slot, exactly `stack_size`** | stone-furnace/coal **50**; burner-mining-drill/coal **50** |
| `lab_input` | 12 slots, but **one stack per science type** | lab/automation-science-pack **200** (stack 200) |
| `chest` | `slots × stack_size` | wooden-chest **1600** (16×100); iron-chest **3200** (32×100) |

Four things this establishes:

* **The output rule tracks the item, not a constant** — 100 for gears, 200 for
  circuits in the same machine. So it is `stack_size`, read per item.
* **Capacity and the production-stop threshold are not the same number, and
  only for a furnace do they coincide.** The table above is what an inventory
  *accepts*. What makes a machine *stop* is a separate question, and the answer
  differs by machine type: a stone furnace stops at a full stack (the run's own
  samples show it sitting at exactly 100 with `status: full_output`), while an
  assembling-machine-1 stops at **3–4 items** — §7 has the measurement. So
  `stack_size` is the right bound for a furnace batch and is **not** a bound
  anyone may generalise to an assembler.
* **The input margin is additive, not proportional** — 50→70, 100→120, 200→220.
  Proportional would have given 240 for a stack of 200. It is the same +20 across
  every machine, every recipe and every ingredient amount tested. It is an
  empirical constant of this game version and should be named as one.
* **`get_insertable_count` disagrees with `insert` on the same entity in the same
  call** (50 vs 70). Use it and you under-fill.

The one capacity that is **not** derivable from what the planner has:
`entity_prototypes` carries no inventory size (`workspace/scripts/map.json`'s
`wooden-chest` entry has `collision_box`, `mine_result`, `mining_time` and
nothing about slots). Chests are the only many-slot case, and the only one the
planner cannot compute — see §5.3. Everything that currently matters is one slot,
so `stack_size` alone is enough.

## 3. Where the plan over-promises: four surfaces, all reproducible offline

Every line below comes from `factorio-bot plan --world workspace/scripts/map.json
--steps`, with no Factorio running. `8d07af4b` is not needed for any of it: these
counts are sized from *demand*, not from an observation, so an empty
`inventories` map changes nothing.

**(a) `produce::PlaceDrill`'s cell take** — `crates/planner/src/method/produce.rs:1310`,
`count: need`, one cell (`plan_cells(..., 1)`), one `Remove` for the whole goal.

```
--goal producing:logistic-science-pack:6 --bots 1,2,3,4
  134635  134645  10  idle 36240  #276 take 150 iron-plate from the cell
```

150 > 100. This is the live failure, on the baseline map, at the current HEAD.
It scales with the goal, without bound. With `--bots 1`:
`have:iron-plate:300` → `take 300 from the cell`; `have:iron-plate:600` →
`take 600`; `have:steel-plate:150` → `take 750 iron-plate from the cell`, behind
**180,240 ticks of idle**. With `--bots 1,2,3,4` the shares are smaller and each
bot gets its own over-cap cell: `have:iron-plate:600` gives four `take 150`s.

**(b) `have::smelt_steps`'s bank take** — `crates/planner/src/method/have.rs:1898`,
`count: furnace_runs * per_craft`, capped only by `need_left`.

```
--goal have:steel-plate:150 --bots 1
  464551  464561  10  idle 144960  #3 take 150 steel-plate from the furnace
```

150 > 100. It does not fire in green science because the large quantities are
claimed by `PlaceDrill`, which is registered ahead of `Smelt` — green's largest
bank take is 40. The bug is no less real: `bank_size`'s `widest` is
`runs.min(MAX_BANK).min(standing)`, so with no standing furnace `k` is 1 and one
furnace carries the whole goal however large it is.

**(c) The ore/ingredient insert** — `have.rs:1553` and `have.rs:1706`,
`count: ore_per_run * runs`, dealt into `room[index]` which is the same
unbounded number.

```
  319581  319591  10  idle 0  #1 insert 750 iron-plate
```

750 into a slot that takes 120.

**(d) Every fuel insert** — `have.rs:1400`, `produce.rs:820`, `produce.rs:1275`,
`produce.rs:1876`, `power.rs:867`, `assemble.rs:1576`, `assemble.rs:2698`;
`fuel_for_duration(duration, BURN_TICKS)` with no cap.

```
   56371   56381  10  idle 0  #2   fuel the furnace with 55 coal
   67313   67323  10  idle 0  #125 fuel the burner-mining-drill with 113 coal
   67323   67333  10  idle 0  #126 fuel the stone-furnace with 68 coal
```

A fuel slot holds 50. A full slot of coal is `50 × COAL_BURN_TICKS` = 133,300
ticks of stone furnace, so **any job longer than that needs a second visit**, and
the plan has no way to say so. This is the same family as the archive's
`["tried to insert 17x coal but inserted 3"]`, which is the *occupancy* version
of it (the slot already held 47).

Two surfaces are already safe and should stay that way: `have::Withdraw`
(`have.rs:2128`) sizes each take from an observed buffer's own contents, and
`have::Stockpile` (`have.rs:4689`) from its own deposits. Neither can exceed what
the container holds; only a chest's 16-stack ceiling bounds them, and nothing
comes near it.

## 4. The mechanism, stated once

> **The planner models every machine inventory as unbounded.** It sizes a load,
> a batch and a take from what the *goal* needs, and the game sizes them from
> what a *slot* holds. Where the two disagree the game wins, silently on the way
> in (`full_output` stops the machine) and loudly on the way out (`removed N of
> M`).

The corollary is the design's whole content: **a long production job is a
sequence of visits, not one visit.** One load, one wait, one take is only correct
while the job fits in a slot. Beyond that the plan needs either more machines or
more trips, and today it emits neither.

## 5. Options, with costs

### 5.1 Where capacity has to come from — settled, not a choice

**Prototypes, not the game.** The furnace in the failing run *did not exist* when
the plan was written; `PlaceDrill` places it and empties it in the same
expansion. There is no entity to ask `inventory_contents_at` or
`get_insertable_count` about. The same is true of every furnace `smelt_steps`
sites and every drill `plan_cells` sites. A capacity query is only answerable for
machines that already stand, which is precisely the case that is *already* safe
(`Withdraw`, sized from an observation).

`FactorioWorld::item_prototypes` carries `stack_size` for every item and reaches
the planner as `PlanState::base`. That is the whole input the rule needs. Cost:
one accessor.

The fallback matters. `crates/planner`'s fixtures build worlds with no item
prototypes, and several tests are pinned byte-for-byte. `stack_size()` must
therefore answer `Option<u32>` and an unknown item must mean **"no cap"**, not a
guessed one — with a `warn!`, because a silent uncapped path is exactly the
failure mode this whole document is about. On the production path the prototypes
are always there.

### 5.2 What to do when a job does not fit — four shapes

**Option A — more visits to one machine.** Emit `ceil(need / cap)` load/wait/take
cycles against the same machine, each with its own lag edge. Costs one transfer
action per stack and no materials. It *saves* time: the `full_output` stall
disappears, which was 9,600 of 36,240 idle ticks in the failing run — 26% of the
wait was the machine standing dead because the plan would not come and empty it.
`smelt_steps` already emits one take per furnace with a per-furnace lag, so the
shape exists; this is the same shape indexed by cycle instead of by machine.

**Option B — more machines.** Require `ceil(need / cap)` machines and split the
runs across them. For `smelt_steps` this is `bank_size`, which already exists:
`widest` becomes `max(widest, need.div_ceil(cap))` and `MAX_BANK` bounds it at 8
(800 plates), beyond which A is needed anyway. For `PlaceDrill` it is
`plan_cells(..., ceil(need/cap))`. Costs a drill and a furnace per extra cell
plus the ground, and buys parallelism the current design deliberately refuses to
pay for — `bank_size`'s own measurement says building a furnace for the lag alone
*loses*, because the lag was already absorbed by other work. Correctness is a
different argument from that one, but the measurement is a warning: B is not free
even when it looks like it is filling idle time.

**Option C — refuse above the cap and let another method take it.**
`PlaceDrill::applicable` returns false when `need > cap`. One line. It does not
work: `Smelt` has defect (b) and (c), so the goal lands on an equally broken
path. Useful only as a temporary guard *after* (b) and (c) are fixed, and by then
it is unnecessary.

**Option D — make the take tolerant.** Teach `ActionKind::Remove` an
`at_most: true` form and have the mod stop complaining on a short remove. This is
the option to be most careful with. It would make the *symptom* vanish for
divergence of every kind, including the kind that means the model is wrong — and
this repository has four separate recorded instances of a mechanism reporting
nothing while broken. The mod's complaint is what makes a short withdrawal
visible at all (`2026-09-03-buffers-are-visible.md` §3 verifies exactly that
chain end to end). Rejected as a default. It is defensible as an *explicit*
per-action flag on takes the planner has deliberately sized as "whatever is
there", and nowhere else.

**Option E — detect divergence before dispatching.** A pre-flight query, in the
shape `plan_verified` already uses for `can_place_entities`: ask the game about
every `Remove` the schedule contains, compare with the plan, re-expand on a
mismatch. One RCON round trip per plan. It cannot help here (§5.1: the container
does not exist yet), and §8 is where it does help.

### 5.3 What stays unfixed, deliberately

Chest capacity (`slots × stack_size`) cannot be computed: `entity_prototypes`
carries no inventory size, and adding one means a mod change, a `types.rs` field,
a `types.lua` regeneration and a snapshot bump. A wooden chest holds 1600
iron-plate and the largest chest transfer in any plan measured here is 37. Not
now — but if `iron-chest` is ever added to `BUFFER_ENTITIES` (see
`Planner::BUFFER_ENTITIES`' own note about stage 2), this becomes reachable.

`LabInput` is one stack per science type (200), and research inserts are
currently ≤75. Latent, same treatment.

## 6. Recommendation

Two layers, in this order. **The guard goes last, not first** — that ordering is
deliberate and it is the opposite of the instinct.

**Layer 1 — one accessor and four sizing fixes.**

```rust
// crates/planner/src/state.rs
impl PlanState {
    /// The item's stack size, or `None` when the world has no prototype for it.
    pub fn stack_size(&self, item: &str) -> Option<u32>;

    /// The most of `item` that one machine's `slot` can hold at once.
    /// `None` means "unknown, therefore unbounded" — fixtures only.
    pub fn slot_capacity(&self, slot: InventorySlot, item: &str) -> Option<u32>;
}
```

with the rule table from §2: outputs and `Fuel` and `LabInput` are `stack_size`,
inputs are `stack_size + INPUT_OVERLOAD` (measured at 20, named as a version
constant beside `COAL_BURN_TICKS`, with the measurements in its doc), `Chest`
is `None` until §5.3 is done.

Then, cheapest first:

1. **`produce::PlaceDrill`'s take** (Option A). Split into stack-sized takes with
   staggered lags. This is what unblocks green science, it is one function, and
   red cannot move (below).
2. **The fuel inserts** (Option A). Cap at one stack; where the job outlasts a
   slot, emit a refuel visit. Seven call sites, one helper.
3. **`smelt_steps`'s batch** (Option B then A). Bound runs per furnace-load by
   `min(out_cap / per_craft, in_cap / ore_per_run)` — note **both** bind, and the
   input is the tighter one for iron: capping output at 100 plates still asks for
   100 ore into a slot that takes 70. Widen the bank first (the machinery is
   there), then cycle.
4. **`PlaceDrill::applicable`'s cost model.** Splitting adds
   `ceil(need/cap) - 1` transfers to the cell's bot-tick cost, which moves the
   crossover against hand-smelting. Consistency, not correctness — but leaving it
   means the gate prices a plan the method no longer emits.

**Layer 2 — a plan-time refusal, once nothing can trip it.** A new
`PlannerError::SlotOverflow { slot, item, required, capacity }` raised where
`PlannerError::BufferShort` is raised today (`PlanState::take_from_buffer`,
`state.rs:1735`), checked over the network in `schedule`. `BufferShort` is the
exact precedent and its doc states the principle: *"a plan that got the
arithmetic wrong fails at the planner rather than four minutes later on a bot
that has walked there."* This one is stronger — it fails **offline, with no game
at all**, which is why every line of evidence in §3 exists.

It must land last because between now and then it would turn "green science
fails after 50 minutes" into "green science will not plan", and that is not
progress. Landed after the four fixes, it is a permanent regression test: any
future method that sizes a transfer from demand fails a `cargo test`.

**Red does not move, and here is the reason rather than a fingerprint.**
`score-map`'s pin is `researched:automation` (`DEFAULT_GOAL`,
`app/src-tauri/src/cli/score_map.rs:63`) — **not**
`producing:automation-science-pack:6`, which is a different, larger plan (369
actions / 47,330 ticks against 202 / 30,085). Both were listed at HEAD
`ffabf393`; the largest transfer of each kind, against its cap:

| | `researched:automation` (the pin) | `producing:automation-science-pack:6` | cap |
| --- | ---: | ---: | ---: |
| take from a machine output | 50 | 50 | 100 |
| insert into a machine input | 10 | 10 | 70–120 |
| fuel | 8 | 8 | 50 |
| take from a chest | 13 | 13 | 1600 |

Every one is inside its cap, so a cap is a no-op on red **by construction** and
the sizing changes have nothing to bite on. `score-map --world
workspace/scripts/map.json --bots 1,2,3,4` was re-run at HEAD `ffabf393` and
reproduces `actions 202, makespan 30085, utilisation 36.4%`. If an implementation
moves it, the cause is a bug in the implementation and not the design; **do not
update the pin.**

Green *does* move, and should: `take 150` becomes `take 100` + `take 50`, and the
9,600-tick `full_output` stall disappears.

## 7. A larger finding in the same family, found by accident

I went looking for `full_output` across the whole run to size §10's fourth
unknown, and found something I was not looking for. Every machine sample in
`run-1788517971-48257`, by status:

```
no_ingredients    7577  31.2%
normal            5982  24.6%
no_fuel           5921  24.4%
working           2320   9.5%
full_output       1281   5.3%
...
```

**1,249 of the 1,281 `full_output` samples belong to two machines**, and neither
is the furnace this document is about:

```
assembling-machine-1 (35.5, -27.5)  iron-gear-wheel          627 samples full_output
assembling-machine-1 (35.5, -23.5)  automation-science-pack  622 samples full_output
stone-furnace        (-7.0, -31.0)  iron-plate                32 samples full_output
```

Those two are the stage-2 red-science cell (`method::assemble::plan_cell`). The
science-pack assembler first reported `full_output` at tick **74,100** holding
**four** packs, never held more than four, and finished **four crafts in the
entire run** (`products_finished: 4`, still 4 at the last sample, tick 260,400).
The gear assembler jammed the same way at three gears.

The mechanism is visible in `map.jsonl`'s placements. The cell has three
inserters — two feeding the assemblers from their supply chests
(`(33.5,-23.5)` and `(33.5,-27.5)`, direction 12) and one handing gears from the
first assembler to the second (`(35.5,-25.5)`, direction 0). **There is no
inserter on the science assembler's output**, which is exactly what
`Planner::BUFFER_ENTITIES`' own doc says by design: *"the cell's product never
enters a chest at all, it sits in the assembling machine's output slot"*. That is
fine for one stack of anything; it is not fine when the machine stops at four.

So a "cell producing 6/min" produced four packs and stopped 1,500 ticks after it
started, and milestone 1 was recorded `milestone_satisfied` anyway, because the
supervisor witnesses the standing structure rather than the output. This is the
`cells_standing` shape again — a model-versus-game disagreement nobody had a
reason to look at.

**I am not designing the fix here.** It is `crates/planner/src/method/assemble.rs`
and it is a cell-design change (an output inserter and somewhere to put the
product), not a sizing one. But it belongs in this document because it is the
same sentence: the planner models machine inventories as unbounded, and the
game does not. It is plausibly worth more than everything else here — the
capacity defect costs one milestone an iteration, this one means stage 2 has
never actually produced at a rate.

Two things to confirm before acting on it: what exactly makes an
assembling-machine-1 stop at 3–4 (measure the threshold against recipe product
count and crafting speed, the way §2 measured the input margin), and whether any
other archived run's cell ever exceeded it.

## 8. The other three failures, which this does not fix

`take 2 iron-plate`→0, `take 3 copper-plate`→0, `take 2 copper-plate`→0. Small
counts, empty machines, ordinary furnaces (not cells). These are the timing half
of the same message: the take fired before the furnace had produced, or the
furnace was starved of ore or fuel. `smelt_steps`' own comment records the shape
("a removal at insert+1924 against a modelled 1920 came back with nine plates out
of ten"), and it gives the take one craft cycle of headroom — evidently not
always enough.

Capacity cannot help them, and nor can more headroom in general: the honest
answer is that the plan does not know when a machine is done. Three responses,
and they are complementary:

* **`divergence_observed` in `recover.rs`**, exactly as
  `2026-09-04-recovery-instead-of-replan-design.md` §5.2 scoped it (~30 lines):
  escalate a `PartialTransfer`/`MissingItem` on the *first* failure rather than
  burning three identical retries. Its ordering advice — after S1 ships, so the
  measurement is not confounded — **stands, and this document does not override
  it.**
* **A retry with a wait** is the one tier that would actually fix these: the
  machine was going to produce, just later. That is a new tier, not a
  reclassification, and it needs the executor.
* **Bookkeeping for a partial take.** The mod moves what it could *and then*
  reports failure, so the bot really is holding the plates. Today the whole
  dependent subtree is abandoned and a replan re-reads the world, which recovers
  the material but discards the plan. A recovery that reads
  `failure.detail`'s `moved 100 of 141` and re-plans only the shortfall is
  strictly cheaper. This is `recover.rs` and `crates/core/src/record/` — both
  currently occupied by another agent — so it is named, not designed here.

**A periodic reconciliation of `EntityGraph` against the game** (the brief's
question 4) is worth its cost, but not for this bug and not first. The cost is
one RCON round trip on a beat and a diff; the precedent is `plan_verified`'s
`can_place_entities` pre-check, which is the same shape and already earns its
keep. The case for it is `cells_standing` — a predicate that read `None` where the
game held a recipe, and produced *three* silently ignored cells over months. The
right home is beside the buffer refresh in `Planner`, as
`Planner::reconcile()`, answering a list of disagreements rather than mutating
anything, so a caller decides what a disagreement means. Half a day for the
query, and the real work is choosing what to compare. **It would not have caught
this bug**, and saying so is the point: a divergence detector finds models that
have drifted, not models that were never possible.

§7 is the stronger case for it, and it also says what to compare first:
`LuaEntity.status`. A cell the plan believes is producing 6/min, sitting in
`full_output` on every beat for 186,000 ticks, is one field read away from being
noticed — and the sampler is *already reading it*, into `samples.jsonl`, where
nobody looks until a run is over.

## 9. Scale, and what to do first

**The smallest thing worth doing, on its own:** fix `PlaceDrill`'s take (§6
layer 1, item 1). It is one function in `produce.rs`, it needs `stack_size()`,
it unblocks the current milestone, red cannot move, and it is provable offline —
`plan --goal producing:logistic-science-pack:6 --steps` shows `take 150` before
and `take 100` + `take 50` after, with no game running. **Half a day including
tests.**

The whole family, decomposed:

1. `stack_size()` / `slot_capacity()` plus the `INPUT_OVERLOAD` constant with its
   measurements written into the doc comment — **0.5 d**. Touches
   `crates/planner/src/state.rs` only.
2. `PlaceDrill`'s take split, and the `applicable` cost model that goes with it —
   **0.5–1 d**. `produce.rs`.
3. Fuel capping and refuel visits across seven call sites — **1 d**. Touches
   `have.rs`, `produce.rs`, `power.rs`, `assemble.rs`; the last two are the
   power plant and the stage-2 cells, so this is the change most likely to move
   a pinned fixture.
4. `smelt_steps`: bank widening plus load/take cycling, with both the input and
   the output bound — **1.5–2 d**. This is the hard one: it interacts with
   `drains`, `queue_machine`, `bank_coal` and the R3 handover, all of which
   assume one load per furnace per smelt.
5. `PlannerError::SlotOverflow` and the `schedule` check — **0.5 d**, last.
6. One measured run at `--seed 31337` to establish that milestone 3 gets past the
   take — **half a day of wall clock**, and it is the only thing that can confirm
   §8's three remaining failures are what is left.

Call it **a week**, and 1+2 alone (one day) is the part that unblocks green.

## 10. What could not be determined without a run

* **Whether milestone 3 completes once the take is split.** The other three
  failures in that run are untouched by this, and one of them (`craft 75
  automation-science-pack` timing out) is not a transfer at all.
* **Whether the input `+20` margin holds for a machine that is mid-craft.** Every
  measurement above is on a freshly created, empty machine. If the margin is
  really "one stack plus one craft's worth of overload", a running machine may
  answer differently. The design's use of it is conservative — it *reduces* a
  batch — so being wrong costs an extra trip, not a failure.
* **Whether `full_output` accounts for more idle than the failures do.** The one
  case measured here is 9,600 ticks on a 36,240-tick wait. `just analyse` reports
  per-machine status; nobody has summed `full_output` across a run.
* **Whether the fuel cap forces refuel visits often enough to matter.** It
  binds at 133,300 ticks of furnace runtime, which the steel-plate probe exceeds
  and green science does not.

## Appendix — how to reproduce every claim, without a game

```bash
# The failure, offline, on the committed baseline map:
factorio-bot plan --world workspace/scripts/map.json \
  --goal producing:logistic-science-pack:6 --bots 1,2,3,4 --steps \
  | grep "from the cell"
#   134635  134645  10  idle 36240  #276 take 150 iron-plate from the cell

# All four surfaces at once:
factorio-bot plan --world workspace/scripts/map.json \
  --goal have:steel-plate:150 --bots 1 --steps
#   #125 fuel the burner-mining-drill with 113 coal   (slot holds 50)
#   #126 fuel the stone-furnace with 68 coal          (slot holds 50)
#   #127 take 750 iron-plate from the cell            (slot holds 100)
#   #1   insert 750 iron-plate                        (slot holds 120)
#   #3   take 150 steel-plate from the furnace        (slot holds 100)

# Red, unchanged. score-map's default goal is `researched:automation`; this is
# the same plan, listed, so its transfer sizes can be checked against §2:
factorio-bot score-map --world workspace/scripts/map.json --bots 1,2,3,4
#   actions 202, makespan 30085, utilisation 36.4%
factorio-bot plan --world workspace/scripts/map.json \
  --goal researched:automation --bots 1,2,3,4 --steps
#   actions 202, makespan 30085 — max take 50, max insert 10, max fuel 8
```

The capacity measurements come from `factorio-bot rcon -s localhost -- '/c ...'`
against a scratch instance, creating each entity, inserting 1000–5000 of the item
into the named `defines.inventory` slot, recording what was accepted, and
destroying the entity in the same command. The stack sizes are read from
`workspace/scripts/map.json`'s own `item_prototypes`, which is what the planner
would read.
