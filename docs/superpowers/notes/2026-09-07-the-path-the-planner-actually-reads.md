# The path the planner actually reads

**2026-09-07.** The entity record gained `input_inventory` earlier the same day,
so a furnace holding ore became distinguishable from one that never received
any. That landed on `serialize_entity` — the path that *describes* the world.
It did not land on `rcon_inventory_contents_at`, the on-demand reply
`Planner::refresh_buffers` pulls and `FactorioSurface::observe_inventories`
ingests.

**So a furnace's ore was visible when you dumped every entity and invisible
when you asked about that one furnace** — and the second is the only one the
planner's own model is built from. The gap was real; this closes it.

## What the planner does with the reply, established before changing anything

One RCON reply carries three inventories and they land in three different
places, with three different standings:

| reading | lands in | standing |
|---|---|---|
| `output_inventory` | `PlanState::buffers` | **material a plan may spend** (`Withdraw`) |
| `fuel_inventory` | `PlanState::fuel` | a **credit**: a top-up pays for what is already burning, less this |
| `input_inventory` | *nothing, until now* | — |

`Buffer`'s own doc already says why fuel is not a buffer: taking coal out of a
running machine stalls the machine the plan may be waiting on. The same
argument covers the input slot, and one stronger one: **`withdraw_slot` maps a
furnace to its *result* slot**, which is the only inventory an
`ActionKind::Remove` this planner emits can address. Ore in an input slot is
not reachable by any action we have.

**So the double-spend seam is untouched, by construction.** The input reading
never becomes a `Buffer`, `has_buffers()` is unmoved, and
`a_furnace_holding_only_ore_is_kept_and_is_not_a_buffer` pins that. Nothing
counts a furnace's ore as available material — before this change or after it.

## What it is for, and the one thing deliberately not done

`crate::method::have`'s `adoptable_furnaces` filters standing furnaces with
`!state.holds_buffer(pos)` and calls what survives **`Reuse::Idle` — "nothing
in this plan has loaded it. Take it as it is."**

A furnace mid-smelt has ore in and nothing out. `holds_buffer` is `false` for
it, so **the planner reads a busy furnace as an idle one** and queues another
batch into it. That is the concrete blind spot the missing reading created.

`PlanState::holds_input` is the evidence that separates the two. **Acting on it
in `adoptable_furnaces` is deliberately left undone**, for two reasons:

- It is a **policy** change, not a plumbing one. Excluding a busy furnace makes
  a plan place another one instead, which costs stone, and in a replan where
  every furnace is mid-smelt it could turn an adoption into a materials
  shortfall. That trade wants a live measurement.
- **No offline baseline can measure it.** `world.dump` never calls
  `refresh_buffers`, so a dump's `inventories` is always `[]` and every
  `PlanState` built offline has an empty `input` map. The whole path is
  unreachable from the offline loop.

A weaker argument for the change, worth recording because it is the honest
counterweight: `holds_buffer` already excludes that same furnace thirty seconds
later, once plates appear in its result slot. Excluding it now is consistent
with what the code already does, not a new policy — it just needs a run to say
so.

## Absent, empty and never-asked stay three answers

The rule this project keeps paying for. Four layers, each of which could have
collapsed it:

- **The mod** omits the key entirely when `input_inventory_index` has no row
  for the entity's type (every chest), and sends `{}` for a furnace with an
  empty slot. It does not send an empty table for the first case.
- **`InventoryResponse.input_inventory`** is `Box<Option<Vec<..>>>` with
  `serde(default)`, so a missing key — including from a mod build older than
  the field — reads `None` and does not fail to deserialize.
- **`ObservedInventory::input`** is `Option<BTreeMap<..>>`, and is the one
  field of that struct that keeps the distinction. `output` and `fuel` are bare
  maps; that is survivable for them because every reader asks "how much of X",
  and both answers are zero. It is not survivable here: `refresh_buffers`
  queries `wooden-chest` alongside `stone-furnace`, and "this machine cannot
  hold ore" is not "this machine is waiting for ore".
- **`PlanState::input`** is `BTreeMap<Pos, Option<..>>`, so **three** answers
  survive to the planner: no entry (nobody asked), `Some(None)` (answered, no
  such slot), `Some(Some(map))` (answered, here are the contents).
  `holds_input` is the verdict and `input_reading` is the raw evidence beside
  it — the shape `WalkSettled` uses for `error` beside `failure`.

## What the baselines can and cannot prove

Re-measured on one release binary built from this branch:

```
researched:automation            176 actions / 21,784 ticks
producing:iron-plate:30          316 /  22,457
producing:logistic-science-pack:6  441 /  47,478
gathered:crude-oil (explored)   2,115 / 317,283
```

Byte-identical to the pre-change baselines. **That is expected and it proves
almost nothing about this change.** An offline plan is made against a dump
whose `inventories` is `[]`, so `observe_inventories` is never called, `input`
is empty on every `PlanState`, and none of the new code runs. Four unmoved
numbers are evidence that **nothing else was disturbed** — which is worth
having, and is all they are.

What does exercise the path: the mod-level Lua tests, which drive
`rcon_inventory_contents_at` in a stub game and read the record it hands to
`table_to_json`; the serde tests on the three wire shapes; and the
`observe_inventories` / `from_world` tests that carry a reading end to end.

## And a live query, because only a live query can settle an on-demand path

`scripts/input_inventory_probe.lua`, headless, one character bot, isolated
instance (`headless-d.toml`), debug build so the mod under test is the one that
loads. Furnace and chest cheated in and disclosed; a shape probe, not a
measurement.

```
[1] stone-furnace keys: fuel_inventory, input_inventory, name, output_inventory, position
     input_inventory is a table: true
[2] wooden-chest  keys: fuel_inventory, input_inventory, name, output_inventory, position
     input_inventory is a table: false
furnace input_inventory iron-ore = 33
```

The chest is the control and it is the important half: **if both answered the
same way the field would be worthless.**

Two details worth keeping:

- **33, not the 34 inserted.** The furnace is fuelled before the ore goes in,
  so it starts smelting in the ticks between the insert and the query. The
  probe's first run asserted `== 34` and printed `PROBE FAILED` — **the
  assertion was wrong, not the mechanism**, which is the cheaper half of this
  file's usual lesson.
- **The chest's record HAS an `input_inventory` key in Lua**, and it is not a
  table. The reply passes through `InventoryResponse` on the way back, so
  Rust's `None` reaches Lua as mlua's null sentinel — light userdata, and
  therefore **truthy**. A script must type-guard (`type(x) == "table"`); `or {}`
  does not substitute a default here and never has.
