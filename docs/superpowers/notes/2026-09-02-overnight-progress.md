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
