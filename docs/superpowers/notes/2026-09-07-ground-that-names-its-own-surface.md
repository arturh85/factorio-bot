# Ground that names its own surface

2026-09-07. Branch `ground-that-names-its-own-surface`, off `106e30aa`.

`tiles` was the last position-keyed writeout that could not say where it came
from. It says now, the parser routes on it through the same `route` the four
`FactorioEntity`-shaped writeouts use, and **the Nauvis guard still stands** —
for four reasons that are no longer about the wire.

## 1. The surface goes in the body header, not the envelope

The mod's header was positional with two fields:

```
x1,y1;x2,y2: name:0,name:1,...
```

It is now three:

```
x1,y1;x2,y2;<surface>: name:0,name:1,...
```

**Why not the envelope.** `writeout` frames every line as `§tick§key§body` and
has no surface slot either, so carrying it there means changing the frame that
every writeout and both readers share, to give a surface to twenty-odd keys with
no place. The body header is where the fact belongs and it is the smaller
change. That was the brief's guess and it survived contact with the code.

**Backward compatibility, and what absent means.** `parse_ground_header`
(`crates/core/src/process/output_parser.rs`) splits on `;` and reads a surface
only when there are three parts; with two it hands the string to `Rect`'s own
`FromStr` exactly as before and answers `None`. `None` is **"an older sender did
not say"** — which is what every archived server log, `workspace/scripts/map.json`
and `map-31337-explored.json` contain — and `route` sends it to the default
surface, the same decision the entity arms already make for a record with no
`surface` key. It is never "an unknown surface" and never Nauvis by assertion.
An empty third field reads the same way: a separator with no name is still not
an answer. `a_header_without_a_surface_is_absent_not_unknown` asserts all of it,
and mutation **C** below is the proof it is not decorative.

**Forward compatibility is not offered, deliberately.** No scheme could give it:
every byte before the first `:` is consumed by a strict `Rect` parse, so a new
mod meeting an older binary produces `malformed tiles line` rather than a silent
misread. Loud is the right failure here, and the release binary embeds the mod
anyway.

## 2. `resources` is dead on both ends — measured, not assumed

The brief asked for `tiles` **and** `resources`. `resources` has:

- **no caller in the mod** — `grep -rn writeout_resources mods/` finds only the
  definition;
- **no arm in the parser** — a line with that key would be logged as
  `unexpected action: resources`.

Resource entities reach the model on the bulk `entities` line instead
(`surface.find_entities(area)` returns them, `serialize_entity` names their
surface), so they have been routed correctly since `b0bb7f53`.

It carries the surface anyway, on the same `ground_header`, because a header
that silently meant Nauvis is exactly the shape this change removes and the
asymmetry would bite whoever revives it. **Nothing exercises that tag**, and its
own comment says so rather than letting its presence imply otherwise.

## 3. The aliasing case is a real test, and the vacuity was designed against

`crates/core/tests/ground_names_its_own_surface.rs`, five tests.

Yesterday's lesson: `a_deletion_on_another_surface_leaves_this_one_standing`
paired "Nauvis kept its chest" with "platform-4 is empty there", and **platform-4
was never occupied at that tile**, so a deletion that routed correctly and then
did nothing passed both halves.

So here, **the other surface genuinely holds a different tile at the very same
coordinate**: water at (0, 0) on Nauvis, `grass-1` at (0, 0) on Vulcanus. Each
surface is asserted to hold *its own* tile by name — an absence is an absence of
that name, not an absence of ingest. The same pairing is repeated through a
*consequential* query rather than a name list: `is_water_at` is what sites an
offshore pump, and Vulcanus answers `false` while being demonstrably occupied at
that tile.

`two_ground_lines_for_one_surface_land_in_one_graph` is the other half, and it
exists for the same reason its entity counterpart does: a `surface_or_create`
handing back a fresh surface per call would pass every aliasing test above while
dropping all but the last chunk of a real planet — a routing bug that looks
exactly like a routing fix.

**One thing the tile tree taught during the writing**: `aabb_quadtree` *panics*
on a second box at a position it already holds ("didn't insert"). The first
version of that test sent the same tile twice; the second line goes to the next
chunk east.

The mod half is covered too. `botbridge_writeout_tiles.rs` already ran the real
`control.lua` in a stub game, and the new
`the_tiles_header_names_the_surface_it_was_read_off` asserts a **non-default**
surface name reaches the header — because a hard-coded `nauvis` would satisfy
every other test in that file, all of which run on the default stub.

## 4. The guard: not lifted, and the reason changed

The wire reason is gone. Four others, each read out of the code rather than
supposed:

1. **`tile_chunks` has no surface in its key** (`control.lua:86`, keyed
   `chunk_x.."/"..chunk_y`). Nauvis chunk (0,0) would suppress the tiles
   writeout for chunk (0,0) on every other surface, silently.
2. **`storage.map_area` is one bounding box** over all charted chunks, and the
   512-tile clamps are applied to it.
3. **The initial-discovery replay is hard-coded to `game.surfaces[1]`** at both
   ends. A loaded save generates no chunks at all — `surface_chunk_dropped` read
   **0** across a whole ten-surface run — so on the only multi-surface world
   this project has, lifting the guard changes nothing whatsoever.
4. Downstream, `OutputParser::on_init` connects only the default surface's
   graph, and `only_surface()` refuses on a multi-surface world.

1 and 2 fail **silently**, which is the failure mode this repo keeps paying for.
The tag was the deliverable; the guard is a stretch that turned out to be four
separate pieces of work, and its comment now names them in place.

## 5. Falsification

Each substitution asserted to match **exactly once** before the build; the file
restored before the next. `cargo test -p factorio-bot-core` each time.

| | mutation | subs | tests killed |
|---|---|---|---|
| A | `ground_header` hard-codes `nauvis` instead of `surface.name` | 1 | **1** |
| B | the `tiles` arm routes to the default surface | 1 | 4 |
| C | a two-field header answers `Some(nauvis)` instead of `None` | 1 | **1** |
| D | the tile's own `surface` field is left `None` | 1 | **1** |

**B killing four is the feature, not redundancy** — the same shape as
`b0bb7f53`'s first mutation killing five. Each of the four asserts a different
consequence of routing: the aliasing case, the consequential query, that two
lines share one graph, and that the tile remembers its surface.

**D is the measurement that matters most.** Routing survives it — `route` reads
the header's surface directly — so it kills *only* the test that asks whether
the tile itself remembers, which is what a dump, a keyframe or a census reads.
Without that test, the field could have been dropped with the routing still
green.

## 6. Verification

- `nix develop -c cargo test --workspace` — **108 test blocks, exit 0**, exit
  code taken from the command and not from a pipe (master carries 107; this
  branch adds `ground_names_its_own_surface.rs`).
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  clean.
- `git status --porcelain crates/` in the main checkout was **clean** at session
  start, so the workspace run was not scoped.
- **Both dumps load**: `workspace/scripts/map.json` and
  `map-31337-explored.json`.
- **Four offline baselines, byte-identical, on one binary I built and
  measured** (release, `--no-default-features --features cli,lua`,
  `--bots 1,2,3,4`):

  | goal | world | expected | measured |
  |---|---|---|---|
  | `researched:automation` | `map.json` | 176 / 21,784 | **176 / 21,784** |
  | `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 | **316 / 22,457** |
  | `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 | **441 / 47,478** |
  | `gathered:crude-oil` | `map-31337-explored.json` | 2,115 / 317,283 | **2,115 / 317,283** |

  `gathered:crude-oil` still refuses on `map.json`, naming the charting radius —
  correct. Everything measured here is single-surface, so nothing should have
  moved, and nothing did.
- **The binary was interrogated, not trusted** — `include_dir!` has no
  `rerun-if-changed`, so a rebuild after a mod edit can ship the old mod and the
  new field then reads uniformly absent exactly as if the game never sent it:

  ```
  strings -a target/release/factorio-bot | grep -c ground_header    4
  ```

- **No live run.** The mod edit is exercised end-to-end by
  `botbridge_writeout_tiles.rs`, which loads the real `control.lua` and feeds
  its output through `OutputParser`, so the two halves are pinned against each
  other on fixtures. What that cannot show is a *generating* world with two
  surfaces, and the guard means no such world exists to run against until items
  1–3 above are done. Stated rather than papered over.

## 7. Still open

- The Nauvis guard, and the four items in §4.
- `daylight` still lands on the default surface though it is per-surface. It has
  the same wire shape as `tiles` had — a candidate for the same treatment.
- `only_surface()` refusing on the world-record save, so `factorio-bot lua`
  cannot run any script against it. Handed over, not touched here.
