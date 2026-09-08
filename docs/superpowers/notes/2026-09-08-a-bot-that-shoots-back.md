# A bot that shoots back

2026-09-08. Seed 31337, `--new`, four headless character bots, game speed 10,
Factorio 2.1.17 Space Age, default map settings. Isolated instance
`workspace/headless-c.toml` (rcon 4332 / game 34212), debug build from
`.worktrees/shoots-back` at `bee324dd`. Five runs; every number below was read
off a live game over RCON, none of it inherited.

**DISCLOSED CHEATS.** Every encounter here was arranged: biters, a spawner and
a worm were created by script, characters were teleported apart so one fight
could not contaminate another, and light armour was inserted into an armour
slot by script. Nothing about the *fights* was arranged — each character used
the kit the game's own freeplay gave it at spawn, and the only thing that
differed between the paired arms was one write to `shooting_state`. The owner's
standing rule is that cheating while developing is fine and a measured run must
be honest and say so; this is a development note and none of it is a measured
run.

## The task was framed as "the bots were unarmed". They were not.

Every headless character bot spawns holding a **pistol in
`defines.inventory.character_guns` and ten `firearm-magazine` in
`character_ammo`**, read back on four separate fresh runs:

```
uid1 guns[pistolx1] ammo[firearm-magazinex10]
uid2 guns[pistolx1] ammo[firearm-magazinex10]
uid3 guns[pistolx1] ammo[firearm-magazinex10]
uid4 guns[pistolx1] ammo[firearm-magazinex10]
```

That is the freeplay starting kit, which `rcon_spawn_bots` asks for by calling
`remote.call("freeplay", "get_created_items")` — CLAUDE.md lists the iron
plates, furnace, drill and wood and does not mention the weapon, which is why
two briefs in a row assumed there was none. **The bots that died on the oil walk
were armed the whole time.** They simply never pulled the trigger.

## A character does not auto-fire, and one biter proves it nine times

Default `shooting_state.state` is `0` (`defines.shooting.not_shooting`) on every
character, always. In the first run a **single small-biter killed nine
characters in a row** — bots 4, 2, 3, 2, 1, 3, 2, 4, 1, 3 across ticks
80,397–93,366 — while every one of them stood there with a pistol and ten
magazines. The biter's health read `hp=15` of 15 in every sample for the whole
massacre. It was never shot at once.

So the premise held: **`shooting_state` is the fire button, and nothing presses
it.**

## The paired control

Four characters, one tick stream, one small-biter spawned twelve tiles east of
each (`scratch/paired2.sh`, run 4):

| | kit | order | outcome |
|---|---|---|---|
| A | pistol + 10 magazines | `shooting_enemies` | biter dead in **~106 ticks**, at range. **Zero damage taken.** |
| B | pistol + 10 magazines | none — the control | 250 → 194 hp, monotonic, biter untouched at 15/15. **Died** (run 3, tick 5,382). |
| C | pistol, **ammo emptied** | `shooting_enemies` | biter dead. Cost **14 hp**. Killed it with **fists**. |
| D | pistol + 10 magazines | `shooting_enemies`, **no enemy** | ammo 10 → 10, hp 250. A held fire button costs nothing when there is nothing to shoot. |

A second wave against A at tick 1,032 died the same way with A still at 250 hp,
so **the order survives a kill** and is not consumed by use.

Three things fall out that were not expected going in:

- **The mechanism is the order, not the gun.** Arm C is the one that says so:
  a character with no ammo at all still killed a small biter, bare-handed, for
  14 hp. The gun makes it free instead of cheap.
- **The idle cost is zero.** Arm D held the order across the whole window and
  spent no ammo and took no damage.
- **The order does NOT survive a respawn.** Killing A by script and watching it
  come back: `before uid=1 sh=1` → `uid9 sh0`. A respawned character is a new
  entity, and it comes back **with nothing** — `main_inventory: {}`, no gun, no
  ammo, no order. So a bot that dies once is *more* defenceless than a bot that
  never has, on both counts at once. (That belongs to whoever owns death
  recovery; it is stated here because it was measured here.)

## Armour is not the cheap win. It is a 2x clock and the same ending.

`light-armor` reads `physical(decrease 3, percent 0.2)` on this install, so a
small biter's 7 becomes `(7-3)*0.8 = 3.2`. Measured rather than computed
(`scratch/armour.sh`, run 5) — bare control against armoured control, no order
on either, one biter each:

```
bare      250 -> 166 over ~520 ticks     died at tick 92,604
armoured  250 -> 208 over the same       died at tick 94,041
```

Exactly the predicted factor, and **exactly the same outcome**: 1,378 ticks of
life against 2,815. **Armour buys 2.04x the clock and changes nothing else**,
because nothing in that arm ever kills the biter. Seven planned actions for
twice as long to die is not a defensive answer; it is a longer death.

Against that, the standing order costs **zero actions and zero items**, and the
armed arm finished the same encounter at full health.

## The order is dangerous near a nest, and that is why it must not just be held

`shooting_enemies` picks its own target, and a structure is a target. A
biter-spawner placed 10.7 tiles from an ordered bot:

```
ammo 10 -> 0 in ~1,500 ticks
spawner 355 -> 170 hp, then REGENERATING back to 184
then: B@247, B@247  -- the nest started producing
```

The bot emptied its magazines into a nest it could not kill, and woke it. It
did not decide to; it was holding the button. **On the oil walk past 32 enemy
structures, a permanently-held `shooting_enemies` converts a survivable pass-by
into a war the roster cannot win.** So the minimal mechanism is *not* "set it at
spawn and forget it" — it has to be gated on a hostile **unit** being near, and
that gate is the open design question below.

## And it cannot answer a worm at all

The other death on the oil walk was a `small-worm-turret`. The arithmetic:

```
small-worm-turret  range 25
pistol             range 15
submachine-gun     range 18   (and its recipe is not enabled at t=0)
```

Measured, not inferred — an ordered bot with full ammo, worm placed 20 tiles
away:

```
WORM hp200 | WORM hp200 | WORM hp200 | WORM hp200 ...   (never scratched)
```

while the characters at spawn died and respawned into range repeatedly. **The
worm is outside every t=0 personal weapon's reach and the standing order is
irrelevant to it.** Half of last night's losses are solved by the order and half
are not, and the half that is not is a routing or a turret problem, not a
weapons problem.

## Numbers this session corrected

- **`defines.inventory` on this install is `character_guns=3`,
  `character_ammo=4`, `character_armor=5`** — not 5/6/7, which is what my brief
  carried. Read off the running game:
  `character_ammo=4 | character_armor=5 | character_corpse=1 | character_guns=3
  | character_main=1 | character_trash=8 | character_vehicle=7`.
  The wrong indices happened to fail loudly here (`cannot insert to nonexisting
  inventory`, and `room for 0` for a shotgun in an armour slot), but slot 5 is a
  *real* inventory, so 5/6/7 is one plausible prototype away from a silent
  wrong answer.
- **`defines.inventory.turret_ammo = 1`** on this install, not 33.
- **`gun-turret` is `enabled=false` at t=0**, so the owner's turret-creep
  retreat position needs research like everything else. What is enabled:
  `firearm-magazine=true`, `light-armor=true`; `gun-turret`, `shotgun`,
  `shotgun-shell`, `grenade`, `submachine-gun`, `piercing-rounds-magazine` all
  `false`.

## `insert_to_inventory` duplicates an item when the target is the actor

Arming through the existing verbs *appears* to work and is not a move:

```
remote.call("botbridge","insert_to_inventory", 1, "character", {x=0.5,y=0.5}, 3, {name="shotgun",count=1})
  -> "wtf, tried to take 1x shotgun from player #1 but only got 0"
  -> readback: guns=[pistolx1, shotgunx2]
```

`rcon_insert_to_inventory` inserts into the destination (which *creates* the
item) and then removes the same count from the actor to make it a move. When
the destination is the actor's own gun/ammo/armour slot the removal finds
nothing to take, and the net effect is duplication. The mod says so loudly in
the reply — the "wtf ... Isn't supposed to happen?!" branch — but nothing reads
that reply. **Do not arm a bot this way.** Equipping from one's own main
inventory is a different operation from inserting into a chest, and the mod has
no verb for it.

## What landed, and what it was verified against

`defend_character_bot` in `mods/BotBridge/control.lua`, raised from
`poll_character_bots` every `DEFEND_PERIOD = 30` ticks: hold the fire button
while, and only while, `surface.find_enemy_units` reports a hostile unit within
`DEFEND_RADIUS = 12` of the character, and lower it again when none is. No new
`ActionKind`, no planner vocabulary, no items, no research. Six stub tests in
`crates/core/tests/botbridge_returns_fire.rs`.

**Verified live on a fresh run with no manual intervention of any kind** (run 6,
after the change, `readlink` confirming the instance loaded this worktree's
mod):

```
gate closed, no enemies      sh0 sh0 sh0 sh0
two biters spawned           kills=2, bot 250/250 hp throughout, gate back to sh0
```

And against the exact failure the gate exists to prevent — a `biter-spawner`
placed 10 tiles away, **inside** `DEFEND_RADIUS`:

```
t=40782  C sh0 ammo9 | SPAWNER hp352      <- the nest does NOT open the gate
t=41293  C sh1 ammo9 | SPAWNER hp352 | B  <- a biter it emitted DOES
t=41519  C sh0 ammo9 | SPAWNER hp352      <- and it closes again
```

The nest sat at 350–352 hp across the whole window against the ungated
version's 355 → 170, and three magazines went into the biters rather than ten
into the building. That `find_enemy_units` answers with `type = "unit"` only is
the entire reason it is the call being made.

**A falsification sweep of eight mutations caught seven and found one real
defect in my own tests.** `DEFEND_RADIUS = 0` — which disables return fire
completely — left every test green, because the radius assertion compared the
value passed to `find_enemy_units` against `DEFEND_RADIUS` itself and both
sides moved together. Pinned to 12 with its bound stated. The other seven
(never raise the order, hold it permanently, search from the origin, a second
copy of the radius, assume the enemy force is called `enemy`, rewrite every
sweep, order a dead character about) each failed the test that names them.

Baselines unmoved on the final HEAD, same binary: `researched:automation`
176 / 21,784 · `producing:automation-science-pack:6` 316 / 22,457 ·
`producing:logistic-science-pack:6` 441 / 47,478 (`map.json`) ·
`gathered:crude-oil` 2,117 / 325,138 (`map-31337-explored.json`).
`cargo test --workspace --no-fail-fast`: 3,079 passed, 0 failed, cargo's own
exit 0 — the +6 over master's 3,073 is this file's tests.

## What to build next, and the one decision that is not mine

This covers **roaming aggro, and only that** — which is the half that no
routing can avoid. Bot 2 of the oil run died to a small-biter **166 tiles from
anything static**; there is no route around that, and the standing order
answers it for zero actions and zero items. The other half — three
`small-worm-turret` at 20.4, 20.7 and 20.7 tiles from a rock bot 1 was sent to
chop, all three already in the dump the planner read — is a **target-selection
gap, not a combat gap**, and no t=0 personal weapon can close it: the worm
reaches 25 and the pistol 15. `entity_graph.threats` is populated and has no
reader in `crates/planner`. **That is the cheaper next move and it is not a
weapon.**

**The open question, which needs the owner and not an overnight decision:**
`shooting_enemies` chooses its own target, so even a gated order may still pick
a spawner over the biter that is actually biting. The strictly-defensive
alternative is `shooting_selected` aimed at the nearest hostile unit, which
never shoots a structure — but that re-aims only as often as the sweep runs, so
a 60-tick sweep shoots at where the biter was, and tightening the sweep costs
tick rate on every bot. That trade is a policy question, and the owner reserved
policy.

The live nest test above bounds that worry rather than dismissing it: the
spawner stayed at 350-352 hp while its own biters were being killed 10 tiles
away, so in practice the gated order shot the units and not the building. That
is one nest in one window, not a proof.

Also open, and named because it is a real cost: **ammo is finite.** Ten
magazines is a hundred rounds, and the nest window above spent three magazines
in ~2,000 ticks. A bot that fights repeatedly runs dry and falls back to fists,
which still won a one-biter fight here for 14 hp but will not win many.
`firearm-magazine` is craftable at t=0 (4 iron plates, 34 planned actions for
ten) if resupply turns out to matter.

Landed on `mods/BotBridge/control.lua` after the death-recovery branch merged
(`1eeeb3ac`), rebased onto it rather than developed alongside.
