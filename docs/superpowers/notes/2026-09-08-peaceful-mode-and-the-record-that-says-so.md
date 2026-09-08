# Peaceful mode, and the record that says so

2026-09-08, branch `peaceful-mode-and-the-record-that-says-so`, off `a398bc82`,
landed as `ba3d8292`. Owner: *"maybe we just add peaceful mode for now?"*

Biters have cost this project two bot deaths and a stalled oil run. Peaceful
mode removes them as an obstacle so the oil and rocket work can proceed. The
flag was the easy half. **Nothing would have recorded it** — a peaceful run and
a hostile one were byte-identical in every provenance field — so the flag and
the field landed together.

Three premises were carried into this task. **Two of them were wrong, and the
measurement that falsified each is below.**

---

## 1. "Show that biters are actually absent" — they are NOT

The task's verification asked for `find_entities_filtered` on `unit` /
`unit-spawner` to prove the enemies were gone. Measured on two isolated
instances, same seed, same radius, `surface.force_generate_chunk_requests()`
out to 24 chunks so the ring beyond the fresh map's ±320 actually exists:

```
                      chunks  surface  map_gen   enemy  spawners  worms
peaceful (headless-c)   3054     true     true     300       169    131
hostile  (headless-d)   3054    false    false     300       169    131
```

**Identical. Peaceful mode removes exactly zero biters.** Nests, worms and
units all generate and stand; what changes is that they do not attack
unprovoked. A later reading of the peaceful world (after it had run a while)
counted **1,408 enemy-force entities** — the extra 1,108 are units the nests
spawned and that were walking around, under peaceful mode, doing nothing to
anybody.

So **the census cannot distinguish the two modes at all**, and that is precisely
why the provenance field is not optional: `peaceful_mode` itself is the only
thing that separates them, and nothing else in the record differs.

Second measurement worth keeping: on seed 31337 the fresh map's generated area
(400 chunks, ±320 tiles) contains **zero** enemy entities. The 169 spawners are
all outside it. That is consistent with the oil finding — crude oil at 372.5
tiles, and the survey that reached it revealing 32 enemy structures.

The rendered value in `run_analysis` therefore says
`peaceful (biters present, not hunting)` in as many words, and a test asserts
the caveat is in the string.

## 2. "Does a savepoint preserve peaceful mode?" — YES, and more than that

Measured directly. Set peaceful over RCON, `game.server_save("level")`, kill
the run, relaunch the **same instance with no `--peaceful` flag at all**:

```
surfacePeaceful=true  mapGenPeaceful=true  chunks=3054  spawners=169  worms=131
provenance.peaceful = true          <- and no flag was passed
```

Two consequences.

**The three-state flag is necessary, not tidiness.** A plain `SetTrue` flag
would have written `false` on that run and silently flipped a peaceful world
hostile. `Option<bool>`: `None` touches nothing, `Some(x)` sets it.

**And this is the strongest single proof that the field records what was TRUE
rather than what was ASKED.** That run requested nothing; a field carrying the
request would have said `false` or `null` and lied about the world it played.

## 3. "Map-gen is probably the right home" — the two homes are one storage

`LuaSurface.peaceful_mode` is not a separate ephemeral state. After the runtime
write, `surface.map_gen_settings.peaceful_mode` reads `true` (it reads `false`
on the hostile control). They are the same value, so the runtime write **is**
the map-gen setting, applied a few seconds later.

That settles the design question by measurement rather than argument, and the
cost side is decisive on its own:

- **Peaceful mode does not change the generated map** — the two runs above
  produced the same resource fingerprint, `c161fa3f437221d0`, the documented
  seed-31337 digest. It is a property of the *run*, like `game.speed`.
- **`--map-gen-settings` is only ever passed when a map-exchange string was
  supplied** (`instance_setup::setup_factorio_instance`), and every measured run
  in this project supplied none — Factorio's shipped defaults. Writing a
  settings file to carry one boolean would put every other map-gen knob into a
  hand-maintained file. That is exactly the trap of the 719-character exchange
  string that shipped in settings from 2021 to 2026-09-06.

So it lives beside `game_speed` in `FactorioParams`, applied over RCON on every
surface once the server is up, and the seed + "default settings" + game version
triple still identifies the map, unchanged.

---

## What landed

- **`--peaceful` on `lua` and `start`.** `--peaceful` = on, `--peaceful false`
  = off, absent = leave the world alone. Declared once in `cli/mod.rs` so the
  two subcommands cannot drift.
- **`FactorioParams::peaceful: Option<bool>`**, applied by
  `FactorioInstance::apply_peaceful_mode` before `game.speed`, and **guarded on
  `server_host`**: `--connect`/`--server` warns and changes nothing, because
  that is somebody else's world. It reads back and warns when the game
  disagrees with the request.
- **`provenance.peaceful: Option<bool>`**, asked of the game at run start the
  way `map_exchange_string` is. **`None` is NOT CAPTURED and must never be read
  as hostile**: every archived run has no key at all.
- **`run_analysis` refuses** a peaceful/hostile comparison, in the same tier as
  `resumed_from`, and lets `None` withhold judgement rather than manufacture
  one.

Both queries are vanilla `/silent-command`, like `ACTIVE_MODS_QUERY` — no
BotBridge function, so this answers even on a server whose bridge failed to
load.

## The refusal, on the two runs that produced it

`run-1788862977-17649` (peaceful, headless-c) against `run-1788863159-84695`
(hostile, headless-d). Every refusal-grade field agrees except one:

```
!! REFUSE   these two runs were NOT produced under the same conditions (peaceful differ)
            peaceful      A=peaceful (biters present, not hunting)   B=hostile   <-- DIFFERENT
            seed          A=31337   B=31337
            git           A=ba3d8292…   B=ba3d8292…
            factorio      A=2.1.17   B=2.1.17
            profile       A=debug   B=debug
            resumed_from  A=fresh world (not resumed)   B=fresh world (not resumed)
ok          same map fingerprint (c161fa3f437221d0)
ok          same roster obtained: [1, 2, 3, 4]
```

**That block is the whole argument in one screen**: without the first row those
two runs are indistinguishable, and any difference between their numbers would
have been credited to whatever change was under test.

## Reproducing it

`scripts/peaceful_in_provenance.lua` is the apparatus and is committed, for the
reason `ElectricOreToPlate` is not reproducible. It calls `record.start` and
holds the game open, bounded in **ticks**, so the census can be taken from
outside:

```bash
factorio-bot lua peaceful_in_provenance.lua --settings workspace/headless-c.toml \
    --headless --bots 4 --game-speed 10 --seed 31337 --new --peaceful
factorio-bot lua peaceful_in_provenance.lua --settings workspace/headless-d.toml \
    --headless --bots 4 --game-speed 10 --seed 31337 --new

factorio-bot rcon -s localhost --settings workspace/headless-c.toml -- \
 '/silent-command local s=game.surfaces[1] s.request_to_generate_chunks({0,0}, 24)
  s.force_generate_chunk_requests()
  rcon.print("peaceful="..tostring(s.peaceful_mode)
    .." spawners="..s.count_entities_filtered{force="enemy",type="unit-spawner"})'
```

The RCON reply arrives on **stderr** at INFO as `rcon ⮞ <body>`.

The chunk generation mutates the world, so these are probe runs and not
measured ones. Nothing was quoted from their timings; only the census and the
booleans.

## Verification

- `cargo test --workspace --no-fail-fast`: **3,087 passed / 0 failed**, cargo's
  own exit 0 (3,080 before; 13 tests added, 6 of them Python).
- `clippy --workspace --all-features --all-targets --deny warnings`: exit 0.
- **Falsification sweep, `scratch/falsify.py`: 10 mutations, 10 RED, 0 green,
  0 that failed to compile**, each substitution asserted to match exactly once,
  restored by copy plus `touch`. It covers the parser's unknown-is-not-false
  rule, the boolean, the trim, the setter's read-back, the shared stamp, serde's
  absent-key default, serde's serialisation of unknown, the refusal tier, the
  hostile-vs-unknown split, and the biters caveat in the rendered word.
- **All four offline baselines unmoved** on `ba3d8292` (debug build): automation
  `176 / 21,784`, `producing:automation-science-pack:6` `316 / 22,457`,
  `producing:logistic-science-pack:6` `441 / 47,478` (all `map.json`),
  `gathered:crude-oil` `2,117 / 325,138` (`map-31337-explored.json`). Nothing in
  the planner was touched and nothing moved.
- `workspace/mods/BotBridge` and both instances' `mods/BotBridge` were repointed
  at this worktree by the debug runs and **restored** to
  `<repo>/mods/BotBridge`.

## What is not established

- **Nothing here measured a biter declining to attack.** That peaceful mode
  stops unprovoked attacks is Factorio's documented semantics of the flag; what
  this session measured is that the flag is set, that it survives a save, that
  it is recorded, and that it removes no enemies. A run that walks a bot past a
  nest and comes back is the missing evidence, and it costs an oil-length run.
- **A surface created after the write is not covered.** `apply_peaceful_mode`
  touches the surfaces that exist when it runs; a space platform generated later
  is hostile. Provenance asks again at run start rather than trusting the
  setter's reply, which is why that is a gap and not a lie.
- **`--peaceful` cannot reach a `--connect`/`--server` run**, deliberately. It
  warns and changes nothing.
