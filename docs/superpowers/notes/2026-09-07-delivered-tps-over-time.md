# The delivered tick rate was already recorded; the average was hiding it

2026-09-07.

## The task, and what was actually missing

The brief was "a run records the game speed it REQUESTED and never the one it
GOT", on the premise that `events.jsonl` entries carry `tick` and **no
wall-clock field at all**, so ticks per second is not recoverable after the
fact.

**That premise is false, and has been since `abfcd2c3` (2026-09-05).**
`EventKind::BatchProgress` carries `elapsed_ms` -- wall-clock milliseconds
since the executor was handed the batch -- and the `Event` around it carries
`tick`. Two clocks, one event, written every 30 s while a batch is in flight.
`tools/run_analysis.py::delivered_tick_rate` already derived a tick rate from
exactly that pair and `just analyse` already printed it against
`60 * provenance.game_speed`.

So no new recording was added, and none was needed. The gap was one level up:

**it reported a single run-wide average, which is the one statistic that
cannot answer "when".**

## The case that proves it, from the archive

`run-1788696619-00325` (seed 31337, four bots, 1x, 2026-09-06 14:10). Its
heartbeats, as delivered tps per interval:

```
0:03 59*  0:33 60  1:03 60  1:33 60  2:03 60  2:33 55
3:00 32   3:17 39  3:36 36  3:54 27  4:08 46  4:31 50
4:56 59   5:25 60
```

Clean for two and a half minutes, **half speed for ninety seconds**, clean
again. The average is 84% of nominal -- *above* `STARVED_RATIO` (0.8) -- so
before this change `just analyse` printed

```
delivered tick rate: 50 tps of 60 nominal (84%) over 14 heartbeat interval(s)
```

with **no flag at all**, and nothing anywhere in the record distinguished it
from a run that held 60 tps throughout. This is the exact shape a cargo build
in a neighbouring worktree produces, which is the case this repo actually
hits: see the memory note *no builds during measured runs*.

It now prints:

```
delivered tick rate: 50 tps of 60 nominal (84%) over 14 heartbeat interval(s), 21148 ticks in 421 s
  ! it did NOT hold that speed throughout: 5 of 13 judged interval(s) under 80% of nominal, 151 s = 39% of the measured time
    worst 27 tps (44% of nominal) over 3:54 -> 4:08 game time
    sagged 3:00 -> 4:31 game time: 151 s of wall at 36 tps average (5 interval(s))
    profile (game time -> tps, * = batch-boundary interval, not judged):
      0:03 59*  0:33 60  ...
```

and a run that held its speed says so in one line rather than saying nothing,
because "no warning" and "nothing was checked" are the pair this project has
been bitten by four times.

## What was changed

`tools/run_analysis.py` only. No Rust, no new file format, no second
mechanism.

- `delivered_tick_rate` keeps every interval instead of summing them, and adds
  `detail`, `worst`, `sag_intervals`, `sag_ms`, `sag_share`, `sag_spans` and
  `sagged`. `starved` keeps its old meaning exactly -- the run-wide average
  fell short -- so every existing reader gets the answer it had.
- `tick_rate_profile` renders it, in both directions.
- `summary_line` carries `!starved=N%` or `!sagged=N%`, so an archive scan
  (`--all --summary`) sees it. Every other column in that listing reads
  identically whether a run got its speed or not: ticks are ticks.
- `tools/test_run_analysis_tickrate.py`, 15 tests, each falsified by breaking
  the implementation one substitution at a time (9 mutations, each matching
  exactly once, each caught).

`starved` and `sagged` are deliberately separate. A run can be starved on
average, or clean on average and slow through half of itself, and only the
second is new information.

## Two things a reader of this number must know

**The batch-boundary interval is not judged.** One interval per batch has its
tick taken from the batch's first dispatch and its wall baseline from the
batch epoch, which is earlier -- so it reads systematically slow. It is kept
in the profile marked `*` (hiding a measurement is worse than labelling one)
and excluded from `worst` and from the sag spans, because a systematic bias
must not be reported as an accusation about the machine.

**The tick on a `BatchProgress` is `FactorioRcon::last_tick`, not a fresh
query** (`LiveRecord::record` -> `recorder.not_before(rcon.last_tick())`).
That is an observation of unknown age: it is refreshed by every RCON command
the run sends, which during ordinary execution is often, and by the lag wait's
own `act.game_tick()` polls. **The failure mode is a false accusation, not a
missed one**: a batch that sends nothing for a while -- the eleven-minute
single-action stall this project has already seen -- freezes `last_tick` and
would read as 0 tps against a game that was running fine.

The fix is small and was deliberately not made here: give `BatchProgress` an
`observed_tick`, asked of the game with `act.game_tick()` at the beat
(`beat_batch_progress` already holds `act: Arc<dyn Actuator>`, so there is no
plumbing to add), and have `delivered_tick_rate` prefer it. **It is blocked on
files this session was told to stay out of**: a new field on `EventKind`
changes the utoipa schema, which fails the Rust snapshot test until
`app/src/api/openapi.snapshot.json` is regenerated and then the TypeScript
contract test until `types.ts` and `openapi.contract.spec.ts` mirror it --
`abfcd2c3` touched all three for the same reason. Handed over rather than
half-landed.

## What was NOT done, and why

**`ticks.jsonl` is not "merely gated on video", and ungating it would have
been the wrong fix.** It is a component of the video recorder: written by
`sample_ticks` inside `crates/core/src/record/video/recorder.rs`, started only
by `VideoRecorder::spawn_tasks`, interleaved with encoder-death detection,
`-progress` liveness and `df` free-space checks, and documented as stopping
when the encoder dies *because the table exists to join a video and a clock
that outlives its video reads as a healthy recording*. Driving it from the run
recorder as well would have put two writers on one format measuring one thing
-- the shape this repo has had to correct three times already (the
inserter-facing rule, encoded in three places that agree).

**No 5x-versus-10x comparison was run**, per the brief, and none could
honestly have been: load average was 29-38 through this session with five
agents building. That is exactly the contention this reporting exists to
expose, and a comparison taken under it would look like data.

## Verification

- 15 new tests pass; the neighbouring `test_run_analysis_rates.py` (39) and
  `test_run_analysis_balance.py` (32) still pass.
- Every new test falsified: 9 one-line mutations of the implementation, each
  substitution matching exactly once, each caught by a named test.
- Real archived runs: `run-1788696619-00325` reports the sag above;
  `run-1788693799-62453` (the first `just bench`) reports `speed held: every
  one of 13 judged interval(s) at or above 80% of nominal, lowest 60 tps`.
- Offline baseline unmoved: `researched:automation` = **176 actions / 21,784
  ticks** on `target/release/factorio-bot`, matching the documented figure.
  No Rust file was touched (`git diff --stat` covers `tools/` only), so the
  plan could not have moved.
- **`cargo test --workspace` was NOT run.** A from-scratch workspace build in
  a fresh worktree, at load 29-38, is the precise CPU contention that starves
  a peer's measured run -- and with zero Rust files changed it could only have
  measured somebody else's master.
