# A pole says how far it throws

2026-09-07. `mods/BotBridge/types.lua`, `crates/core/src/types.rs`,
`crates/planner/src/state.rs`.

The follow-up to `2026-09-07-a-pole-says-how-far-it-reaches.md`, which found
that `pole_wire_reach` — a hand-kept table of `maximum_wire_distance` — read
**30.0** for `big-electric-pole` against the game's **32**, corrected it by
hand, and could go no further because no field existed to derive it from. The
owner's ruling was to remove the possibility of it happening again by sending
the real field. That is this change.

`FactorioEntityPrototype::maximum_wire_distance` now crosses the bridge, and
`pole_wire_reach` reads it, with the four vanilla numbers kept as an
explicitly-documented fallback under the name `vanilla_pole_wire_reach`.

## Attribute or method: a METHOD, and the attribute spelling exists

`get_max_wire_distance(quality)`, checked against this install's
`runtime-api.json` rather than recalled. There is **no `maximum_wire_distance`
attribute** on `LuaEntityPrototype` in 2.1.17.

`maximum_wire_distance` is the **data-stage** name — it is what
`base/prototypes/entity/entities.lua` writes, and it is what anybody reaching
for this would try first. Reading it as an attribute raises, the mod's `pcall`
swallows it, and the field arrives nil for every prototype in the game with
nothing anywhere saying it should not have. That is how `crafting_speed`
arrived nil for 1,028 prototypes, and `get_supply_area_distance` nearly
repeated it.

The serialiser fixture holds that shut by carrying a **decoy attribute** with
the wrong number: a reader that went back to the attribute gets 30 and fails,
rather than getting *some* number and passing.

## Table against prototype, per pole type

Read off a live 2.1.17 headless game, seed 31337, all 1,028 entity prototypes
dumped through `world.dump`:

| prototype | table said | the game says | |
|---|---:|---:|---|
| `small-electric-pole` | 7.5 | 7.5 | agrees |
| `medium-electric-pole` | 9.0 | 9.0 | agrees |
| `big-electric-pole` | 32.0 | 32.0 | agrees |
| `substation` | 18.0 | 18.0 | agrees |

All four agree — but only because the **32 was corrected by hand yesterday**.
Against the table as it stood two days ago this row would have read 30 against
32. So the pair of tables is now one checked, one caught, and neither
maintained by hand any more.

## The field is NOT "is this a pole", and that is measured

`get_max_wire_distance()` is the maximum over **every** wire kind, so a machine
reports its *circuit* wire distance. Over the same 1,028 prototypes:

```
  4  report a pole's copper span   7.5 / 9 / 32 / 18
 94  report a CIRCUIT wire distance  stone-furnace 9, wooden-chest 9,
                                     power-switch 10, agricultural-tower 30,
                                     every assembling machine 9
930  report 0                        trees, explosions, corpses, segments,
                                     and steam-engine
```

**A caller that read a positive number here as "this is a pole" would find 98
poles in vanilla** and would wire a network through an assembling machine. The
`entity_type == "electric-pole"` gate in `pole_wire_reach` is what stops it,
and it is the same gate `pole_supply_half_extent` uses. Its falsification —
removing the gate — kills exactly the test written for it.

This corrected the change's own documentation. The Rust field doc, the mod
comment and one test all said `Some(0.0)` meant "a furnace, a chest"; the live
game says a furnace reports **9**. The test now uses a `tree-01`, which is one
of the 930. **An inference about Factorio data is not a measurement**, again.

## Absent stays distinguishable from zero

- `Some(d)` with `d > 0` — the game's number.
- `Some(0.0)` — the game saying nothing connects to this. Sent, not dropped.
- `None` — **the sender did not say**: every dump and snapshot written before
  today, `workspace/scripts/map.json` included. Only this falls back.

The mod sends the field for **every** entity, unlike `supply_area_distance`,
because this method carries no `subclasses` restriction and answers 0 rather
than raising — so there is nothing for the `pcall` to gate on, and a collector
that dropped the zeros would be deciding what a pole is in Lua, where no Rust
test can see it.

## The fallback survives, and the reason is measured, not cautious

`vanilla_pole_wire_reach` keeps the four numbers for the same reason
`vanilla_pole_supply_half_extent` keeps its: deleting the *supply* shim with
nothing else changed made all three offline goals refuse to expand at all, and
the refusal blamed **the water**, one layer downstream, with no mention of
poles. Every archived world predates the field, so every one of them reads
`None` here.

## Four baselines, unmoved, and what they cannot prove

One binary each, built from the same target directory, `--bots 1,2,3,4`;
`before` is `91af5903` (this branch's parent), `after` is this change.

| goal | before | after |
|---|---|---|
| `researched:automation` | 176 / 21,784 | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,457 | 316 / 22,457 |
| `producing:logistic-science-pack:6` | 441 / 47,478 | 441 / 47,478 |
| `gathered:crude-oil` (explored map) | 2,115 / 317,283 | 2,115 / 317,283 |

Byte-identical, and `gathered:crude-oil` still refuses on `map.json` —
correct, not a regression.

**They prove the fallback is intact and prove nothing about the field.** Every
measured goal is pre-`electric-energy-distribution-1`, so a big pole never
appears in any of them; and every archived dump predates the field, so
`maximum_wire_distance` is `None` on all four and the new code path is never
entered. The field's behaviour is carried by the unit tests and by the live
probe above, not by these numbers.

## Falsification

Six mutations, one at a time, each asserting its own substitution count is
exactly 1 *in the breaking edit*, each run asserting the target test actually
**ran** under the mutation. All six killed exactly one test.

One of them was a broken experiment first, and it found real redundancy. A
test written here for "an absent field falls back to vanilla 32", through
connectivity, turned out to be `two_big_poles_are_wired_at_thirty_one_tiles`
in different words — the fixture ships every pole with the field absent, so
that pre-existing test *is* the shim's live case. Both died to the same
mutation. The new one was deleted rather than kept.

## And the trap that ate the first live probe

The first headless run reported `maximum_wire_distance` **absent for all 1,028
prototypes** — precisely the uniform-absence tell this project has already
recorded twice for a swallowed method read.

It was neither. **`include_dir!` has no `rerun-if-changed`**, so the release
binary embedded the mods snapshot from before the edit; `strings` on the binary
found `get_max_wire_distance` **zero** times, and the extracted
`workspace/wire-a/mods/BotBridge/types.lua` was the old file. Touching any
source file in `crates/core` forced the re-embed and the field appeared.

This is the stale-binary entry in `CLAUDE.md` from a third side: there, the
*binary* was new and the *mod* was old, and here the binary is new and the mod
it carries is old. **The tell is the same and the fix is not**, so check the
binary itself (`strings -a <bin> | grep <new symbol>`) before concluding
anything about what the game reports. The probe ran in an isolated workspace
(`workspace/wire-a.toml`, ports 4351/34251/7551); the shared
`workspace/mods/BotBridge` symlink was never touched and still points at the
main checkout.
