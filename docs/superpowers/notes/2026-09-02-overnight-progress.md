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
