# The census lands

**2026-09-07**, branch `the-census-lands-and-a-furnace-names-its-recipe`, off
`7808d8d6`.

The mod has been able to enumerate `game.surfaces` since `95c183be`
(`collect_surfaces`, `serialize_surface`,
`remote.call('botbridge', 'surfaces')`, a `surfaces` field on
`world_snapshot`, and `FactorioSurfaceInfo` in `crates/core/src/types.rs`).
**Nothing in Rust could receive it.** This is the Rust landing, and nothing
else: the record now says what surfaces a save has. It ingests none of them.

## Where it landed

Three pieces, in the destinations the two handover notes had already
established by measurement — none of them re-derived here.

1. **`WorldSnapshot.surfaces: Option<Vec<FactorioSurfaceInfo>>`**
   (`crates/core/src/factorio/snapshot.rs`), landed by `apply_snapshot` under
   `daylight`'s **non-erasure rule**: a sender that says nothing does not
   overwrite what a newer one said, or "this build is old" silently becomes
   "nobody ever looked".
2. **`GameGlobals.surfaces`** (`crates/core/src/factorio/globals.rs`), read
   back through **`FactorioWorld::surface_census()`**. Not on `FactorioWorld`
   itself: the separation agent established that `FactorioWorld` is
   constructed in exactly one place (`process_control.rs:246`) and reached
   only through `FactorioInstance`, while `output_parser` and `snapshot` hold
   a `FactorioSurface` and nothing above it. `GameGlobals` is the aggregate
   they can reach (`surface.globals`), and it is game-global for the same
   reason a surface list is — a surface cannot hold the list of surfaces.
3. **The stdout half**: `writeout_surfaces()` in `writeout_initial_stuff`
   (`mods/BotBridge/control.lua`) **plus its `"surfaces"` arm in
   `output_parser.rs`, in this same commit.** That constraint was hard and is
   the reason the mod side shipped without it: the parser logs
   `unexpected action: <key>` as an **error** for a writeout key it has no arm
   for, so emitting the census first would have put a red line in every run
   that looks like a defect and is not. `control.lua` carried a comment saying
   exactly that at the point the function now occupies.

A dump carries it too (`FactorioSurface`'s manual `Serialize`/`Deserialize`),
for the reason `daylight` is there: an offline plan has no game to enumerate,
so a dump that dropped the census would make the question unanswerable rather
than merely stale.

## The arm is all-rows-or-none, deliberately unlike its neighbour

`entity_prototypes` directly above it `filter_map`s a bad row away. That is
right for a prototype table — missing one entry is *degraded*. A **census**
missing one row is wrong in the exact direction the field exists to prevent:
it under-reports the surfaces a save has while reading as a complete answer.
A census that cannot be parsed stays `None`.

## Absent is not empty, at four seams

`None` is *nobody enumerated* — every archived dump, and every BotBridge older
than the field. An empty list would claim a running game has no surfaces,
which cannot happen. That distinction is asserted, not merely documented:
`absent_and_empty_are_different_values`, the dump's two-levels-of-absence
collapse, the snapshot's non-erasure, and the parser's refusal.

And **`planet: None` is a real answer, not absence of one**: a space platform
is a surface that is not a planet. Half of the world-record save's surfaces
take it, so it is a live case rather than a defensive one.

## Falsification

Nine breaks against the shipped code, one at a time, each asserting its
substitution matched **exactly once** and restoring before the next
(`scratch/falsify.py`, not committed).

| # | break | subs | red |
|---|---|---|---|
| A | `apply_snapshot` erases the census when the sender says nothing | 1 | 1 |
| B | the parser drops bad rows instead of refusing the census | 1 | 1 |
| C | the parser arm is renamed away | 1 | **2** |
| D | `WorldSnapshot.surfaces` misspelt on the wire | 1 | 1 |
| E | the dump drops the census | 1 | 1 |
| F | an absent census loads as an empty one | 1 | 1 |
| G | `FactorioWorld::surface_census` answers `None` | 1 | 1 |
| H | `writeout_surfaces` is never called at init | 1 | 1 |
| I | the census is emitted as a bare `print`, not a `writeout` | 1 | **2** |

C and I are honest rather than redundant: C removes the stdout arm entirely,
which is the whole feature, and I breaks the `§tick§key§` envelope that two
tests read through.

**The sweep's own first run was a broken experiment, and it is worth
recording.** It reported `COMPILE ERROR` for all nine, including the two Lua
breaks, which cannot compile-fail. The detector treated cargo's
`error: test failed, to rerun pass ...` — cargo reporting a **red test** — as a
compile failure, so every genuine result was swallowed by the branch meant to
catch a broken experiment. Nine identical lines are themselves the tell: a
mutation sweep whose rows do not differ is measuring the harness.

## Verification

- `cargo test --workspace` — **106 test blocks green, exit 0** (master carries
  105; this branch adds `surface_census_lands.rs`). Run with the command's own
  exit code captured, not a pipeline's.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  clean.
- `git status --porcelain crates/` was checked first, as instructed: the only
  dirty paths were this branch's own.
- **`pnpm run test:coverage` — green** (97.18% statements, gate satisfied).
  **`pnpm lint` fails in this worktree and passes in the main checkout**, on
  `error TS2688: Cannot find type definition file for 'web-bluetooth'`. That
  is environmental, not this change: a fresh `pnpm install` in the worktree
  resolved `app/node_modules/@types/` to `node` alone, where the main
  checkout has eight entries including `web-bluetooth`. No frontend file was
  touched here, and the main checkout's `pnpm lint` exits 0.

### Baselines — four, byte-identical, on one binary I built and measured

Release build of this branch (`--no-default-features --features cli,lua`),
`--bots 1,2,3,4`, seed 31337, Factorio 2.1.17, default settings.

| goal | world | expected | measured |
|---|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 | **176 / 21,784** |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 | **316 / 22,457** |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 | **441 / 47,478** |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,115 / 317,283 | **2,115 / 317,283** |

`gathered:crude-oil` still refuses on `map.json`, naming the charting radius —
correct.

The binary was interrogated rather than the game, because `include_dir!` has
no `rerun-if-changed`:

```
strings -a target/release/factorio-bot | grep -c writeout_surfaces   2
strings -a target/release/factorio-bot | grep -c collect_surfaces    4
```

### What the baselines cannot prove

They prove a serialisation-only change moved nothing, which is what such a
change must do. They prove **nothing about the census**, and cannot: **no
archived dump carries one.** `map.json` and `map-31337-explored.json` were
both written before the field existed and load with `surfaces: None` — the
correct reading, and also exactly the reading a broken implementation would
produce. **The tell for a plumbing failure is uniformity**, and on the offline
basis the field is uniformly absent for the honest reason, so the tell is
unavailable there.

**No live probe was run by this session.** The box was busy with a peer's
release test run and a peer was waiting on a server start, and the coordinator
descoped mid-task and asked for the files released promptly. So the live
numbers below are **the previous session's measurement, cited, not mine**: the
world-record save answers **ten surfaces** — nauvis, gleba, vulcanus, fulgora,
aquilo and five space platforms — through
`remote.call('botbridge', 'surfaces')`
(`2026-09-07-two-things-the-mod-could-not-say.md`). That probe exercised the
mod half only; **the Rust arm added here has never been crossed by a running
game.** The cheapest confirmation is any headless run on this branch: the
census now rides `writeout_initial_stuff`, so one line with key `surfaces`
should appear in server stdout and `FactorioWorld::surface_census()` should
answer `Some` afterwards. A run whose census reads `None` after that means an
old mod, never "no surfaces".

## Scope, and what was deliberately not touched

The Nauvis guard in `on_chunk_generated` stays. `game.surfaces[1]` in the
chunk replay stays. `FactorioWorld` still holds one surface, and
`a_census_written_through_a_surface_is_read_back_through_the_world` asserts
both halves at once: the census says ten, `world.len()` says one, and the
disagreement is now **a statable fact about this bridge** rather than a
silence.

**A second task on this branch, "a furnace should name its recipe", was
descoped by the coordinator mid-flight and is not here.** One finding from it
survives and is worth not re-deriving, read out of
`workspace/client1/doc-html/runtime-api.json` (2.1.17):

* **`LuaEntity.recipe` does not exist.** The current recipe is
  **`LuaEntity.get_recipe()`, a METHOD**, `subclasses: ["CraftingMachine"]`,
  returning `(LuaRecipe?, LuaQualityPrototype?)`. `types.lua` already calls it
  correctly for `assembling-machine` and for assembling-machine ghosts; the
  gap is that `entity.type == "furnace"` reaches neither arm.
* **`LuaEntity.previous_recipe` is an ATTRIBUTE with
  `subclasses: ["Furnace"]`**, `RecipeIDAndQualityIDPair` — *"the previous
  recipe this furnace was using, if any"*. So a furnace standing idle can
  still name what it was smelting, which `get_recipe()` alone cannot, and
  whoever takes this should decide deliberately which of the two (or both) a
  `recipe: null` on 1,215 furnaces should become.
* **Whether `CraftingMachine` covers `furnace` at runtime was NOT established
  here** — the API JSON lists the subclass name and nowhere enumerates its
  members. `crafting_progress` and `products_finished` carry the same
  subclass tag and the mod reads neither off a furnace, so this needs one live
  `get_recipe()` call against a furnace before anything is built on it.
