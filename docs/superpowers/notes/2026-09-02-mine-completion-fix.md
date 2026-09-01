# Mine completion signal: false success from cross-player crediting

## Root cause

`mods/BotBridge/control.lua`'s `on_mined_entity` (hooked to
`on_player_mined_entity`) used to loop over **every connected bot**:

```lua
for idx, player in pairs(game.players) do
    if storage.p[idx] and player.connected and player.character then
        local mining = storage.p[idx].mining
        if mining then
            if mining.entity == event.entity then
                ...
                mining.left = mining.left - 1
                if mining.left <= 0 then
                    action_completed(event.tick, mining.action_id)
                    ...
```

`event.player_index` (who actually mined) was never consulted — only whether
a bot's *stored* `mining.entity` happened to equal the entity in *this*
event.

`crates/planner`'s `nearest_resource_tile`/`resource_tiles_for`
(`crates/planner/src/method/util.rs`) pick tiles per bot from the modeled
remaining `amount`, with no reservation between bots planned in the same
pass — two bots gathering the same item from the same patch can legitimately
be assigned the *same tile*, each taking a share of it (this is by design:
the tile's total amount comfortably covers both shares). Concurrent mining of
one resource entity by two different players is normal, supported multiplayer
behaviour — each player gets their own `on_player_mined_entity` event, with
their own `player_index`, for their own swings.

But the old loop matched **any** bot whose `mining.entity` equalled the
event's entity, not just the bot named in `event.player_index`. So when two
bots shared a tile, every swing by either bot decremented *both* bots'
`left` counters. Whichever bot needed fewer additional swings hit zero first
— on a mix of its own and the other bot's progress — and reported success
having personally mined only part of its requested count. This matches the
run's evidence exactly: in milestone 2's second iteration, bot 4's `mine 4
copper-ore` (apparently not sharing a tile) settled in 483 ticks against a
480-tick plan (correct), while bot 2's `mine 4 copper-ore` settled in 266
ticks and bot 3's `mine 5 copper-ore` in 362 ticks — both well under their
120-ticks-per-unit plan, consistent with the two of them cross-crediting each
other's swings on a shared tile.

## Fix

`crates/executor`, `rcon_actuator.rs`, and `output_parser.rs` were all
already correct — the defect was entirely inside the mod's completion
bookkeeping, so the writeout format (`action_completed`/`action_failed`) is
unchanged and no parser change was needed.

In `mods/BotBridge/control.lua`:

1. **`on_mined_entity`** now looks up only `storage.p[event.player_index]`
   instead of iterating every bot, so a bot's mining completion depends only
   on its own swings.
2. **Delivered count is now the actual buffer, not an assumed 1 per event.**
   Added `inventory_to_dict(inventory)` (mirrors `products_to_dict` but reads
   a real `LuaInventory`'s `get_contents()`) and changed the decrement from
   `mining.left = mining.left - 1` to `mining.left = mining.left -
   (inventory_to_dict(event.buffer)[mining.prototype.name] or 0)`. This also
   makes `recent_item_additions`' `itemlist` (previously built from
   `products_to_dict(proto.mineable_properties.products)`, the *expected*
   yield) record the *actual* mined buffer.

Verified against `workspace/factorio-api-docs/runtime-api.json`:
`on_player_mined_entity.player_index` ("The index of the player doing the
mining"), `on_player_mined_entity.buffer` (`LuaInventory`, "The temporary
inventory that holds the result of mining the entity"), and
`LuaInventory.get_contents()` (returns `ItemWithQualityCount[]` with `name`/
`count`, aggregating this correctly is what `inventory_to_dict` does).

No `rcon.print` was added or used for debugging; the two `writeout(...)`
lines in `action_completed`/`action_failed` are untouched.

## `products_to_dict`'s expected-vs-actual yield — same bug or separate?

**Separate, and lower-impact than it looked.** `recent_item_additions` (fed
by the buggy `products_to_dict(proto.mineable_properties.products)` call) is
written in three places in `control.lua` but never read anywhere in it, and
`grep -rn "recent_item_addition" crates/` returns nothing — no Rust code
reads it via any RCON getter. It was dead bookkeeping, not the completion
signal: the signal was purely `mining.left = mining.left - 1`, which does not
consult `mining_results`/`products_to_dict` at all. So the "expected yield"
bug did not by itself cause the false-success/under-delivery defect; the
cross-player crediting above did. Both are now fixed at the same call site
(`on_mined_entity`) since fixing the completion accounting required reading
the real buffer anyway, but they are two distinct defects that happened to
live one line apart.

## Verification

- `nix shell nixpkgs#lua5_4 -c luac -p mods/BotBridge/control.lua` — passes.
- `nix develop --command cargo fmt --all -- --check` — clean (no Rust files
  touched).
- `nix develop --command cargo clippy --workspace --all-features
  --all-targets -- --deny warnings` — clean.
- `nix develop --command cargo test --workspace --quiet` — exit 0, all suites
  green (Rust side was unaffected; no Rust code changed).
- No Factorio run was started; `workspace/mods/` was left untouched per
  instructions (a live run may depend on it) — only the repo copy
  `mods/BotBridge/control.lua` was edited. This means the fix is **not yet
  live** in any running `workspace/mods/` checkout; the next fresh workspace
  seed (or a manual sync) is needed before a live run picks it up, per the
  debug-build mod-resolution note in `CLAUDE.md`.
