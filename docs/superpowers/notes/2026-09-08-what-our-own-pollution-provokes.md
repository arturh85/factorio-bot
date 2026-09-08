# What our own pollution provokes: measured, and the answer is "almost nothing yet"

2026-09-08, branch `what-our-own-pollution-provokes`, off `144603b2`.

## The question

The owner:

> *"as soon as we have radar or more than 2 steam engines the biters will start
> to evolve and send out biters to attack the base"*

The project was **completely blind** to this. `pollution` and `evolution`
appeared zero times in `mods/BotBridge/control.lua`, zero times in
`crates/planner/src/`, zero times in `crates/executor/src/`; the only hits in
`crates/core/src/` were commented-out lines in a bundled `config.ini`. The
world dump carries only the `player` force, so **the enemy force was not in the
model at all**.

So "does our own production provoke attacks on the timescales we run at" was
folklore. It is now a number.

## What was built

The observation, and only the observation. No pollution-aware planning, no
attack predictor, no defence trigger — nobody had established that these
numbers move at all, and **a flat curve is a complete result**.

Per surface, on the mod's existing 300-tick force beat (sample schema 2 → 3):

| field | source | why this one |
|---|---|---|
| `total` | `LuaSurface.get_total_pollution()` | cheap, comparable across runs |
| `at_spawn` | `get_pollution(spawn)` | `get_pollution` is a **chunk** reading and needs a position; spawn is the one position that means the same thing on every run. A grid was rejected: O(chunks) per beat for a question nobody has asked. |
| `produced` / `absorbed` | `game.get_pollution_statistics(surface)` | the game's own decomposition **by emitting prototype** — the half that can name the cause |
| `pollutant` | `LuaSurface.pollutant_type` (an **attribute**) | under Space Age, Nauvis pollution and Gleba spores are different substances behind one number |
| `evolution` | the enemy force's factor **and its three causes** | `by_pollution`, `by_time`, `by_killing_spawners` |

**The decomposition is the whole value.** Without it a rising number cannot be
attributed and the folklore survives the measurement.

## The measurement

`scripts/pollution_watch.lua`, seed 31337 `--new`, four headless character
bots, 10x, debug build, isolated instances on non-default ports. Two arms on
**the same map** (identical `map_exchange_string` in both provenance files),
differing only in `--peaceful`. Each builds `researched:automation` — an
offshore pump, a boiler, a steam engine, a pole, a lab and burner drills and
stone furnaces — and then **holds the game open for 90,000 GAME TICKS** while
the sampler runs. The hold is bounded in ticks, never in poll counts: a poll
count is not a duration.

375 shared force samples per arm, 31.7 minutes of game time.

### Evolution over the run (hostile arm)

```
    at       total  at_spawn       evo  by_poll  by_time  by_kill
  5:00        37.1       0.0   0.00135  0.00005  0.00130  0.00000
 10:00         8.6       0.0   0.00255  0.00005  0.00250  0.00000
 15:00         3.3       0.0   0.00374  0.00005  0.00370  0.00000
 20:00         0.0       0.0   0.00493  0.00005  0.00490  0.00000
 25:00         0.0       0.0   0.00612  0.00005  0.00610  0.00000
 30:00         0.0       0.0   0.00730  0.00005  0.00730  0.00000
 31:11         0.0       0.0   0.00760  0.00005  0.00760  0.00000
cumulative emitters: burner-mining-drill 43, stone-furnace 13, boiler 6
```

## Four findings

**1. On our timescales, evolution is a CLOCK, not a factory.** Over 31.7
minutes evolution reached **0.0076**, of which **`by_time` is 0.0076 (99.3%)
and `by_pollution` is 0.0000549 (0.7%)**. Had we built nothing at all, the
number would be within 1% of where it is. *Nothing this project currently does
moves enemy evolution measurably.* Combat is a later-game concern, and the
owner's mechanism is real but its threshold is above where we operate.

**2. `by_pollution` stops growing at ~tick 25,800 (7 minutes) and never moves
again.** The bots finish the milestone, the burner drills and furnaces run out
of fuel, and emission stops. **Total pollution peaks at 41 and decays to
exactly 0 by minute 18** — absorption exceeds emission by a wide margin at this
scale. A 41-unit cloud is nothing: it is not that our factory is clean, it is
that our factory is *tiny and briefly alive*.

**3. Peaceful mode does not change evolution.** Same seed, same map, same
script, 375 samples compared **tick for tick**:

```
max |evolution_hostile - evolution_peaceful| = 7.8e-06
```

and that entire difference sits in `by_pollution`, not `by_time` — i.e. it is
the two runs' bots having done marginally different work, not peaceful mode
suppressing anything. `by_time` is **identical to every printed digit at every
shared tick**. This pairs with the earlier finding that peaceful does not
remove biters (same census, 1,408 enemy entities walking around): **peaceful
changes whether they attack, and nothing else we can measure.**

**4. The measurement is a LOWER BOUND, and says so.** `researched:automation`
builds **one** boiler, **one** steam engine and **no radar**. The owner's
scenario names *radar* and *more than two steam engines* — neither was present.
What is established is that our current t=0 ladder does not provoke anything;
what is **not** established is where the threshold is. The apparatus is
committed and re-running it against a bigger build is one command.

## An honest gap: `at_spawn` was 0 in 375 of 375 samples, in both arms

A uniformly constant field is exactly the tell for a plumbing failure, so it
was checked rather than believed. Against a live game over RCON:

```
spawn={x = 0, y = 0}  at_spawn=0  total=0
surface.pollute({0,0}, 50)
                      at_spawn=50 total=50
```

**The accessor works.** So `0` is a real chunk reading, not a broken one. *Why*
the spawn chunk held none while the surface held up to 41 is **not
established** — the natural candidate is that the build sat in a neighbouring
chunk and absorption ate the spread before it reached (0,0), and that is a
hypothesis, not a measurement. Left open deliberately rather than replaced with
a story.

(The probe world had **no machines**: a `--new` run's built world is not
re-saved, so re-opening `pollution-a` gives the fresh seed-31337 map. That is
why the injection test was the discriminator and not a reading off the real
build.)

## Absent is not zero, at four levels

The failure this whole feature is built around is a run that **never looked**
being rendered as a run that measured a calm world — which would answer the
owner's question with a number nobody took.

- **Mod**: every read is individually `pcall`ed and writes its key **only on
  success**.
- **Rust**: `Option<PollutionSample>`, and every field inside it optional.
- **Analysis**: `not-captured` and `no-samples` are separate statuses; a
  missing reading renders `?`, never `0.00000`; a run with no readings gets a
  **NOT CAPTURED** paragraph and deliberately no table a reader could mistake
  for a flat one.
- **TypeScript**: mirrored nullable, pinned by the contract spec.

Verified on a real archive: `tools/fixtures/run-1788679826-02267` reports

> NOT CAPTURED: none of this run's 142 force samples carries a `pollution` key
> (schema [2]); the mod never recorded it. That is 'we never looked', NOT
> 'nothing polluted and nothing evolved'.

## Two defects the work found before any run

**A raise in the new reader cost the WHOLE force sample.** `pollution_totals`
raised on a surface with no `name` (`surfaces[nil] = entry`), and because it
sat inside `sample_force_body` the outer `pcall` swallowed it — research,
production and power all silently gone, with no symptom but a missing line.
Caught by `the_world_state_samplers_run_for_the_whole_session`, which exists
for exactly that. Two independent fixes, and **two independent tests**, because
the falsification sweep proved they are independent: the name fallback keeps
the *reading*, the dedicated `pcall` keeps the *line*.

**`pnpm lint` caught three stale test helpers that `pnpm test` could not** —
vitest strips types and runs. Exactly as `CLAUDE.md` predicts.

## Falsification

`tools/pollution_falsification_sweep.py`: **11 mutations, all 11 red in the
named test.** Three were green on the first pass, and each green was a finding
about something different:

- Twice, "remove `#[serde(default)]` from an `Option<T>`" is a **no-op** —
  serde's derive already yields `None` for a missing `Option`. The mutation was
  measuring nothing. Replaced with the mutation the field actually guards
  against: a missing reading *defaulting to a measured zero*.
- Once, the sweep itself was wrong: it pointed two independent defences at one
  test (above).

The sweep backs up by file copy and restores by copy **plus `touch`** (never
`git checkout -- <file>`, never `copy2`), refuses to report anything unless the
baseline is green, treats a mutation that fails to compile as a finding rather
than a red, and checks the tree is clean afterwards.

## Baselines

Unmoved, on one binary (`bd18714e`, debug — plan output is deterministic
across profiles):

| goal | world | actions / ticks |
|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,117 / 314,345 |

## Incidental: `map_exchange_string` is live-confirmed

`CLAUDE.md` records that the field had never been written non-null by any run.
**Both arms here wrote one**, and the two strings are byte-identical — so the
"same seed, same settings, same map" condition is now *recorded* rather than
assumed, for the first time.

## What NOT to conclude

- Not "our factory is clean". It is small and it stopped.
- Not "evolution never matters". It matters above a threshold we have not
  reached and have not located.
- Not "peaceful mode is free". It changes attack behaviour; this says only that
  it does not change the evolution *number*.
