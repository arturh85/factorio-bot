# A chunk knows which surface it is on — and the parser did not ask

2026-09-07. Branch `a-chunk-knows-which-surface-it-is-on`, off `7808d8d6`.

The task was to lift the mod's Nauvis guard in `on_chunk_generated`. **The
guard is not the blocker, and lifting it would have been wrong.** What follows
is what the investigation found, in the order the findings arrived, with the
measurements beside each.

## 1. How the parser addresses a surface today: it does not

`OutputParser` holds **one `Arc<FactorioSurface>`** and every writeout lands in
it. `read_output` (`crates/core/src/process/output_reader.rs`) builds the
parser, takes `output_parser.world()` back out, and `process_control.rs:246`
wraps *that* in `FactorioWorld::nauvis_only`. The world is built **from** the
parser's surface, so the parser has no handle on the aggregate and no way to
reach a second surface even in principle. `snapshot::attach_world` (the
`--connect` path) does the same thing.

So "routing a chunk to the right surface" was indeed the real architectural
problem, exactly as the brief guessed. **What the brief did not know is that
the route is already in the data.**

## 2. The wire already names the surface — for entities

`serialize_entity` (`mods/BotBridge/types.lua:954`) has carried
`record.surface = entity.surface.name` since 2026-09-06, and
`FactorioEntity::surface: Option<SurfaceId>` has existed to receive it. **It
was read by nothing.** Four writeouts carry a `FactorioEntity` and therefore
already say which surface they are about:

- the bulk `entities` line (one per chunk),
- `on_some_entity_created` / `on_some_entity_updated` / `on_some_entity_deleted`.

`tiles` and `resources` do **not**. Their wire format is a compact header —
`x,y;x,y: name:0,name:1,...` — with no slot for a surface, and
`FactorioTile::surface` is filled with `None` by the parser for exactly that
reason, with a comment saying so. **That is what actually blocks lifting the
guard**, and it is a wire change, not a parser change.

## 3. The guard has never run on a loaded save, and its counter says zero

Measured on the world-record save (`workspace/wrload.toml`, ten surfaces,
Factorio 2.1.17 Space Age):

```
surface_chunk_dropped writeouts in the whole server log:  0
```

Not because nothing was dropped. **`on_chunk_generated` fires only for chunks
the game generates**, and a loaded save generates none — every chunk already
exists. What feeds the model instead is the *initial-discovery replay*, and
that replay is hard-coded to `game.surfaces[1]`, so non-Nauvis chunks are never
offered to the guard at all.

**Absence of drops is not absence of dropping.** The counter is honest about
what it counts and silent about the case that matters.

## 4. The live aliasing bug, which is not hypothetical

The same run produced **2,609 `on_some_entity_deleted` writeouts**, of which
**2,601 named `platform-4`, `platform-2` or `platform-3`** — a space
platform's asteroids being destroyed as the platform flies.

`FactorioSurface::on_some_entity_deleted` calls
`forget_inventory(&entity.position)` and `entity_graph.remove(&entity)`. Both
are keyed by **position alone**. So all 2,601 were applied to **Nauvis**, at
coordinates like `(-14.6, -38.9)` and `(9.7, -45.2)` — a few tiles from the
starting base.

That is precisely the aliasing `FactorioWorld` exists to prevent, happening on
the only multi-surface world this project has, and the Nauvis guard cannot see
it: per-entity events do not pass through `on_chunk_generated`.

## 5. What the chunk replay actually does

`on_whoami` collects `game.surfaces[1].get_chunks()` into
`client_local_data.initial_discovery`, and `on_tick` replays it:

```lua
local maxi = id.idx + 1 -1        -- i.e. id.idx
...
for i = id.idx, maxi do ... end   -- exactly one iteration
```

**One chunk per tick, confirmed by reading, and hard-coded to
`game.surfaces[1]` in both halves** (collection at control.lua:1332, replay at
:1364). The other two `game.surfaces[1]` uses the brief counted are
`chunk_screenshot` / `chunk_screenshot2`, which are not part of ingest.

Because it is one chunk per tick, **ingest time is exactly the charted chunk
count in ticks** — independent of how much is in each chunk.

## 6. What a second surface costs — measured, not estimated

Asked of the running world-record save over RCON:

| surface | chunks | entities | player-force entities |
|---|---:|---:|---:|
| nauvis | 2,962 | 326,790 | 39,998 |
| gleba | 2,018 | 79,376 | 1,376 |
| vulcanus | 2,083 | 221,591 | 1,399 |
| fulgora | 2,352 | 23,255 | 1,805 |
| aquilo | 2,418 | 7,439 | 1,435 |
| platform-1..5 | 156–329 | 41–553 | 41–507 |
| **all ten** | **13,109** | **659,296** | **46,781** |

Three things fall out, and two of them contradict what was expected.

**The other planets are not small.** The premise in `scripts/wr_surfaces.lua` —
"a speedrun visits, takes what it needs and leaves" — holds for what was
*built* (1,376–1,805 player entities each against Nauvis's 39,998) and fails
for what was *charted*: gleba, vulcanus, fulgora and aquilo are each 68–82% of
Nauvis's chunk count.

**So the cost is charted ground, not factory.** Ingest is one chunk per tick,
so:

- Nauvis alone: **2,962 ticks ≈ 49 s** at 60 UPS.
- All ten: **13,109 ticks ≈ 3 min 38 s** — **4.43x**.

The save runs at **59.99 UPS measured idle**, so 60 is real here. Whether it
holds 60 *during* replay was not separately measured — stated rather than
assumed.

**Aquilo is the inversion worth remembering**: 2,418 chunks (82% of Nauvis's
replay cost) carrying 7,439 entities (2% of Nauvis's). Replay time tracks
ground, memory tracks entities, and they are not the same surface ranking.

**Memory.** With Nauvis ingested (2,962 chunks, 323,542 entities after the
mod's fish/flying-text discard), the measured resident sizes were:

```
factorio (the game)      1,103,688 kB   (1.10 GB)
factorio-bot (the model)   375,316 kB   (0.37 GB)
```

The 375 MB is everything — binary, prototypes, graphics, the quadtree — so it
is an upper bound on the per-surface part, not a per-surface figure. Adding
vulcanus adds 68% of Nauvis's entity count; adding all nine others roughly
**doubles** the modelled entity count (332,600 against 323,542).

**The "39,237 entities" everything has been validated against is the
player-force count on Nauvis.** The model actually carries ~323,000 there,
because 270,870 of them are ore tiles.

## 7. What was landed

`b0bb7f53` — **the enabling refactor, with the guard left standing.**

- `FactorioWorld.surfaces` moves behind a `RwLock` so the aggregate can grow
  while it is held as an `Arc`. `surface_or_create(&self, id)` builds a missing
  surface from **the world's own globals**, so `insert_surface`'s `Arc::ptr_eq`
  invariant holds by construction and a parser-created surface can never bring
  a second research state. A `BTreeMap` under a lock rather than a `DashMap`,
  because `surface_ids` answers in name order and a census whose order changes
  between reads is one two runs cannot be compared on.
- `OutputParser` holds that world beside its default surface and routes the
  four `FactorioEntity`-shaped writeouts. `read_output` returns the world it
  built rather than one surface, so a routed surface reaches
  `FactorioInstance` instead of being wrapped away by a second
  `nauvis_only`.
- `surface()`, `nauvis()` and `only_surface()` answer by value; ten call sites
  in `crates/server` and the REPL follow.

### The falsification that came back green, and what it found

Six mutations, each verified to substitute **exactly once**. Five kill exactly
one test; the first kills five, and that is the feature rather than
redundancy — each of the five asserts a different consequence of routing.

| mutation | tests killed |
|---|---|
| `route` ignores the surface the mod named | 5 |
| `surface_or_create` forks the globals | 1 |
| the bulk `entities` arm routes the whole batch by nothing | 1 |
| `only_surface` picks one instead of refusing | 1 |
| `surface_or_create` never remembers what it created | 1 |
| a record that says nothing invents a surface | 1 |

**The fifth came back GREEN on the first attempt, and investigating it found a
real gap.** The first version of that mutation disabled the read-lock fast
path in `surface_or_create`; the suite stayed green because that path is an
*optimisation* — the `or_insert_with` below is where the remembering happens.
So the mutation was wrong. But rewriting it to break the actual memory
revealed that **nothing asserted routing twice to one surface lands in one
graph**: a `surface_or_create` returning a fresh graph per call would have
passed the whole file while dropping 2,263 of the world-record save's 2,264
platform-4 events — a routing bug that looks exactly like a routing fix,
because the entities do leave Nauvis.
`two_records_on_one_surface_land_in_one_graph` exists because of that.

### Live confirmation

A headless run on this branch (`workspace/headless-a.toml`, one character bot,
seed as the workspace had it) with the release binary:

```
§0§surfaces§[{"name":"nauvis","index":1,"planet":"nauvis"}]
unexpected action errors:   0
entity records, by surface: 14,188 nauvis  (nothing else)
```

Three things confirmed at once. **The census crosses a running game** — the
peer session that landed it had never seen its own writeout parsed, and
`surfaces: None` on a dump is the same reading a broken arm would give; it is
not broken. **Routing is inert on a single-surface world**, which is what
every measured run is. And `only_surface()` **answered**, because the plan ran
and printed a roster — so the world held exactly one surface and routing did
not quietly split Nauvis in two.

The run ends on `a real plan must take a positive number of ticks`, which is
`goal_smoke.lua` behaving as documented: that assertion is true only of the
fabricated `--clients 0` roster, and on `--headless` the goal is already
satisfied.

### Verification

- `cargo test --workspace` — **107 test blocks, exit 0** (master carries 106;
  this branch adds one file). Exit code taken from the command, not a pipe.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  clean.
- **Four offline baselines, byte-identical, on one binary**, re-measured by
  this session before and after: `researched:automation` 176 / 21,784 ·
  `producing:automation-science-pack:6` 316 / 22,457 ·
  `producing:logistic-science-pack:6` 441 / 47,478 on
  `workspace/scripts/map.json`, and `gathered:crude-oil` 2,115 / 317,283 on
  `map-31337-explored.json`. `gathered:crude-oil` still refuses on `map.json`
  with `no crude-oil is charted anywhere this plan can`.
- **Both dumps loaded** — `map.json` and `map-31337-explored.json`. Nothing
  about the wire shape moved: a dump is a `FactorioSurface`, and
  `FactorioWorld` has no serde at all.

## What was deliberately not done

**The guard stays.** Lifting it would send `tiles` and `resources` lines from
Gleba into the Nauvis surface, because those two writeouts cannot say which
surface they came from. A guard that drops chunks honestly is better than a
parser that files them wrong. Lifting it needs, in order:

1. a surface in the `tiles` / `resources` wire header;
2. the parser routing them the way it now routes entities;
3. the replay iterating `game.surfaces` instead of `game.surfaces[1]` — at
   which point the *time* cost above becomes real.

**Nothing was ingested from a second surface end to end**, and no run was made
that tried. What was proved is that the entity half routes correctly, in tests,
including the aliasing case the whole surface refactor exists to prevent.

## A latent defect nobody has to act on yet

The guard compares `surface ~= game.surfaces['nauvis']` **by name** while the
replay feeds it `game.surfaces[1]` **by index**. Those are two different
claims and they agree only because Nauvis happens to be created first. The
census (`collect_surfaces`) can now settle it per save; on the world-record
save it does agree — `nauvis` is index 1. Spotted by the census session, left
standing here because changing it without a reason to would be churn.

## What ten surfaces would mean

Not a recommendation — the numbers, so whoever decides has them.

- **Time: 4.43x**, and it is arithmetic rather than a guess, because the
  replay is one chunk per tick. 2,962 ticks becomes 13,109; 49 s becomes
  3 m 38 s at 60 UPS. **The replay rate is the thing to change first**, and it
  is a one-line `maxi` in `on_tick` — 10 chunks per tick would put ten
  surfaces back inside Nauvis's current cost. Nothing was measured about what
  the game can sustain there; `writeout_tiles`'s own comment says ~2.8
  chunks/tick on this box, which suggests the current rate is already
  conservative by a factor of two or three and that the ceiling is real.
- **Memory: roughly 2x the modelled entities** (332,600 added against
  323,542 held), inside a 375 MB process.
- **And most of it buys very little.** The other nine surfaces hold 6,783
  player-force entities between them against Nauvis's 39,998, and the flow
  graph's own validation puts 99% of plate and circuit production on Nauvis.
  So ten surfaces roughly doubles the model to gain ~15% more built factory.
  **Vulcanus alone** — 2,083 chunks, 221,591 entities, 1,399 player entities —
  is the natural first one to prove the pipeline on, and it is also the most
  expensive of the nine.
