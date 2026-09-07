# An asteroid deleted from Nauvis — the fix had already landed, and the live run says so

2026-09-07. Branch `an-asteroid-deleted-from-nauvis`, off `ee28485e`.

The brief was: deletions are not on the routing yet, put them there. **They
already were.** `b0bb7f53` — landed on master before this session started —
routes all four `FactorioEntity`-shaped writeouts, `on_some_entity_deleted`
included, and `crates/core/tests/a_chunk_knows_which_surface_it_is_on.rs`
already carries the aliasing case as a test.

So this session did the two things that were genuinely outstanding: **run it
against the world-record save**, which nobody had, and **close a vacuity in the
one test that is supposed to prove it**.

## 1. The deletion writeout already carries a surface

Measured, not inferred, at both ends:

- `mods/BotBridge/control.lua:3739` writes `on_some_entity_deleted` through
  `serialize_entity`, which has carried `record.surface = entity.surface.name`
  since 2026-09-06.
- `crates/core/src/process/output_parser.rs:469` routes it:
  `self.route(&entity.surface.clone()).on_some_entity_deleted(entity)?`.

The only position-keyed writeout still unrouted is `tiles` (line 193), which is
the known, deliberate case the mod's Nauvis guard covers. **The guard stays**,
untouched.

## 2. The live run — and the number

`workspace/wrload.toml`, RCON 4390, the world-record save, release binary built
from this branch, read-only throughout (`scripts/deleted_elsewhere_probe.lua`
places nothing, mines nothing, and issues no mutating command). Server logs on.

```
on_some_entity_deleted writeouts in the run:   928
                          platform-4:          779
                          platform-2:          130
                          platform-3:           17
                          nauvis:                2
```

779 + 130 + 17 + 2 = 928, exactly. **Nauvis absorbs 2 of 928.** Before routing
it would have absorbed all 928, at coordinates a few tiles from the starting
base.

And the parser said so in the run's own output, in the three lines this session
added to `surface_or_create`:

```
model now tracks surface platform-4 (2 in total)
model now tracks surface platform-3 (3 in total)
model now tracks surface platform-2 (4 in total)
```

Exactly the three surfaces the deletions name, and nothing else — the bulk
`entities` replay is hard-coded to `game.surfaces[1]`, so no other writeout
could have created them.

**What this does NOT show, stated rather than papered over.** The counts are
what the *wire* said and what the parser *created*; nobody read the platform
graphs back out of the live model, because there is no Lua binding for a
per-surface census and the run could not get that far (see §3). The link "each
routed record lands in that surface's graph and stays there" is pinned by
`two_records_on_one_surface_land_in_one_graph` and by the new test below, on
fixtures. A partial result with its limits stated.

The earlier session's 2,609 / 2,601 and this run's 928 / 926 are the same
world at different lengths, not a discrepancy: this run stopped early.

## 3. Routing makes `only_surface()` refuse on the WR save — the seam works, and it costs a path

The run ended on:

```
Error:   × Failed to start Factorio (no world available)
```

which is `app/src-tauri/src/cli/lua.rs:369`, reached because
`FactorioInstance::surface()` is `FactorioWorld::only_surface()` and the world
now holds **four** surfaces. That is the porting seam doing exactly what it was
built to do — a caller that never said which surface it meant stops working
rather than silently getting Nauvis.

**The consequence is new and worth knowing before somebody debugs it as a
regression: `factorio-bot lua` can no longer run any script against the
world-record save.** The seam fires during startup, before the script does. It
never fired before because it never had a second surface to see; routing is
what gave it one. Every single-surface run (which is every measured run this
project makes) is unaffected — verified by the four offline baselines below and
by the note of 2026-09-07 §"Live confirmation".

This is a handover, not a defect: the fix is to port `FactorioInstance::surface`
and its callers to name a surface, which is the review `only_surface` exists to
force.

## 4. The vacuity that was found, and the test that closes it

`a_deletion_on_another_surface_leaves_this_one_standing` asserts that a
platform-4 deletion leaves the Nauvis chest standing, and pairs it with
"platform-4 holds nothing there either". **That pairing does not do the work it
looks like it does**: platform-4 was never occupied at that tile, so a deletion
that routed correctly and then did *nothing at all* passes both halves.

Proved by mutation, not by reading: emptying `FactorioSurface::
on_some_entity_deleted` of its `entity_graph.remove` call leaves that test
**green**.

`a_deletion_removes_from_the_surface_it_was_routed_to` closes it — create on
platform-4, assert it stands there, delete on platform-4, assert it is gone from
*that* graph while the world still holds two surfaces. Same shape, and the same
reason, as `two_records_on_one_surface_land_in_one_graph`.

### Falsification

Each substitution asserted to match **exactly once** before the run.

| mutation | tests killed |
|---|---|
| the `on_some_entity_deleted` arm routes to the default surface | 2 |
| `FactorioSurface::on_some_entity_deleted` skips `entity_graph.remove` | **1** |

The first kills both deletion tests, each for its own reason — the aliasing one
because the chest leaves Nauvis, the new one because the chest never leaves
platform-4. The second kills **only the new one**, which is the measurement
that matters: it is the evidence that the pre-existing test could not tell a
working deletion from a no-op.

The narration lines in `surface_or_create` have no test and are not claimed to
have one. They are `info!` (`paris`, stdout) and their correctness is the live
run above.

## 5. Verification

- `nix develop -c cargo test --workspace` — **107 test blocks, exit 0**, exit
  code taken from the command and not from a pipe. `git status --porcelain
  crates/` was clean in the main checkout at session start; the work was done
  in an isolated worktree regardless.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  clean.
- **Four offline baselines, byte-identical, on one binary, re-measured before
  and after the change**: `researched:automation` 176 / 21,784 ·
  `producing:automation-science-pack:6` 316 / 22,457 ·
  `producing:logistic-science-pack:6` 441 / 47,478 on
  `workspace/scripts/map.json`, and `gathered:crude-oil` 2,115 / 317,283 on
  `map-31337-explored.json`. `gathered:crude-oil` still refuses on `map.json`,
  which is correct.
- **Both dumps load.**
- `workspace/mods/BotBridge` still points at the main checkout
  (`readlink`-verified after the run). The mod was not edited, so the
  `include_dir!` staleness trap did not apply to it; the binary was
  interrogated for the new narration string anyway (`strings -a … | grep -c
  "model now tracks surface"` → 1) before the run.

## 6. Still open

- **`tiles` and `resources` cannot say which surface they came from**, so the
  mod's Nauvis guard must stay. Unchanged, and deliberately.
- **`OutputParser::on_init` connects only the default surface's graph** —
  `self.world.entity_graph.connect()` and `flow_graph.update()`. A routed
  surface's graph is never connected, and a surface created *after* `on_init`
  could not be even if the call iterated. Not a deletion problem, but it is the
  next hole in the same wall.
- **`only_surface()` now refuses on any multi-surface save**, §3.
- **`daylight` lands on the default surface** while daylight is per-surface.
  Same wire-header shape as `tiles`.
