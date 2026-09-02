# Overnight progress — 2026-09-02

Priority order set by Arturh: **reliability first, watchability second.**

## Landed

- **The run record enrichment plan (12 tasks) is complete and merged**, plus a
  whole-branch audit and its 7-finding fix wave. Gates at the last full check:
  781 frontend tests, 1063 Rust tests, clippy clean, coverage 96.98/93.01.
- **`check_bots_interchangeable` is scoped** to the items an expansion actually
  reads (`1506bf45`), reviewed and approved. It compared whole inventories, so a
  four-bot run refused to plan `have("iron-ore", 20)` over a difference in
  *iron-plate* — an item that plan never touches.
- **`milestone_stuck` finally writes `last_error` and `best_steps`.** Both
  fields had existed with no writer for the life of the project.
- **Per-bot share sizing is specced** (`3e08f8af`), against measurement rather
  than argument — see below.

## First live run that got past planning

`run-1788298772-73007`. Planning produced a real 3-bot DAG and every new stream
worked:

- `plan_created` written for the first time ever, with ids, bots, deps,
  `planned_start` and `planned_duration`;
- `samples.jsonl` 4,146 bytes — 7 bot samples on the 60-tick beat, 1 force
  sample on the 300, all schema- and run-stamped, positions unrounded
  (`4.8359375`), inventories real;
- `milestone_stuck.last_error` carried the full traceback, which is the only
  reason the next defect is legible at all;
- `map.jsonl` 0 bytes — correct: nothing was placed, so no keyframe.

It then died in **execution**, not planning:
`success=0 failed=2 lost=2 pending=1`, then
`game rejected the command: early eof`. Under investigation.

Also: only **3 of 4** clients connected. Unexplained.

## What the share-sizing spec overturned

Written by copying the crate outside the repo, bypassing the guard and
measuring, rather than reasoning from the code:

1. **The guard's motivating failure did not reproduce.** Asymmetric rosters
   already expand *and schedule* correctly, up to a 62-action red-science plan.
   So deleting the guard rests on absence of evidence — the spec flags this as
   its own weakest claim.
2. **The intuitive fix is worse.** "Ask the richer bot for less" measured 2427
   ticks against 2156 for splitting the work evenly and letting holdings decide
   only participation and remainder.
3. **The real defect is not asymmetry.** Four *identical* bots mine 24 ore for a
   bill of 16, because the per-bot read uses `inventory_count` where the
   shortfall was taken against `available` — so the chain is asked for
   `share + reserved` instead of `share`. That is live in every multi-bot run
   today and has nothing to do with the crash that led here.

## Ruling carried into step 4

The guard gets deleted as the spec recommends, **but not on absence of evidence
alone**: a committed regression test must first cover the asymmetric-roster
expand-and-schedule path, so the property the guard asserted becomes tested
rather than merely un-disproven.

## Open

- execution failure (`early eof`, and 6 actions failing before it);
- 3-of-4 client connection;
- guard runs before method resolution, so it can refuse a goal `AlreadySatisfied`
  would have settled without any split — moot once the guard goes.

## A process defect I caused (2026-09-02, ~00:40)

`75a098a8` is titled as a docs commit but also contains the sampler crash fix:
`mods/BotBridge/control.lua`, `crates/core/src/record/samples.rs`,
`crates/core/src/process/output_parser.rs` and the OpenAPI snapshot.

Cause: I ran `git add <two .md paths>` and then a bare `git commit`. The
explicit-paths discipline protected the **add**, not the **commit** — a bare
commit takes the whole index, including files a concurrently running agent had
already staged. The agent then found its own working tree clean, correctly
concluded its fix was already committed, and reported "already shipped".

The code is correct and tested; only the history is misleading. I am not
rewriting it: two agents are mid-edit in `crates/planner/`, and disturbing
`master` under them risks more than a wrong commit message costs.

**Rule going forward, with concurrent agents in one checkout: use the pathspec
form `git commit -- <paths>`, never `git add <paths>` followed by a bare
`git commit`.** The second form is only safe when nobody else is staging.

## Status at ~01:00

**Per-bot share sizing is complete** (spec `3e08f8af`, steps 1-4 in `6fcbba5c`,
`8cc73153`, `7d614681`, `56870959`, `f3a22e29`), each step reviewed clean.
The interchangeable-bots guard is retired behind a regression test that reaches
`schedule()` — which the guard never did. As the reviewer put it, the guard
could only refuse loudly; it never verified a scheduled outcome, so the property
is better protected now than it was before. Steps 5 (prose) and 6 (re-measure
`more_bots_finish_sooner`, whose ledger comment is stale at 6691 against a
measured 7075) remain open and are cosmetic by comparison.

**The sampler crash is fixed** and live-verified: run 4 reached
`success=2 failed=1 lost=0` — real actions completing, server surviving. That is
the first time any action has succeeded in a recorded run tonight.

**The API audit found why runs are slow and why timings cannot be trusted.**
Bots largely do not walk: the "stuck" check measures leg duration rather than
being stuck, so every path leg over ~9.2 tiles is teleported by an unbounded
distance, and a sibling branch reports the walk successful while leaving the bot
walking in a straight line forever. Fix in flight, with observability required
in the same change — every walk duration in every existing record may be
fiction, and the record must be able to say so.

**Not acted on, for Arturh to decide:** the Factorio Learning Environment drops
graphical clients entirely for server-side `character` entities, retiring the
26s load and 90s connect wait. It would also retire the screenshot pipeline,
which is the watchability half of this project. That trade is not mine to make.

## Open gap: EventKind::Teleport has no writer (~01:30)

`c64a995b` fixed the walk defect and added `EventKind::Teleport`, contract-tested
through the OpenAPI seam. **Nothing writes it.** `grep -rn EventKind::Teleport
crates/` returns one hit, and it is a comment.

A teleport is now loud in `tracing` diagnostics and still absent from
`events.jsonl` — so it is visible to whoever tails stderr and invisible to the
artefact runs are compared with. That is the declared-and-never-written shape
this branch was built to eliminate, recreated inside the change meant to close
it. Not the implementer's fault: the writer lives in `crates/scripting_lua`,
which `crates/core`'s `OutputParser` cannot depend on, and that was outside its
stated file list.

**Next action when `crates/scripting_lua` is free:** wire a writer so a teleport
reaches the record, then rebuild, re-seed `workspace/mods`, and rerun. Until
then, any walk duration in a recorded run still cannot be trusted — which is the
whole reason the walk fix mattered.

Also deferred by that change, honestly: `needs_destroy_to_reach` is now carried
to the caller and warned about, but not acted on. A leg the pathfinder flagged
as blocked is still walked.

## Rung status at ~02:15

| rung | state |
|---|---|
| 1 gather iron ore x20 | **satisfied**, 2 iterations (was 4 before the walk fix) |
| 2 gather copper ore x20 | **satisfied**, 4 iterations |
| 3 smelt iron plates x10 | **satisfied** — genuinely, `already_satisfied`; freeplay hands each bot 8 plates and the goal asks possession of 10 across the roster |
| 4 research automation | **crashes** — `Share(b)` sizing vs. free assignment; fix in flight |
| 5-7 power, belts, oil | not reached |

Run 5 (`run-1788303085-75848`): 0 failed, 0 lost across the whole run, 273 KB of
samples. The cleanest run of the night, and the first where every satisfied
milestone reports **why** it was satisfied.

## Defects found and fixed tonight, in the order they bit

1. **The sampler killed the game.** `character.mining_target` — that attribute
   belongs to `LuaEntity`, not a character. Fired the first tick a bot really
   mined, so every planning-only test passed. Fixed, and both samplers are now
   wrapped so telemetry can never raise into the main loop again.
2. **The guard compared whole inventories**, refusing to plan `iron-ore` over a
   difference in `iron-plate`. Scoped, then retired entirely behind a test that
   reaches `schedule()` — which the guard never did.
3. **Shares were sized in the wrong ledger** — `inventory_count` where the
   shortfall was taken against `available`, so four *identical* bots mined 24
   ore for a bill of 16.
4. **Bots did not walk.** The "stuck" check measured leg duration, so every leg
   over ~9.2 tiles was teleported by an unbounded jump; a sibling branch
   reported the walk successful while leaving the bot walking forever.
5. **Satisfaction was inferred, not checked.** Now `planner::holds` answers it,
   three-valued, and the supervisor raises when a goal demonstrably does not
   hold.
6. **A mine action reported success at half its count.** `on_mined_entity`
   completed *any* bot's task matching the entity, never checking
   `event.player_index`, so two bots on one tile decremented each other.

## Two corrections to things I said earlier tonight

- I called rung 3 a false success. It was not: the samples show the goal
  genuinely held. The milestone is *named* "smelt" while the goal asks
  possession.
- I approved retiring the interchangeable-bots guard on the stated ground that
  nothing still assumed interchangeability. `Researched` does. Neither I nor the
  step-4 reviewer caught it; the rung-3/4 diagnosis did.

## Still open

- `EventKind::Teleport` has no writer — teleports are loud in `tracing`, absent
  from `events.jsonl`.
- `needs_destroy_to_reach` is carried and warned about, not acted on.
- Option 3 for `Researched` — a real multi-bot decomposition — remains the fix
  that removes the tension rather than choosing a side. Option 1 (bind a share
  to its bot) is being implemented instead, trading parallelism for correctness.

## Run 6 — the best of the night (~02:45)

`run-1788304631-16800`, with every fix in: walk, per-bot sizing, satisfaction by
checking, the `stated_holder` weld, mine completion, and share binding.

- **Rungs 1-3 satisfied in 2,160 ticks.** They took 25,599 in run 5. A **12x
  speedup** from correcting one event handler — `on_mined_entity` was completing
  *any* bot's task matching the entity, so two bots on one tile decremented each
  other and the planner re-planned the difference, four times per milestone.
- **Rung 4 plans for the first time**: a **95-step plan** for
  `research automation`, where every earlier attempt died during expansion. The
  share binding did what it was for.
- **The failure moved from the planner to the transport.**
  `expected value at line 1 column 1` is serde's message for input that is not
  JSON at all, plus an earlier `Unexpected Response: nil`. A research plan
  exercises crafting, machine insertion and lab placement — paths the ore
  milestones never touched.

Asked for one generalisation with the fix, since this is the third reply-shaped
failure tonight: **an unparseable RCON reply must report what it received**,
truncated, not merely that parsing failed. `expected value at line 1 column 1`
is a message that says nothing about its own cause.

## Hazard: `git reset --hard` in a shared checkout (~03:15)

An agent reported its in-flight edits vanishing mid-task; the reflog shows
`reset: moving to HEAD`. **No committed work was lost** — every commit of the
night is reachable — but one agent's uncommitted working tree was destroyed by
another's reset.

With several agents in one checkout, `git reset --hard` is not a local
operation. Dispatches now say so explicitly. The pathspec commit form
(`git commit -m "…" -- <paths>`) protects the index; nothing protects the
working tree except not running that command.

## What run 6's failure actually was

Both malformed replies are explained, and neither was a transport bug in the
end:

- `Unexpected Response: nil` — `action_failed(tick, action_id)` called with no
  reason, so `tostring(nil)` travelled as the game's entire verdict.
- `expected value at line 1 column 1` — the mod writes plain-text pathfinder
  verdicts ("Error: failed to path find") into the same slot a success fills
  with JSON, and Rust fed them to serde. A real verdict became a syntax error
  and was discarded.

That also explains an anomaly I had noticed and not chased: milestone 4 had a
95-step plan, an error, and **zero `action_dispatched` lines**, because
`rcon_place_entity` did not stamp the tick on its refusal exits.

**The blocker underneath is in the planner, not the transport.** `AtPosition`'s
reach radius is a disc, so a bot already standing on the target satisfies it
with `travel == 0`, no `Walk` is emitted, and the placement is refused because
the bot is inside the new entity's own footprint — bot 1 at `(38.30, 16.48)`
placing a furnace at `[38, 16]`. Being fixed as an annulus.

## Run 7 — the bots build (~03:50)

`run-1788307982-79011`, with the annulus, teleport writer and RCON reply work in.

**107 actions dispatched, 99 succeeded.** By verb:

```
mine 60 · craft 14 · place 14 · fuel 14 · insert 3 · take 2
```

That is the first run in which this system does anything beyond mining: it
places furnaces, fuels them, inserts ore, takes plates, and crafts. `map.jsonl`
carries **156 KB** — the entity map has real content for the first time,
because entities are genuinely being placed. 544 KB of samples beside it.

Rung 4 reached **73 steps across 5 iterations** before sticking on:

```
ERROR: the target stone was gone before mining finished -- something else mined it first
```

Two things about that message. It is the next defect — the planner assigns two
bots the same resource tile, exactly as the mine-completion diagnosis predicted
("`nearest_resource_tile`/`resource_tiles_for` reserve nothing between bots
planned in the same pass"). That fix stopped two bots decrementing each other's
counters; it did not stop them being sent to the same tile.

And it is legible at all only because of the reply work a few hours earlier. The
same failure would have read `expected value at line 1 column 1`.

## Rung status

| rung | state |
|---|---|
| 1 gather iron ore | satisfied, 3 iterations |
| 2 gather copper ore | satisfied, **1 iteration** |
| 3 smelt iron plates | satisfied (`already_satisfied`) |
| 4 research automation | 73 steps, stuck on resource-tile contention |
| 5-7 power, belts, oil | not reached |

## The viewer has not seen any of this (~04:10)

Run 7 is the richest record the project has produced: **720 frames**, 156 KB of
entity map, 544 KB of samples, 107 actions.

The server running on :7492 answers `/api/v1/runs/{id}` and 404s on `/samples`
and `/map` — it is an **old binary**, started before those endpoints existed.
Every release build tonight used `--no-default-features --features cli,lua`,
which is correct for running the game and excludes the server crate, so `serve`
is not even a subcommand of the binary I have been building.

Consequence, stated plainly: **the entire watchability half of this work has
never been exercised against a real run.** The samples, the entity map, the
teleport events, the analysis view's plan-to-outcome join — all of it is unit-
and contract-tested, and none of it has rendered a genuine 107-action run.

Next build must be `--all-features` (or add `server`), then restart the viewer
and actually look at run 7. That is a first-order task for the morning, not a
footnote: it is exactly the "every gate was green while the thing did not work"
shape this night has been about, and I walked into it while chasing rung 4.

Note there is a `factorio-bot serve` on :7492 that I did not start — left alone.

## Run 8b, and the viewer finally seeing a real run (~04:40)

Tile reservation helped planning and exposed the next layer:

- milestone 1 down to **1 iteration** (was 3);
- rung 4's plan up to **96 steps** (was 73);
- but only 11 actions dispatched before sticking on
  `cannot place item 'stone-furnace' because surface.can_place_entity said 'no'`.

The annulus got the bot out of its own footprint; the game now refuses the tile
for a different reason. That is a model-versus-game disagreement, which is
precisely what the keyframe `divergence` list was built to catch — and it has
never been used in anger. The diagnosis was told to look there first.

Note the copper-ore contention error still appeared once at milestone 2, which
matches the tile-reservation author's own stated limit: claims are per-plan, so
two *successive* plans can still pick the same tile.

**The viewer now serves a real run.** Against run 7 on :7500:
`/samples` 722 records, `/map` 15, `/frames` 720, `skipped: 0` on both new
streams — so the schema stamp, ingestion and readers all hold against genuine
data rather than fixtures.

**Trap worth keeping:** an `--all-features` build enables the tokio-console
subscriber, which binds a fixed port, so a viewer and a game run cannot both be
`--all-features`. Run 8 died with `AddrInUse` and a core dump whose panic named
`console-subscriber` and nothing about the game. The binaries are now split:
`factorio-bot-serve` (all features) for the viewer, `factorio-bot`
(`cli,lua`) for runs.

## A trap I warned others about and then walked into (~05:10)

The CLI printed, and I had not been reading it:

```
scripts workspace copy is STALE: 2 file(s) differ
```

`workspace/scripts/research_run.lua` was 12 lines behind the repo — missing the
`record.teleports()` drain added with the teleport writer. I re-seeded
`workspace/mods` before every single run tonight and **never once re-seeded
`workspace/scripts`**.

Consequence: when I reported `teleports: 0` for run 8b, that was not "no
teleports happened". It was the script never draining the queue. The number was
real and meant nothing, which is the exact failure mode this night has been
about.

**Both copies must be re-seeded before a run**, and the STALE warnings the tool
already prints are worth reading rather than scrolling past. It also warned
about `workspace/plans` (15 files) — unexamined, probably irrelevant, but
unexamined.

## Run 9 — a regression, and part of it is the truth arriving (~05:30)

`run-1788310810-27811` hit the 25-minute timeout having dispatched **9 mine
actions**, against run 7's 107. Two failed with `ERROR: too far too mine`. All
four bots connected this time, where earlier runs got three.

Under diagnosis. The hypothesis worth stating up front is that some of this is
not a regression at all: before the walk fix, bots teleported past every path
leg over ~9.2 tiles, so runs were fast and their durations were fiction. They
now walk. Tile reservation then sends four bots to four *different* tiles, which
are further apart than one shared tile, so there is more walking to do and it is
now really done.

The diagnosis was told to separate that honest cost from any genuine defect, and
explicitly not to propose reverting the walk fix. Restoring the old throughput
number by reintroducing teleportation would be the same category of mistake this
whole night has been about, pointed the other way.

Note also: run 9 was still running the **stale** `workspace/scripts` copy, so
its teleport lane is empty for that reason rather than for want of teleports.

**Operational consequence:** the 1500 s run timeout was sized when bots
teleported. It is now too short for a four-bot ladder that genuinely walks;
future runs need longer, or a smaller ladder per run.

## Run 10 — a failure that reports itself (~06:00)

`run-1788313837-06402`. Rung 1 stuck after 7 iterations, but the *shape* of the
failure is the win:

```
ERROR: could not start mining for 301 ticks: another character is standing on the iron-ore
```

Named cause, in **2,391 ticks**. The same class of problem in run 9 hung for
360 s per plan, three times, and said nothing — the bounded mining timeout
landed since. `map.jsonl` is 582 bytes rather than 0, so the start-of-run
keyframe works and an early-failing run now records a map.

The defect is the gap the tile-reservation author flagged in writing: **the tile
selector has no occupancy notion.** Before reservation, four bots raced for one
tile and the loser failed cleanly; now they are spread across adjacent tiles and
physically block each other. A bot mining tile A has to stand somewhere, and
that somewhere is tile B.

## Two operational corrections

The CLI offers `FACTORIO_BOT_REFRESH_MODS=1`, `FACTORIO_BOT_REFRESH_SCRIPTS=1`
and `FACTORIO_BOT_REFRESH_PLANS=1` — a supported per-file refresh. I have been
doing this by hand with `cp` and `rm -rf`, badly, and missing a copy.

There is also a **third** copy, `workspace/plans`, holding the same 15 script
files, and it has been stale all night. Whether anything reads it is unchecked —
worth establishing rather than assuming it is vestigial.
