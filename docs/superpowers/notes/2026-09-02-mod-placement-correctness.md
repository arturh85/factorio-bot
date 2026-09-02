# Mod placement correctness: audit findings A3 and A2

Date: 2026-09-02
Scope: `mods/BotBridge/control.lua` (repo copy only — `workspace/mods/` was not
touched, a live run may be using it), plus one new Rust integration test.
Source: `docs/superpowers/notes/2026-09-02-factorio-21-api-audit.md`, findings
A3 and A2.
Authority for every API claim below: `workspace/factorio-api-docs/runtime-api.json`
(`application_version: 2.1.17`, `api_version: 6`).

---

## 1. What the two API calls actually return

Read off the class that **returns** the value, not the class that owns the call
site — the trap this project has hit twice.

| Call | Declared on | Returns |
|---|---|---|
| `remove_item(items)` | `LuaControl` (inherited by `LuaPlayer`) | `uint32` — "The number of items that were actually removed." Not optional. |
| `create_entity{…}` | `LuaSurface` | `LuaEntity`, **optional** — "The created entity or `nil` if the creation failed." |

Both were discarded. That is two bugs in two lines:

* `create_entity` returning `nil` after the item was already taken **destroys
  the material** — item gone, nothing built.
* `remove_item` returning `0` **builds for free** — the entity exists and
  nobody paid.

Two more entries that decided the shape of the fix:

* `LuaControl::get_item_count(item?)` — "Get the number of all or some items in
  this entity", i.e. across every inventory the player has. It is the matched
  pair for `remove_item`, which also acts on the whole entity. The
  affordability check and the spend therefore range over the same set, which is
  what makes a disagreement between them meaningful rather than routine.
* `LuaSurface::create_entity`'s `raise_built` — "If true;
  `defines.events.script_raised_built` will be fired on successful entity
  creation. Defaults to `false`." The mod does not pass it, so the game
  announces nothing and `on_some_entity_created` is called by hand. That is why
  the undo path below needs no matching deletion event.

## 2. The order chosen, and what happens if the second step fails

**Create first, charge second, `destroy()` the entity if the charge falls
short.**

The alternative — charge first, create, refund on failure — has to hand the
item back through `insert`, which itself returns "the number of items that were
actually inserted" and can fall short if the inventory filled in between. A
refund that silently loses material is the original bug with more steps.
Undoing a build cannot half-work: `destroy()` either removes the entity or the
entity was already gone.

If the second step (the charge) fails, the handler destroys the entity it just
created and returns a refusal. Nothing is announced before payment clears:
`on_some_entity_created` — the only thing that tells the Rust `EntityGraph` the
entity exists — now runs *after* the charge succeeds, so the undo path leaves no
phantom in the graph. This closes the "what could break" the audit flagged
against A3 (an `on_some_entity_created` for an entity that is then destroyed)
by never emitting it, rather than by adding a compensating deletion.

Every exit still calls `stamp_tick()`, including the two new ones, so the reply
keeps carrying the tick the game judged at.

## 3. Did `charge_item_to` fit? No.

`charge_item_to(holder_player_id, item)` does
`holder.get_main_inventory().remove({name=item, count=1}) == 1`. Two mismatches:

1. **It charges a narrower set than `rcon_place_entity` checks.** The
   affordability guard immediately above is `player.get_item_count(item_name)`,
   which counts every inventory the player has. `get_main_inventory()` is the
   main inventory alone. Pairing them would put the check and the spend on
   different sets — the exact asymmetry the audit's C1 flags as a defect
   elsewhere — and a placement whose item happened to sit outside the main
   inventory would be created and then destroyed for no reason. `remove_item`
   is the counterpart to `get_item_count`, and that is the pair used.
2. **It resolves a holder that is not this handler's actor.** Its whole reason
   for existing is that in `rcon_place_blueprint` the bot holding the material
   need not be the bot placing the blueprint, so it takes an id and re-looks-up
   the player. `rcon_place_entity` already has the acting player in hand and has
   no third-party holder.

What was reused is the *reasoning*, not the code: check the return, and undo the
build if it is not 1. That is the same pattern `rcon_place_blueprint` already
runs at its `charge_item_to` call sites.

## 4. What the caller sees

The RCON reply keeps its three existing shapes, which
`FactorioRcon::place_entity_timed` (`crates/core/src/factorio/rcon.rs`)
discriminates:

| Reply | Meaning to Rust |
|---|---|
| starts with `{` | success; parsed as the entity |
| `§player_blocks_placement§` | the actor is in the footprint — walk it aside and retry |
| anything else | a refusal; `note_placement_refusal` remembers it **only if** it contains `can_place_entity said 'no'` |

The new failure prints

```
cannot place item '<item>' because taking it from the player '<name>' removed nothing
```

which deliberately joins the existing *material* family ("… because the player
'<name>' does not have any"), not the site family. It does not contain
`CAN_PLACE_REFUSAL`, so it is **not** written into the placement-refusal ledger
— correctly: an empty hand says nothing about the ground, and remembering it
would fence the planner out of a site the game never refused. It is not the
sentinel either, so no walk-and-retry is triggered. Reply length is unchanged
(one message plus the `§tick§…` stamp), so the "unparseable replies report what
they contained" path (`RconUnexpectedOutput`) is not perturbed.

No new `writeout(...)` kind was introduced, so `output_parser.rs` needs no
change and no action can hang to its 360-second deadline.

## 5. A2 — `force_build` → `build_mode`

`LuaItemCommon::build_blueprint` in 2.1.17 declares exactly:

```
surface, force, position, direction, build_mode, skip_fog_of_war, by_player, raise_built
```

There is no `force_build`. An unknown key in a `takes_table` call is simply not
read, so the flag has been discarded since the 2.0 port and the parameter's
default applied instead. The parameter's own documentation, verbatim from
`runtime-api.json`:

> If `normal`, blueprint will not be built if any one thing can't be built. If
> `forced`, anything that can be built is built and obstructing nature entities
> will be deconstructed. If `superforced`, all obstructions will be
> deconstructed and the blueprint will be built.

Default: `defines.build_mode.normal`. `defines.build_mode` has exactly three
values: `normal` (0), `forced` (1), `superforced` (2).

So **`force_build = true` maps to `defines.build_mode.forced`** and
`force_build = false`/nil to `defines.build_mode.normal` — and the status quo
was the *opposite* of what a caller asking for `force_build = true` wanted:
all-or-nothing instead of build-what-you-can.

Implemented as one helper, `blueprint_build_mode(force_build)`, used by both
call sites (`rcon_place_blueprint` and `rcon_cheat_blueprint`) so the two cannot
drift. `superforced` is intentionally unreachable from the boolean: it
deconstructs *all* obstructions, a bigger promise than any caller here made.

The nearby comment above `placement_check_args` that discusses `build_mode` is
about `LuaSurface::can_place_entity`, a different method with no such parameter;
it is correct as written and was left alone. The new comment says so explicitly
so the next reader does not merge the two.

### Behaviour change to be aware of

`forced` also deconstructs obstructing *nature* entities, which 1.1's
`force_build = true` did not. A blueprint placed over trees will now clear them.
Given this project's interest in honest material accounting, that is a
deliberate consequence of asking for what the caller asked for, not an
accident — but it is a real change from what runs have been getting (which was
`normal`, i.e. refuse the whole blueprint).

`raise_built` was **not** added. `by_player` already causes `on_built_entity` to
fire and the mod's `on_some_entity_created` is registered for it; adding
`raise_built` would report every blueprint entity twice.

## 6. What is unverified without a live game

* **A2 is unverified against a real Factorio.** Blueprint placement has not been
  exercised by any run in this project, so there is no live evidence at all —
  neither that the old flag was being ignored in practice nor that
  `build_mode = forced` behaves as the docs say. The claim rests entirely on
  `runtime-api.json` plus the tests below, which prove only what argument table
  the mod *hands* the game.
* Whether `forced` clearing trees is desirable for this project's runs is a
  product decision nobody has made with data.
* For A3: that `create_entity` really returns `nil` (rather than raising) for
  the specific refusals this project hits, and that `remove_item` really can
  return 0 after `get_item_count` returned nonzero (quality mismatch requires
  Space Age; a same-tick inventory change is the vanilla route). The fix is
  correct either way — it is defensive — but the frequency of the second case is
  unmeasured.
* The tests stub `serialize_entity`; the serialisers have their own coverage in
  `crates/core/tests/botbridge_serialisers.rs`.

## 7. Tests

New file `crates/core/tests/botbridge_placement_material.rs` — six tests that
load the real `mods/BotBridge/{types,control}.lua` into a Lua 5.4 state on top
of a stub game that *counts* what the handler did (creates, charges, destroys),
which is what no reply body reveals. Modelled on the existing mod-source harness
in `rcon.rs`'s `transfer_guarantee_tests` (that module was not touched — another
agent is working in that file).

1. `a_failed_creation_charges_the_player_nothing`
2. `a_charge_that_takes_nothing_undoes_the_build`
3. `an_unpaid_placement_reads_as_a_material_refusal_not_a_site_refusal`
4. `a_successful_placement_charges_exactly_one_item`
5. `force_build_reaches_the_game_as_defines_build_mode_forced` (both handlers)
6. `no_force_build_reaches_the_game_as_defines_build_mode_normal` (both handlers)

Checked against the pre-fix `control.lua` (`git show HEAD:…` into place, run,
restore): **5 of the 6 fail**, the sixth being the happy path, which the old
code also got right. So the suite is red on the bug and green on the fix.

## 8. Gates

* `luac -p mods/BotBridge/control.lua` — OK
* `cargo fmt` — clean
* `cargo clippy --workspace --all-features --all-targets -- --deny warnings` — clean
* `cargo test --workspace` — all green (the tree also carries another agent's
  in-progress edits to `crates/core/src/factorio/rcon.rs`,
  `crates/scripting_lua/src/globals/{rcon,record}.rs` and
  `crates/server/src/game/query.rs`; those were not touched and were green too)

## 9. Not done

* A1, A4, A5, A6 and the Rank B/C findings from the audit are untouched.
* `workspace/mods/BotBridge/control.lua` is unchanged, so a debug run will log
  the mods-drift line until the workspace copy is refreshed or deleted.
