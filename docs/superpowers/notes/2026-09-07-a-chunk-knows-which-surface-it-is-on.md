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

*(filled in below once the change is in — see the commit.)*

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
