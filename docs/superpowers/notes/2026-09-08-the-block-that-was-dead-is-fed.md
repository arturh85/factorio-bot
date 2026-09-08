# The block that was dead is fed

*2026-09-08. The reproduction asked for by
`2026-09-07-a-powered-block-that-is-not-powered.md`, run on current master
(`b5376c94`) before hunting a cause. **It does not reproduce.** The premise the
brief flagged as most likely wrong was wrong, and that is the whole result.*

## What was inherited, and what was measured

Everything under "inherited" comes from the handoff note and the brief. Nothing
under "measured" is quoted from either.

| | inherited | measured 2026-09-08 |
|---|---|---|
| Run A places | 63 of 63, `done=true failed=0` | 41 of 41 at the **planned tile** |
| coal consumed | **0 of 200** | **80 of 100** (50→14, 50→6) |
| inserter swings | **none** | 6 arms live, none `no_power` |
| plant | boiler + engine + pump standing | boiler, pump, engine all `working` |
| plant→block hop | ~48 tiles | ~48 tiles, same siting |

Two headless runs, `workspace/headless-a`, seed 31337 `--new`, four character
bots (`plan_created.bots` = `[1,2,3,4]`, checked), `--game-speed 10`, debug
binary built from this worktree. Window bounded in **game ticks** (20,001 in
both), never in poll counts.

## Run 1 — `ElectricOreToPlate`, no siting hint: the peer's Run A shape

```
plan: 54 steps, 41 place steps, bots 4
build: done=true failed=0 lost=0 pending=0
plan kept at the planned tile: 41 of 41

chests loaded by hand:      iron-ore x96, coal x50   (each of two)
window:                     20,001 ticks

after:  chest (5.5,-3.5)  coal  50 -> 14
        chest (3.5,-1.5)  coal  50 ->  6
        furnace (7.0,3.0) FUEL SLOT  coal x7
        furnace (9.0,3.0) FUEL SLOT  coal x5

boiler        :: working                          1
offshore-pump :: working                          1
steam-engine  :: working                          1
inserter      :: waiting_for_source_items         4
inserter      :: waiting_for_space_in_destination 2
stone-furnace :: no_ingredients                   2
```

**Items moved, and the whole chain moved them.** Coal left the chests, rode the
belt and ended in the furnaces' *fuel* slots. No bot did that: the loading was
a single `insert_to_inventory` into each chest before the window opened, and
nothing in the plan ran during it. That is chest → inserter → belt → inserter →
furnace, machine-driven, with the only power source 48 tiles away.

`plan kept at the planned tile: 41 of 41` is the stamp fix visible from here —
the diagnosing runs read **40 of 41**, the missing one being a
`small-electric-pole` at (3.5, −2.5) the build had mined out from under itself.

## Run 2 — `SmeltRow24`: 152 entities, 624 kW, zero generators

The block the coordinator named as the clean test of *generation*, since it
distributes internally and carries no generator at all. Sited with
`near = {x = 30, y = 30}` — with no hint it refuses at spawn on ground, which
is a siting question and out of this lane.

```
build: done=true failed=1 lost=0 pending=0
plan kept at the planned tile: 165 of 166   (one stone-furnace failed to place)

boiler        :: working                    1
offshore-pump :: working                    1
steam-engine  :: working                    2      <- 1,800 kW nameplate
inserter      :: waiting_for_source_items   48
stone-furnace :: no_fuel                    23
```

**48 of 48 electric inserters are powered.** Not one reads `no_power`. The
plant `ensure_powered` sited carries two engines against the block's 624 kW,
and every arm in the block is on that network across the hop.

The furnaces read `no_fuel` because `SmeltRow24` has no chest in it and the
probe therefore had nothing to load — a harness limit, not a planner one. So
this run proves **power**, and says nothing about throughput.

## `no_power` appears zero times in either run

That is the reading that settles it, and it is worth stating as the method
rather than the result. `entity.status` distinguishes *"I have no electricity"*
from *"I have nothing to pick up"* by name. Every previous attempt at this
question read plate counts, which cannot tell those apart — and the peer's
"not one inserter swing" was itself an inference from a zero.

54 electric arms across two blocks, none of them dark.

## What closed it, and how we know it was that

`the-stamp-clears-only-its-own-ground` (2026-09-07). `place_blueprint` was
mining every non-character, non-resource entity in its build area before
stamping — **including the plan's own power poles**, laid moments earlier by
the same expansion — and reporting `done=true failed=0` over dark ground.

The evidence that this is the cause and not a coincidence is the 40 → 41:
the entity the earlier runs lost is exactly a pole of the run
`ensure_powered` laid, it is named in the diagnosing log
(`WARN mining entity in build area: small-electric-pole @ 3.5/-2.5`), the fix
removed the mine, and the pole now stands. The peer agrees it explains their
pair better than either of their own hypotheses: Run B refused at plan time and
so never built anything, which is why it never mined anything — the
`Some`/`Ok(None)` asymmetry is that only one of the two got far enough to
destroy its own power.

**Retracting to uncertainty where that is all we have**: nothing here
*proves* the stamp was the only cause of the peer's Run A, because that run's
world is gone. What is established is that the failure does not reproduce on
master, and that the mechanism named for it is gone.

## What `ensure_powered`'s `Some` promises — asked, and left alone

It promises a claim about a **plan**: *if every step returned is executed and
every pole is still standing, the game's own rule says the site is powered.*
Its doc already lists three things outside that claim, and the third was "the
poles are still standing", which was false for `Goal::Built` until the stamp
fix. It is now true, and the doc says so with the live figures above.

**The honest fix was neither a siting bug nor a new postcondition.** The
postcondition idea — `method::blueprint` drops `Powering::powered` at
`blueprint.rs:1768` where `method::extract` states it — remains available and
remains *defence in depth*, not a fix: there is no known remaining way for a
block's power to evaporate between `ensure_powered` and dispatch. It is in a
file a peer session owns and was left untouched.

## The trap the next person will fall into, and one guard against it

**A fix that only works when plant and block are adjacent passes a fixture and
fails here.** On seed 31337 it *cannot* be adjacent: drills need ore at 18.4
tiles from spawn and a plant needs water at 48.1, so no anchor satisfies both.
Both live runs had a real hop.

Checked: the suite does exercise a multi-pole run —
`ensure_powered_plans_a_block_whose_own_draw_it_no_longer_double_counts` emits
**5** pole placements between the fixture's lake and its block. That was
measured, not assumed, and it is now **asserted** (`poles >= 3`) with the
reason beside it, so a future fixture that quietly moves the lake next to the
block fails instead of proving nothing. Five poles is about 30 tiles against
the live 48, so the fixture is a smaller hop than the map forces — the
assertion pins "not adjacent", not "as far as reality".

**The first mutation I chose to falsify that assertion came back GREEN, and it
was a finding about my own reasoning rather than about the test.** Moving the
fixture's block from (60, 56) to (6, 6) — "put it next to the lake" — produced
**7** poles, more than the 5 it replaced. The plant is sited *at the water
wherever the block is*, so the block's position does not shorten the run; there
is no hub position that makes them adjacent. The mutation that does kill the
test is the one that describes the defect the assertion is guarding against:
`for pole in run` → `for pole in run.into_iter().take(1)`, a version that lays
one pole beyond the plant and can therefore only power an adjacent site. It
fails with `got 2 pole placement(s)` and kills exactly that one test, 5 of 6
in the module still passing.

## Baselines: none moved

Debug binary, this worktree, `b5376c94` plus the doc and assertion changes
described here. Goal strings stated beside each, as asked.

| goal | world | actions / ticks |
|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,117 / 325,138 |

All four identical to the figures in the brief. Expected: nothing in this
change reaches planning.

## What is still open, and is not power

- `ElectricOreToPlate` moved **coal** and not **ore** — 96 of 96 ore sat in
  both chests for 20,001 ticks while coal drained. That is the belt-lane class
  this repo already documents (`2026-09-06-t-junction-smelter.md`), a
  blueprint-design question, and it is why the block still makes zero plates.
- `SmeltRow24` with no siting hint refuses at spawn
  (`transport-belt fits at [0.5, 0.5] facing 8 does not hold there`). Siting is
  a peer session's lane.
- One `stone-furnace` of 166 failed to place in run 2 (`failed=1`).

## Method, for the next reproduction

Two things cost a run each and are cheap to avoid:

- **`rcon.move` refuses a walk that would end inside a belt**, correctly and by
  name. Loading a block's chest does not need the walk at all —
  `insert_to_inventory` is a mod remote call that moves the bot itself if it
  must.
- **Read the inventory back after loading it.** The first probe printed table
  addresses instead of contents and could not say whether its own setup had
  worked, which would have made any conclusion about "nothing moved" worthless.
  A chest is `output_inventory`; `input_inventory` is `nil` there, and
  `Option::None` reaches Lua as truthy light userdata.
