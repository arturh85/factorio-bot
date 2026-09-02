# Fixing B1: entity query payloads dominated by trees

Date: 2026-09-02
Scope: `crates/core/src/factorio/rcon.rs`, `crates/scripting_lua/src/globals/record.rs`,
`crates/scripting_lua/src/globals/rcon.rs`, `crates/server/src/game/query.rs`.
Finding: B1 in `docs/superpowers/notes/2026-09-02-factorio-21-api-audit.md`.

## What was measured

Independently re-derived the audit's entity-type distribution from
`workspace/server-log.txt` (the stdout `writeout_entities` path, whole
discovered map, 473 chunks):

```
10542 tree
 2693 resource
  326 simple-entity
  229 explosion
  177 corpse
   ...
   27 unit
   20 fish
```

Trees are ~79.7% of the "natural terrain" population on this map, matching
the audit's 10,510/2,681 figures (small differences are just a slightly
different log snapshot). This confirms the *composition* problem is real.

The RCON keyframe query (`record.rs::keyframe_snapshot`) is a *different*
code path from the stdout writeout, and its raw (pre-filter) wire payload is
not captured by any existing artifact — `map.jsonl` only records the
post-filter `game` list, and nothing logs RCON traffic byte counts. I did not
run Factorio to generate one (per constraints). Cross-checked instead against
`workspace/runs/*/map.jsonl`: 27 recorded keyframes, average query area
~2443 tile², average post-filter `game` entity count ~450. The type filter
added below removes tree/fish/unit/character/dropped-item records from the
wire *by construction*, in every keyframe query, regardless of local density —
it cannot make any reply larger, and on the map's own numbers (79.7% of the
non-relevant-type population is trees alone) the removed fraction is large in
the common case of unfiled or partially-cleared terrain near a query box.

## Fix 1: `find_entities_filtered` can now filter by a list of types

`EntitySearchFilters.type` accepts a string *or array of strings* [V].
`FactorioRcon::find_entities_filtered`'s `search_type` was `Option<String>`,
so no caller could ask for more than one type without either issuing several
RCON calls or fetching everything and filtering in Rust after the wire.

Widened to `Option<Vec<String>>` (`crates/core/src/factorio/rcon.rs`). All
call sites updated:

- `crates/scripting_lua/src/globals/record.rs` (`keyframe_snapshot`) — the
  one consumer that actually wanted this. It now sends
  `keyframe_relevant_types()`: every `EntityType` `keyframe_relevant` already
  admits (`Furnace`, `Inserter`, `Boiler`, `Lab`, `OffshorePump`,
  `MiningDrill`, `StorageTank`, `Container`, `Splitter`, `TransportBelt`,
  `UndergroundBelt`, `Pipe`, `PipeToGround`, `LogisticContainer`,
  `AssemblingMachine`, `Resource`) plus `simple-entity` (needed for
  `rock-big`/`rock-huge`, which the game's `type`+`name` filters cannot OR
  together — they narrow the same query, not offer alternatives). The
  existing `keyframe_relevant` name-check still runs client-side on what
  comes back, now doing only the small `simple-entity → rock-big/rock-huge`
  narrowing instead of discarding ~80% tree/fish/unit noise after it already
  crossed the wire.
- `crates/scripting_lua/src/globals/rcon.rs` (`rcon.find_entities_in_radius`,
  a user-Lua-facing binding) and `crates/server/src/game/query.rs`
  (`GET /api/v1/game/find-entities`) still expose a single `entity_type`
  string at their surface — unchanged public contract, just widened to
  `Some(vec![t])` internally. No OpenAPI/Lua-doc change needed.
- `crates/core/src/factorio/snapshot.rs::attach_world` and the other
  `rcon.rs` call sites (`is_area_empty`, water-avoidance in
  `place_entity_timed`/blueprint placement) were **left unfiltered on
  purpose**. `attach_world` feeds `EntityGraph::add`, whose `blocked_tree`
  keys placement refusals off *every* collidable entity including trees —
  confirmed by reading `entity_graph.rs`'s `add()`, which inserts every
  nonzero-collision-box entity except resources/rails into `blocked_tree`,
  separately from the curated `entity_tree` whitelist. Filtering trees out of
  `attach_world`'s query would reintroduce the exact forest-siting bug the
  brief warned about (`blocking_boxes_within`). Documented this reasoning
  directly on `find_entities_filtered`'s doc comment so the next reader
  doesn't "fix" it the wrong way.

## Consumer map (who needs which types)

| Caller | Types needed | Why |
|---|---|---|
| `record.rs::keyframe_snapshot` | the 16 `entity_tree`/`resource_tree` types + `simple-entity` | Compares game vs. `EntityGraph::snapshot_within`, which only ever holds these |
| `snapshot.rs::attach_world` | all (no filter) | Feeds `EntityGraph::add`, whose `blocked_tree` needs every collidable entity, trees included, for placement refusal checks |
| `rcon.rs::is_area_empty` | all (no filter) | Answers "is anything at all here", by design |
| `rcon.rs` placement retry / blueprint build-area checks | all (no filter) | Same reason as `is_area_empty` — obstruction checks, not model population |
| `globals/rcon.rs` Lua binding / HTTP `find-entities` | caller-chosen (single type) | Generic query surface; unchanged |

## Fix 2: `find_entities_filtered` / `find_tiles_filtered` moved to `remote_call_json`

Read both `remote_call` (`rcon.rs:794`, unchanged) and `remote_call_json`
(`rcon.rs:888`). What the completeness check buys: this RCON client reads
exactly one packet per command (`enable_factorio_quirks(true)`), so a reply
split across packets is truncated in-band with no error from the `rcon`
crate — `cmd()` returns the first packet's body as a plain `Ok`. `remote_call`
returns that truncated text straight to the caller; the only symptom is a
`serde_json::from_str` parse failure downstream, and critically the *pooled
connection* is still returned to `bb8` holding the unread remainder, so the
next command issued on it reads someone else's reply — cross-talk, not just a
one-off parse error. `remote_call_json` runs a cheap `IgnoredAny` completeness
parse before handing the text back, and calls `conn.mark_desynced()` on a
short read so `ConnectionManager::has_broken` drops the connection instead of
recycling it.

Both `find_entities_filtered` and `find_tiles_filtered` returned unbounded,
area-based replies (not fixed-size like `players` or `player_force`) via the
plain `remote_call` path — exactly the shape `remote_call_json` exists for.
`attach_world`'s unfiltered 400×400-tile query is the highest-risk caller
(order of ~900 kB scaled from the audit's 2.8 MB/473-chunk measurement,
larger than the 767 kB `world_snapshot` reply that already goes through
`remote_call_json`). Moved both functions to `remote_call_json`. Behavioural
note: on a non-JSON reply (e.g. "no such function" from an old mod), the
error type changes from `RconReplyNotJson` (produced by `parse_reply` after a
failed parse) to `RconUnexpectedOutput` (produced by `remote_call_json`
itself) — still an `Err`, just a different variant/message; no test depended
on the old variant for these two calls.

## Silent truncation

No `limit` parameter was added. None of the identified callers need one:
`record.rs`'s query is now bounded by *type*, not count, and its box is
already sized from the run's own placements; `attach_world` is deliberately
bounded by *area* (`DEFAULT_ATTACH_RADIUS`), not count; `is_area_empty` and
the placement/blueprint obstruction checks only need to know "empty or not".
A `limit` with no accompanying "was this everything" signal is exactly the
kind of silently-precise field this project has already been burned by, so I
did not introduce one. The `remote_call_json` switch is the mechanism that
turns "reply was too big" into a loud, attributable error instead of a
quietly short list.

## Mod changes

None needed. `mods/BotBridge/control.lua`'s `rcon_find_entities_filtered`
(`control.lua:2904`) already passes its `filters` table straight through to
`game.surfaces[1].find_entities_filtered(filters)` — an array-valued `type`
field works with zero mod-side changes. (Not touched — another agent owns
`control.lua` right now.)

## Determinism

No change touches what entities exist in the world model or in what order —
`entity_tree`/`blocked_tree`/`resource_tree` population is untouched
(`attach_world` still fetches everything). The change only narrows what one
RCON reply *carries* for one already-filtering consumer (`record.rs`'s
keyframe comparison), which is diagnostic/recording code, not planner input.
`expansion_is_deterministic` and the planner's pure state are unaffected.

## Verification

- `nix develop --command cargo build --workspace --all-features` — clean.
- `nix develop --command cargo clippy --workspace --all-features --all-targets -- --deny warnings` — clean, zero warnings.
- `nix develop --command cargo fmt --all -- --check` — clean.
- `nix develop --command cargo test --workspace --all-features` — all 23 test
  binaries passed, 0 failed (unit + integration + doctests). No test's
  expectations moved; none needed to.
