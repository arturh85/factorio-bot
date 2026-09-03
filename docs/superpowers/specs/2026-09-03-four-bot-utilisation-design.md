# Getting `researched("automation")` under nine minutes with four bots

**Status**: design. Nothing here is implemented. Every number is measured from an
archived run with `tools/run_analysis.py` or read out of the event log directly;
where a number is derived rather than measured it says so and shows the
arithmetic.

**Headline**: the milestone is 74,153 ticks (20.6 min). **39.1% of it is bot 1
standing next to a furnace waiting for plates.** Fixing bot utilisation alone
lands at ~13.8 min. Fixing smelting alone lands at ~14.2 min. Doing both lands
at ~7.4 min. Neither is sufficient; both together have ~20% margin against the
nine-minute target. The floor with one lab is ~5.5–6.0 min, of which 5,999 ticks
(1.67 min) is the research reaction itself and cannot be touched.

Reference run: `workspace/runs/run-1788459085-32452`, milestone 1, ticks
4,732 → 78,885.

---

## 1. Where the 20.6 minutes goes

### 1.1 The whole milestone is one bot's timeline

Bot 1 is on the critical path for every tick of milestone 1. That is not an
inference — it is a partition. Take bot 1's dispatch→settle intervals for
actions and walks, merge the overlaps, and the busy time plus the gaps sums
exactly to the span:

| bucket | ticks | % of milestone |
| --- | ---: | ---: |
| `mine` (bot 1's own) | 19,083 | 25.7% |
| walking (bot 1) | 12,851 | 17.3% |
| `craft` (bot 1) | 6,162 | 8.3% |
| `research` (the lab reaction) | 5,999 | 8.1% |
| **idle — waiting on a furnace or a cell** | **28,987** | **39.1%** |
| idle — dispatch overhead and other (158 gaps) | 1,071 | 1.4% |
| **total** | **74,153** | **100%** |

`place`, `insert`, `take` and `fuel` settle in the tick they dispatch and
contribute zero. That is why a verb histogram cannot see any of this — the
brief's warning about the "88% is hand-mining" error applies to walking *and* to
the 39% that is a wait rather than an action at all.

Everything the other three bots did in the whole milestone:

| bot | dispatches | act ticks | walks | walk ticks | busy% |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 132 | 31,244 | 36 | 12,851 | 59.5% |
| 2 | 4 | 1,691 | 2 | 435 | 2.9% |
| 3 | 4 | 1,691 | 3 | 614 | 3.1% |
| 4 | 4 | 1,693 | 3 | 591 | 3.1% |

Fleet utilisation 50,810 / 296,612 bot-ticks = **17.1%**. Zero failed walks —
the enclosure and walk-refusal work landed and is not the problem any more.

### 1.2 The 28,987 idle ticks, itemised

Every one of them is a single wait between loading a furnace and taking the
plates back out:

| idle ticks | action it was waiting for | model says |
| ---: | --- | ---: |
| 12,244 | `take 50 iron-plate from the cell` | 12,240 (`240 × 51`) |
| 4,036 | `take 20 iron-plate from the furnace` | 4,032 (`192 × 21`) |
| 3,269 | `take 16 iron-plate from the furnace` | 3,264 (`192 × 17`) |
| 2,118 | `take 10 copper-plate from the furnace` | 2,112 (`192 × 11`) |
| 2,116 | `take 10 copper-plate from the furnace` | 2,112 |
| 1,732 | `take 8 iron-plate from the furnace` | 1,728 (`192 × 9`) |
| 1,156 | `take 5 iron-plate from the furnace` | 1,152 (`192 × 6`) |
| 1,156 | `take 5 copper-plate from the furnace` | 1,152 |
| 772 | `take 3 iron-plate from the furnace` | 768 (`192 × 4`) |
| 388 | `take 1 copper-plate from the furnace` | 384 (`192 × 2`) |
| **28,987** | | |

**The nine `Smelt` waits total 16,743 measured against 16,704 modelled — the
model is exact to 0.2%.** This is not a bug in the model. It is the model
working correctly and describing a plan that puts `runs × smelting_ticks` of one
stone furnace on the critical path, nine times over.

`crates/planner/src/method/have.rs:790-828`:

```rust
let per_run = smelting_ticks(&ctx.state, &recipe, &furnace_entity);
let smelt_lag = per_run.saturating_mul(runs).saturating_add(per_run);
for id in insert_ids {
    let lag = if id == fuel_id { 0 } else { smelt_lag };
    steps.push(Step::Link { from: id, to: remove_id, lag });
}
```

One furnace, `runs` cycles, serially. A stone furnace smelts one iron plate in
3.2 s = 192 ticks. Twenty plates is 4,032 ticks whatever the roster size. The
comment above the lag says "the bot is free to do other work across this lag —
that is what it is for." **Measured, it was free to do other work for 39 of
those 16,743 ticks.** There is no other work, because §1.4.

### 1.3 The one outlier: the drill cell

`take 50 iron-plate from the cell` is the only wait the model gets wrong, and it
is the largest single block in the milestone. `PlaceDrill`
(`crates/planner/src/method/produce.rs:1008+`) builds one burner-mining-drill
feeding one stone furnace and links the take with
`lag = spec.ticks_per_item * (need + 1)` = `240 × 51` = 12,240 ticks
(`produce.rs:1247`). The drill is the bottleneck at 240 ticks/ore against the
furnace's 192 ticks/plate, which `cell_spec` models correctly.

Measured: fuel at tick 26,700, take at tick 55,742 — **29,042 ticks, 2.37× the
model.** 16,798 of that overlapped other bot-1 work; the residual 12,244 landed
as pure idle. The overrun's cause is not established. Candidates, in the order I
would test them: the drill's eight coal buy `8 × 1,600 = 12,800` ticks of burn
(`fuel_for_duration(12240, DRILL_BURN_TICKS)`), so a cell sized to finish
exactly as its fuel runs out stalls dead if anything runs a few percent slow;
the drill was placed at tick 26,696 on a patch the same plan had been
hand-mining; the mod's real drill throughput may differ from the prototype
figure. **This is a measurement to take, not a conclusion to act on** — the
recommendation in §4 does not depend on which it is.

What *does* matter is the comparison the method makes to choose the cell at all
(`produce.rs:1022-1035`):

```rust
cell_setup_bot_ticks(state, &spec, need) < hand_smelt_bot_ticks(state, &spec, need)
```

Both sides are **bot-busy ticks**, and the doc says so in place: "once fuelled
the bot walks away, and the drill mines unattended… So the comparison is
*bot-busy* ticks, not wall-clock ticks." For 50 plates the commit message
records 6,540 (hand) vs 5,928 (cell), so the cell wins by 9%.

That is the right objective when a bot's time is scarce. **In this run three
bots were 97% idle and the winning bot then stood still for 12,244 ticks.** The
objective is inverted for the situation we are actually in.

### 1.4 Why bot 1 has nothing to overlap the waits with

`Researched::expand` (`have.rs:1660-1974`) states all three of its subtrees —
the trigger item, the lab, and the science-pack bill — as
`Holder::Share(ctx.chain_actor)`:

```rust
// have.rs ~1878, comment above it
// `Holder::Share`, not `Holder::Anyone`. The research is one action reading
// one bot's inventory, so the packs have to end up in one inventory...
steps.push(Step::Subgoal(Goal::Have {
    item: item.clone(), count: *count, whose: Holder::Share(ctx.chain_actor),
}));
```

`method/mod.rs:675-698` turns a stated `Holder::Share(bot)` into a chain owner:

```rust
let owner = match stated_holder(goal) {
    Some(Holder::Bot(bot) | Holder::Share(bot)) => Some(*bot),
    _ => None,
};
```

`schedule.rs:350` turns an owner into a single-candidate tier with no fallback:

```rust
None if owner.is_some() => vec![vec![owner.expect("just checked")]],
```

with the comment at `:332-349` making it explicit — "A chain owner names a
specific bot before scheduling begins, so it is a hard constraint: no tier falls
back past it."

`SplitAcrossBots` cannot reach any of it. `have.rs:2070`:

```rust
fn claims(&self, site: GoalSite) -> bool { site.top_level && !site.in_chain }
```

and `applicable` additionally requires `matches!(whose, Holder::Anyone)`. The
whole `Researched` subtree is neither top-level nor `Anyone`.

**This is deliberate and load-bearing.** `goal.rs`'s `Holder::Share` doc records
why, including the crash the brief cites, and prices the binding at "22,072
ticks on a live four-bot run". The design document that argued the unbound
version was safe (`2026-09-01-per-bot-share-sizing-design.md`) carries a
correction at its own head saying its §4 safety argument is inverted. Any
proposal that separates sizing from binding walks straight back into that crash.

### 1.5 Where the walking goes

Bot 1's 36 walks in milestone 1 average 357 ticks. The destinations cluster into
five sites on this map:

| site | approx. position | bot-1 visits in m1 |
| --- | --- | ---: |
| base / iron | (−31, −27) | 9 |
| stone | (−57, 13) | 8 |
| coal | (−25, 49) | 6 |
| copper | (26, 20) | 6 |
| water / power plant | (−8, −55) | 3 |

Bot 1 ping-pongs between four resource patches ~50–80 tiles apart, twenty-six
times. Bots 2–4 make two or three walks each, all to the iron patch. **Stone,
coal and copper mining — 65 + 30 + 26 = 121 units, 14,613 ticks — is 100% on bot
1, and so is every trip to reach them.**

---

## 2. Corrections to the brief

Three of the brief's premises do not survive the data. Two of them change the
plan.

**2.1 "The measured blocker is that the bots are never assigned work."** Half
right, and it is the *smaller* half. Giving bots 2–4 work removes ticks from
bot 1's *work* buckets (19,083 mine + 12,851 walk). It cannot touch the 28,987
idle ticks, because those are a wait on bot 1's own chain: shortening the queue
in front of a furnace does not make the furnace faster. Utilisation fixes alone
project to 49,550 ticks = **13.8 min**. This is the single most important
correction: the brief's diagnosis is correct as a diagnosis and insufficient as
a plan.

**2.2 "20.1 min — an archived single-bot run. So 4 bots are now at parity with
1 bot."** This comparison is invalid; the two runs are on **different maps**.
Bucketing every walk destination to a 20-tile grid:

```
run-1788449752-46541 (4-bot, 27.1m):  (-60,20) (-40,-20) (20,20) (-20,40) ...
run-1788459085-32452 (4-bot, 20.6m):  (-40,-20) (-60,20) (20,20) (-20,40) ...
run-1788401146-98497 (1-bot, 20.1m):  (0,40) (-20,40) (20,-40) (100,0) ...
```

The two four-bot runs share a map exactly. The solo run does not — it has a
resource 100 tiles east, a site absent from the other two entirely. The
defensible statement is the one that controls for the map: **27.1 min → 20.6 min
on the same map, same roster, is a real 24% improvement from today's five
commits.** The solo run is evidence about a different world and should be
retired from the comparison.

(The solo run is still useful as *structure*: its idle profile is the same
shape — 24,613 of its 25,497 bot-1 idle ticks are furnace waits, including one
`take 50 iron-plate from the furnace` that waited 9,796 ticks against a modelled
`192 × 51 = 9,792`. The disease is not a four-bot disease.)

**2.3 "A measured serial budget suggested ~24,481 ticks (6.8 min) is the floor
if nothing parallelises."** That figure is the *hand-mining* total, and it is the
one quantity here that divides cleanly by four. It is not a floor; it is the
largest divisible term. Re-measured on the reference run: 24,158 bot-ticks of
mining over 200 units at a uniform 120.8 ticks/unit
(`VANILLA_CHARACTER_MINING_SPEED = 0.5`, `util.rs:99`, so 2 s per unit for every
one of iron, stone, coal and copper — the rates really are identical).

The brief's older accounting reproduces exactly, for the record:
`run-1788449752-46541` m1 gives mining 45,478 (stone alone 19,953), walking
31,085 (26,254 + 1,052 + 1,609 + 2,170), crafting 7,034, lab 5,999.

---

## 3. The floor

Assumptions, all stated so each can be attacked separately:

1. **Hand mining is 120.8 ticks/unit** for every raw material here. Measured, 49
   actions, uniform.
2. **The bill of materials is ~200 raw units** — 79 iron ore, 65 stone, 30 coal,
   26 copper on the reference run; 183 units on the solo run on a different map.
   Roughly 106 iron plates and 27 copper plates are required by the recipe tree
   (lab 36 Fe + 15 Cu; steam engine 31 Fe; 10 science packs 30 Fe + 10 Cu;
   boiler, pump, poles, pipes ~14 Fe + 2 Cu), and the stone is furnaces at 5
   each.
3. **A stone furnace smelts one plate in 192 ticks.** 133 plates = 25,536
   furnace-ticks, which divides by the number of furnaces running at once.
4. **Research is 5,999 ticks and is irreducible with one lab.** Measured, and
   the same to the tick in all three runs.
5. **Crafting 10 automation-science-packs is 3,009 ticks** on one bot; the
   packs' ingredients are per-pack independent, so the craft divides.
6. **Walking**: 357 ticks per site transition on this map, ~4 transitions per
   bot if each bot owns one resource, plus one round trip to the water site.

Then the critical bot's floor:

| leg | ticks | derivation |
| --- | ---: | --- |
| its quarter of the gathering | 6,040 | 24,158 / 4 |
| its walking | 3,500 | ~4 transitions + a power-plant round trip |
| smelting latency it cannot overlap | 2,000 | 25,536 / 8 furnaces, mostly pipelined behind mining |
| its share of non-science crafting | 2,000 | convergent bill stays on one bot |
| science packs | 800 | 3,009 / 4 |
| insert + research | 5,999 | irreducible, one lab |
| dispatch overhead | 1,071 | measured |
| **floor** | **21,410** | **5.9 min** |

With a second lab the research leg halves to 3,000 and the floor is ~4.9 min,
paid for with ~51 more raw units (36 Fe + 15 Cu) = 6,163 bot-ticks = 1,541 ticks
split four ways. Net ~−1,450. That is the shape of the owner's "~4 optimised"
estimate and it survives the arithmetic.

**So: floor ≈ 5.9 min with one lab, ≈ 4.9 min with two. Nine minutes is 32,400
ticks — 51% of headroom over the floor.** The target is not tight. What makes it
hard is that the two largest terms have to be attacked separately.

---

## 4. Ranked changes

Ranked by **saving ÷ risk**. Savings are ticks off the 74,153-tick milestone,
computed against the measured buckets in §1.1 and §1.2. Risk is a judgement,
stated with what specifically could go wrong.

### R1 — Fan a one-shot smelt across a bank of furnaces

**Saving: ~12,300 ticks (16.6% of the milestone). Risk: low. Ratio: highest.**

**Mechanism.** `smelt_steps` (`have.rs:~370-830`) sites exactly one furnace via
`free_area_near` and links `smelt_lag = per_run × runs + per_run`. Make it site
`k` furnaces, distribute `runs` across them, and link the take with
`per_run × ceil(runs / k) + per_run`. `plan_cells` already does deterministic
multi-site siting for `BuildCell`; the ordered-collection and `total_cmp`
discipline is unchanged.

**Choice of `k`.** Two variants, and the second is much better:

- *Naive*: `k` proportional to `runs`, building fresh furnaces per smelt. With
  `k = 4` the nine waits fall from 16,743 to 6,144 (`192 × (ceil(n/4)+1)`
  summed). But every furnace is 5 stone, and the reference run already placed 12
  furnaces for 12 smelts — the baseline run placed **33** and spent 19,953 ticks
  mining stone for them.
- *Preferred*: **reuse standing furnaces.** `BuildCell` already has
  `cells_standing` (`produce.rs:~805`) for exactly this question on the rate
  path. A `Smelt` that adopts up to `k` idle stone furnaces near its ore anchor
  before placing new ones turns the existing furnace population into a bank.
  With an 8-wide bank the nine waits fall to **4,416 ticks**, and stone demand
  goes *down*, not up.

**Interaction with sizing/binding: none.** The furnaces, their stone and their
coal all stay with the chain owner; every action added is the owner's. This
change is invisible to `Holder`, to chain ownership and to `schedule.rs`'s
candidate tiers. That is why it ranks first: it is the largest single saving and
the only one of the top four that cannot possibly reintroduce the
`run-1788405365-21697` crash.

**Risks.** (a) Placement geometry — eight furnaces near one ore anchor need
eight legal, reachable, non-colliding tiles; `free_area_near` reserves real
collision boxes already, and `tile_reservation.rs` / `tile_capacity.rs` guard
the rest, but the *reach* condition for eight separate `Insert`s from one
standing position is new. Expect the bot to walk within the bank; budget ~10
ticks per furnace, not zero. (b) `take` must now be `k` removals, not one, which
changes the action count and every makespan pin in the crate. (c) Coal: `k`
furnaces need `k` fuel loads; `coal` is computed from `recipe_ticks × runs`
total energy, which is correct in aggregate and has to be divided the same way.

**Testable offline.** Entirely. `crates/planner/tests/smelt_roots.rs` and a new
case asserting the lag on a 20-plate smelt drops from 4,032 to ~768.

### R2 — Judge drill-vs-hand by makespan, not bot-busy ticks

**Saving: ~10,700 ticks (14.4%). Risk: low–medium. Ratio: second.**

**Mechanism.** `PlaceDrill::applicable` (`produce.rs:1022-1035`) compares
`cell_setup_bot_ticks` against `hand_smelt_bot_ticks`, both in bot-busy ticks,
and both deliberately excluding unattended machine time. Add the roster to the
comparison: the hand path's mineable component divides by the number of bots
that can actually work the patch (the `seats` figure `expand_goal_body` already
computes), while the cell path's `ticks_per_item × need` does not divide by
anything. For 50 iron plates on four bots:

| | bot-busy (today's gate) | makespan (proposed gate) |
| --- | ---: | ---: |
| hand-smelt | 6,540 | ~1,600 mine (4-way) + 1,536 smelt (8-wide bank) = **3,136** |
| build a cell | 5,928 | 12,240 modelled, **29,042 measured** |

Today's gate picks the cell by 9%. A makespan gate picks hand-smelting by 4–9×.

**This does not delete `PlaceDrill`.** It is the right method when the roster is
one bot, when the ore is far and the bot has other work, and for any `Producing`
rate goal — which is a different code path entirely (`BuildCell`). It should
lose only where a roster exists to divide the work.

**Interaction with sizing/binding: none directly** — it changes which method
claims a goal, not who owns the chain. But it composes with R1 (the hand path is
only cheap once the furnace bank exists) and with R4 (the ore only divides once
`SharedSmelt` can fire). Land R1 first.

**Risks.** (a) `applicable` currently has no roster in scope; threading one in
must not make the predicate depend on anything unordered. (b) The gate flipping
to hand-smelting for the `steam-power` trigger means 50 iron ore get hand-mined
that the drill used to supply free — that is 6,040 bot-ticks of new work, 1,510
per bot, and it is *why* this ranks below R1 rather than above it. (c) The
measured 2.37× cell overrun should be understood before it is used as evidence
for anything; the makespan argument holds on the *modelled* 12,240 alone.

**Testable offline.** Yes, as a planner unit test on which method claims a
50-plate `Produced` goal at roster sizes 1 and 4.

### R3 — Re-own inventory-independent subtrees to other bots

**Saving: ~13,000 ticks of mining + ~9,400 of walking = ~22,400 (30.2%). Risk:
medium. Ratio: third by ratio, first by absolute saving.**

This is the crux; §5 treats it properly. In summary: a subtree whose product is
**world state** (a furnace standing at a fixed position, a machine with coal in
it) does not need to converge in any inventory, and can therefore be sized
against and bound to a different bot — sizing and binding still agree, just four
times over. `Step::Owned` (`method/mod.rs:51`) is the existing, shipped
mechanism for exactly this, and `SharedSmelt` already uses it.

### R4 — Let several bots load the lab

**Saving: ~2,200 ticks (3.0%). Risk: low. Ratio: fourth.**

**Mechanism.** `craft 10 automation-science-pack` is 3,009 ticks on one bot, and
it sits between the last plate and the research — dead centre of the serial
tail. Each pack is 1 copper plate + 1 iron gear and is independent of every
other pack.

The reason it does not divide is `Researched`'s statement that "the research is
one action reading one bot's inventory". **That is not what the run does.** The
actual sequence is `insert 10 automation-science-pack into the lab` and then
`research automation`: the precondition is on the *lab's* inventory, not a
bot's. A lab accepts inserts from any number of bots. So the packs can be
crafted 2–3 per bot in four parallel `Step::Owned` sub-chains and inserted by
four different bots, with no handover, no chest, and no convergence.

3,009 → ~800 (four-way craft) plus four inserts at 0 ticks and up to one extra
walk to the lab per bot.

**Interaction with sizing/binding.** Each pack sub-chain is `Holder::Share(bot)`
sized against that bot's own copper plate and gears — sizing and binding agree
by construction, the same way `SharedSmelt`'s supplier shares do. What must be
checked is that `Condition::HasItem { who: Actor::Role, item: pack }` on the
insert resolves to the *inserting* bot and not the taker; `Actor::Role`
(`action.rs:17-29`) resolves against the chain binding, so a `Step::Owned`
sub-chain gets this right for free.

**Risk.** Low mechanically, medium in that it is the first place the crate would
model a *machine's* inventory as the convergence point rather than a bot's. If
that generalises badly, confine it to the lab.

### R5 — Resource-affine owner selection

**Saving: folded into R3's ~9,400 walking figure; ~3,000 additional if done
separately. Risk: low. Ratio: fifth.**

`even_shares` (`have.rs:2232-2262`) sorts candidates by `(spare, BotId)` —
poorest-first, which is right for *how much* but says nothing about *where*. Add
distance to the ore anchor as a secondary key when spare ties (which it does, at
the start of a run, for every bot). Without this, R3 spreads the work but the
four bots may all be sent to the same patch and then all sent to the next one.

Determinism is preserved: distance is an `f64` compared with `total_cmp`, and
`BotId` remains the final tie-break.

### R6 — A second lab

**Saving: ~3,000 gross, ~1,450 net. Risk: medium. Ratio: last.**

Halves the 5,999-tick research leg. Costs 36 iron plates and 15 copper plates
(10 electronic circuits, 10 gears, 4 belts) = ~51 raw units, and the two labs
must split the 10 packs. It only becomes worth doing once R1–R4 have taken the
milestone under ~8 min and the research leg is a third of what is left. **Do not
do this first**: it adds material demand to a plan whose problem is latency, and
it moves the makespan pins for a reason that is easy to mistake for a
regression.

### Summary of the ranked plan

| | change | saving (ticks) | risk | touches |
| --- | --- | ---: | --- | --- |
| R1 | furnace bank for one-shot smelts | 12,327 | low | `have.rs` `smelt_steps` |
| R2 | makespan-aware drill gate | 10,708 | low–med | `produce.rs` `PlaceDrill::applicable` |
| R3 | re-own placement/fuel subtrees | 22,394 | med | `have.rs`, `method/mod.rs` |
| R4 | multi-bot lab loading | 2,209 | low | `have.rs` `Researched::expand` |
| R5 | resource-affine owner choice | (in R3) | low | `have.rs` `even_shares` |
| R6 | second lab | 1,450 net | med | `have.rs` `Researched::expand` |

Projected residual with R1–R5:

| leg | now | after |
| --- | ---: | ---: |
| bot-1 mining | 19,083 | 6,040 |
| bot-1 walking | 12,851 | 3,500 |
| `Smelt` waits | 16,743 | 4,416 |
| cell wait | 12,244 | 1,536 |
| crafting | 6,162 | 3,953 |
| research | 5,999 | 5,999 |
| overhead | 1,071 | 1,071 |
| **total** | **74,153** | **26,515 = 7.4 min** |

And the two counterfactuals that make the case for doing both halves:

- **utilisation only** (R3 + R4 + R5): 74,153 − 13,043 − 9,351 − 2,209 =
  **49,550 = 13.8 min**
- **smelting only** (R1 + R2): 74,153 − 12,327 − 10,708 = **51,118 = 14.2 min**

---

## 5. The crux: splitting under an owned chain

The constraint is not negotiable and is worth restating in its own terms.
`run-1788405365-21697` failed with `precondition has 3 iron-ore … does not hold
for bot 2` because a chain was sized against bot 1's stock and bound to bot 2.
**A chain's bill and its runner must be decided together.** Below, each option is
judged on exactly that.

### 5.1 The distinction that unlocks this: inventory-convergent vs world-convergent

Not everything in `Researched`'s subtree needs to converge. Sort the subtree:

**Inventory-convergent** — must end in one pair of hands, because a single
downstream action reads one inventory:
- the 10 automation-science-packs *as ingredients of the craft* (each pack's
  copper plate and gear must be in the crafter's hands);
- the iron plates for the steam engine, before `craft 1 steam-engine`;
- the copper cable before `craft 1 electronic-circuit`.

**World-convergent** — the product is a fact about the map, and *any* bot can
produce it:
- `place stone-furnace at [-31,-25]` — the 5 stone, the craft and the placement
  can all be one other bot's errand end to end. Nothing downstream reads the
  placer's inventory; the next action reads `Condition::EntityAt`.
- `fuel the furnace with 1 coal` — the coal must be in the *fuelling* bot's
  hands, which `Actor::Role` already expresses correctly.
- the boiler, the offshore pump, the steam engine's *placement* (not its craft),
  the electric pole's placement.
- the lab's placement.
- and, per R4, the pack **inserts** — the lab's inventory is world state.

On the reference run, the world-convergent set is: 65 stone (7,854 ticks), 30
coal (3,616), 13 furnace crafts (390), 12 placements (0 ticks), and — decisively
— **fourteen of bot 1's twenty-six site transitions**, because stone and coal
are two of its four patches. Call it 11,860 act ticks plus ~5,000 walk ticks
that need not be on the critical bot at all.

### 5.2 Option A — let `SplitAcrossBots` claim pure-gathering `Share` subgoals

Relax `claims` from `site.top_level && !site.in_chain` and `applicable` from
`Holder::Anyone`, so that a `Have{stone, Share(b)}` inside a chain gets split.

**Sizing and binding: broken.** The subgoal was sized against bot `b`'s stock
and the downstream `craft 1 stone-furnace` reads bot `b`'s inventory. Splitting
the mining across four bots leaves the stone in four inventories and the craft
fails for exactly the reason `run-1788405365-21697` failed. **Reject.** This is
the shape the brief warned about.

### 5.3 Option B — a fallback tier for actions any bot could run

Give `schedule.rs:350` a second tier of "bots whose preconditions are also
satisfiable".

**Sizing and binding: broken, and worse, silently.** The scheduler evaluates
feasibility per candidate against a forked state — which is precisely the
mechanism `2026-09-01-per-bot-share-sizing-design.md` §4 argued was safe and
which the live run then falsified. A precondition that is satisfiable *at
scheduling time* for bot 2 is not the same claim as a bill that was *sized*
against bot 2, and the divergence appears at execution. This is also structurally
identical to the walk-refusal ledger the brief names: a preference placed
downstream of a hard constraint. **Reject.**

### 5.4 Option C — size shares per-bot at expansion (four correct chains)

Recommended, and it is the option the codebase has already half-shipped.

`Step::Owned { whose: Holder, steps: Vec<Step> }` (`method/mod.rs:51`) opens a
new chain owned by a *named* bot from inside an enclosing chain. `method/mod.rs`
calls it "the one place a *different* bot's share is [emitted]" (`:732`), and
`SharedSmelt` uses it today: on the reference run, plan 2 gave bots 2, 3 and 4
`mine 5 iron-ore` and `insert 5 iron-ore` into bot 1's furnace, inside bot 1's
`Researched` chain, and bot 1 took the 20 plates. **The cross-chain, cross-bot
path through an owned chain is shipped, running, and produced zero failures in
this run.**

The change is to emit `Step::Owned` for world-convergent subtrees too. Concretely,
where `smelt_steps` today emits the furnace's own bill into the enclosing chain,
it would emit:

```
Step::Owned { whose: Holder::Share(supplier), steps: [
    Subgoal(Have { item: "stone", count: 5, whose: Holder::Share(supplier) }),
    Subgoal(Have { item: "coal",  count: n, whose: Holder::Share(supplier) }),
    Act(craft stone-furnace),      // Actor::Role -> supplier
    Act(place stone-furnace at P), // Actor::Role -> supplier
    Act(fuel the furnace),         // Actor::Role -> supplier
]}
```

with `P` fixed at expansion exactly as it is today.

**Does sizing still agree with binding? Yes, and by the same construction that
makes the current code correct.** Each sub-chain is sized against `supplier`'s
own stock (`state.available(&Holder::Share(supplier), item)`) and bound to
`supplier` by `mod.rs:675-698`. The taker's chain is sized against the taker and
bound to the taker. There is no chain anywhere whose bill and runner disagree.
Four independently-correct chains, not one chain with a relaxed constraint.

**What breaks, honestly:**

1. **A cross-chain edge lands on the critical path.** `smelt_steps`'s own doc
   already declined this trade — "placing it in a supplier's chain would buy an
   extra cross-chain edge on the critical path for about five stone and one coal
   of work. That is a real residual and a deliberate one: stage 1 changes one
   thing." That judgement was made when the residual was one furnace. It is now
   65 stone, 30 coal and fourteen site transitions, and the arithmetic points
   the other way. **This recommendation is the documented next step of a shipped
   design, not a new idea** — which is most of why it is a medium risk and not a
   high one.
2. **`infer_edges` is not exercised on this path.** `red_science.rs`'s
   `every_expansion_replays_in_time_order` says so in writing: "On today's
   fixture it does not yet discriminate… So this guards the moment that stops
   being true (a method reusing an existing furnace, an `Actor::Bound` holder, a
   `Consolidate`)". A supplier-owned placement is exactly that moment. Expect to
   find real dropped-edge bugs, and expect that test to start earning its keep.
3. **Seat contention.** `worth_converging`'s G6 refuses a split when the patch
   cannot spare `k + roster` seats (`have.rs:~2478`). Four bots mining four
   *different* patches do not contend, but the seat accounting is per-item and
   would need to see that.
4. **Who is `supplier`?** It must be a deterministic function of the state —
   least-loaded by planned ticks, tie-broken by `BotId`, ideally distance-aware
   per R5. Not "whoever the scheduler likes", which is Option B in disguise.

**Testable without a live run? Almost entirely.** The planner is pure: a unit
test can assert that a `Researched("automation")` expansion on a four-bot
fixture produces at least one `Step::Owned` per non-taker bot, that every
chain's owner equals the bot its bill was sized against, and that
`assert_preconditions_hold_over_time` holds over the whole schedule. The one
thing only a live run shows is walk feasibility to four patches at once, which
the run recorder already measures (`wfail`, currently zero).

### 5.5 Option D — what I would actually do first

**Do R1 before R3.** Not because it is easier, but because R3's benefit is
measured against a critical path whose largest term is currently a furnace wait.
Land R1, re-measure, and R3's true saving becomes visible instead of estimated.
Doing them in the other order means R3's ~22,000 ticks partly vanish into slack
that R1 would have removed anyway, and the run that follows will look like R3
underdelivered.

---

## 6. What to measure, and what falsifies each change

Every run must be on the **same map** as `run-1788459085-32452` — §2.2 exists
because that was not controlled last time. Record the seed in the manifest, and
compare the walk-destination clusters as a cheap check that it held.

The primary instrument is `python3 tools/run_analysis.py --json <run-dir>`, plus
the bot-1 idle-gap decomposition used in §1.1 and §1.2, which should be added to
the tool: **a gap histogram with the action each gap was waiting for is the
single measurement that would have prevented today's misdiagnosis**, and it is
about twenty lines on top of what `join_actions` already computes.

| change | success criterion | falsified by |
| --- | --- | --- |
| R1 | the nine `Smelt` waits sum below 6,000; no wait exceeds `192 × (ceil(n/k)+1)`; furnaces placed does not rise | a wait still equal to `192 × (n+1)` (the bank was not used); stone mined rises above 65 (the bank is not being reused) |
| R2 | `take … from the cell` absent from m1; `mine iron-ore` unit count rises by ~50 and is spread across bots | total m1 mining rises without the span falling — the gate flipped but the ore did not divide, i.e. R2 landed without R3 |
| R3 | `dispatches by bot` no worse than 4:2:2:2; stone and coal appear in bots 2–4's verb histograms; bot-1 walk ticks below 6,000 | any `precondition … does not hold for bot N` — the sizing/binding invariant broke, revert immediately; or `wfail > 0`, meaning four bots to four patches is not walkable here |
| R4 | `craft automation-science-pack` appears on ≥3 bots, each ≤1,000 ticks; ≥2 `insert … into the lab` | a lab that refuses a second inserter, or research starting before all 10 packs are in |
| R5 | bot-1 site transitions ≤ 8; each bot's walk destinations cluster to one or two sites | walk ticks flat while dispatches spread — the work divided but the travel did not |
| R6 | `research automation` ≤ 3,050 ticks | research still 5,999 with two labs standing (the second lab is unpowered, or holds no packs) |

**The one aggregate to watch**: fleet utilisation, currently 17.1%. It should
reach ~45–55%. It cannot reach 100% and should not be pushed toward it — the
research leg is 5,999 ticks during which there is genuinely nothing for anyone to
do, which caps a nine-minute run at ~82% by construction.

**Makespan pins that will move**, each of which must be explained in place and
never silently updated: `have.rs:6794` (12,403), `have.rs:6872` (12,428),
`produce.rs:1984` (10,315), `red_science.rs`'s `many < 2310` ceiling, and the
`many.saturating_mul(2) < one` floor. R1 moves all of them; R3 moves the
`red_science.rs` pair hardest, and the ceiling should be *retightened* after, per
the instruction that file already gives itself.

---

## 7. What should not be done

**Do not add a fallback tier to the owner constraint.** §5.3. It is the exact
structure of both prior failures in this area: the walk-refusal ledger that shipped
inert because it sat downstream of a single-candidate constraint, and the
per-bot-share design's §4 argument that a `Share` chain's binding could safely be
left to the scheduler — which a live run falsified twice in one run, naming a
different bot each time.

**Do not relax `SplitAcrossBots`'s `claims` guard.** §5.2. Splitting a bill whose
consumer reads one inventory is the crash, restated.

**Do not "fix" the drill cell by giving it more coal or a second drill.** The
2.37× overrun is real and worth understanding, but the cell is the wrong tool for
a one-shot 50-plate goal on a four-bot roster whether or not it hits its model:
12,240 modelled ticks against ~3,100 for four bots hand-mining into a furnace
bank. Tuning it optimises the losing branch. (Keep `PlaceDrill` for `Producing`
rate goals and solo rosters, where it wins on both objectives.)

**Do not chase the verb histogram.** 39% of this milestone is a wait, 17% is
walking, and `place`/`insert`/`take`/`fuel` all record zero. The histogram sees
none of it. The gap decomposition is the instrument; the histogram is a
cross-check.

**Do not compare across maps.** §2.2. The 1-vs-4 conclusion that motivated part
of this brief was drawn from two different worlds, one of which has a resource
100 tiles east.

**Do not touch the executor to make the waits shorter.** Every one of the ten
waits is a real furnace running at the real recipe rate; the executor is
correctly waiting for a real precondition. Any "improvement" here would be the
teleport hack again — manufacturing an arrival the game never granted. The 192
ticks per plate is physics; the only lever is how many furnaces run at once.

**Do not add a second lab first.** §R6. It adds material demand to a
latency-bound plan and moves the makespan pins for a reason that reads exactly
like a regression.

**Do not aim for 100% fleet utilisation.** Three bots idle during the 5,999-tick
research leg is correct behaviour, not a bug to be optimised away with make-work.
Utilisation is a diagnostic here, never a target.

---

## 8. Confidence

**Under nine minutes is reachable.** Projected 7.4 min with R1–R5, against a
derived floor of 5.9 min with one lab. The projection is built from disjoint,
directly measured buckets rather than from a model, and the two largest terms
(the 28,987-tick furnace wait and the 31,934 ticks of bot-1 mining and walking)
have mechanisms whose savings are arithmetic rather than speculative: 192 ticks
per plate divided by a furnace count, and 24,158 bot-ticks divided by four.

**Confidence that <9 min is reachable: high (~85%).** The residual risk is
concentrated in R3, and it is not "will it save the ticks" — it is "will four
bots working four patches at once expose dropped edges in `infer_edges` that the
crate's own tests say they have never been pointed at." That is a schedule risk,
not a feasibility risk.

**Confidence that ~6 min is reachable: moderate (~50%),** and it needs the
second lab or near-perfect overlap of the gather and smelt phases.

**Confidence that <5 min is reachable: low.** Two labs put the floor at ~4.9
min, which leaves no margin at all for replans, walk variance or the ~1,000
ticks of dispatch overhead.

**Single highest-value change: R1, the furnace bank** — 12,327 ticks, no
interaction with the sizing/binding constraint, testable entirely offline. It is
not the largest absolute saving (R3 is, at 22,394) but it is the largest saving
per unit of risk, it is a precondition for R2's arithmetic, and it should be
landed and measured before R3 so that R3's real benefit is observed rather than
estimated.
