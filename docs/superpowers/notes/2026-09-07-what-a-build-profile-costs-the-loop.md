# What a build profile costs the loop we live in

2026-09-07, worktree `.worktrees/iteration-profile`, branch
`a-profile-for-the-loop-we-live-in`, off `master` at `9e628c22`.

This continues `2026-09-07-what-the-build-profile-costs.md`, which measured what
the *shipping* profile costs. The question changed three times while this ran,
and the third framing is the one that matters:

> **"I see no reasons for release builds ever, only when CI creates real
> releases."** — owner

So the question is not "which second profile do we add". It is **what should a
developer or an agent type**, and what must that profile carry to be usable.

## The one-sentence answer

**Fat LTO is what makes a rebuild expensive, and `opt-level` is what makes the
binary slow — they are two separate knobs and this repo had them tied
together.** Dropping fat LTO makes the edit-rebuild loop **13x cheaper in CPU**
and costs the offline `plan` loop **nothing**, provided `opt-level` stays at 1
or above. `opt-level = 0` is the setting that is 7.7x slower at runtime, and it
is the only one that is.

## The correctness gate, first, because nothing else counts without it

An optimisation setting must not change a plan. Across **seven** configurations:

| goal | actions / makespan |
|---|---|
| `researched:automation` | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,457 |
| `producing:logistic-science-pack:6` | 441 / 47,478 |
| `gathered:crude-oil` (`map-31337-explored.json`) | 2,115 / 317,283 |

Stronger than the four summary numbers: the full `--steps` output is
**byte-identical to `base` on every configuration**, **19,255 lines compared**
(241 + 414 + 593 + 2,603 per config, across six non-base configs and four
goals), after stripping `tracing`'s stderr lines.

**Strip ANSI before you filter timestamps.** A `grep -v '^[0-9]{4}-'` filter
matched nothing, because each `tracing` line begins with an ANSI escape and not
with its own timestamp — so all four oil comparisons reported DIFFERS on a
run where nothing differed. **The tell was that every configuration differed
from base in the same file, including one that differs from base by a single
`opt-level`** — a uniform difference across configurations is a property of the
comparison, not of the thing compared.

## The grid

`jobs = 4`, sccache on, mold on (it is in `build.rustflags` on master already).
Cold builds are a fresh `CARGO_TARGET_DIR`; warm is `touch
crates/planner/src/lib.rs` then `cargo build --release`, which really does
recompile five crates and relink — verified from the build log and the
binary's mtime, because a 2-second "rebuild" is exactly what a no-op looks
like. Three repeats of warm and plan, interleaved across configurations so the
box's mood is shared rather than assigned to whoever was unlucky.

**Load is given beside every timing.** The box ran 25-35 for the first hour and
2-8 for the rest; that is why cpu, not wall, carries the argument.

| config | lto · cu · opt | cold cpu | cold wall (load) | **warm cpu** ×3 | **plan cpu** ×3 | oil wall | binary |
|---|---|---:|---|---|---|---:|---:|
| `base` (today) | fat · 1 · s | 171.9 | 377.6 (13→28) | **59.8 · 60.3 · 59.7** | 1.3 · 1.1 · 1.3 | 107 s | 29.69 MB |
| `fat3` | fat · 1 · 3 | 283.1 | 310.0 (11→31) | **82.3 · 82.8 · 85.9** | 1.0 · 1.4 · 1.2 | 87 s | 35.16 MB |
| `it0` | off · 4 · 0 | **94.9** | 117.4 (6→12) | 4.3 · 4.3 · 5.2 | **10.0 · 10.3 · 10.4** | **966 s** | 120.91 MB |
| `it1` | off · 4 · 1 | 194.3 | 170.6 (10→14) | **4.4 · 4.4 · 5.0** | **1.3 · 1.3 · 1.3** | 108 s | 42.88 MB |
| `it2` | off · 4 · 2 | 255.6 | 280.4 (2→14) | 4.6 · 4.4 · 5.1 | 1.1 · 1.1 · 1.1 | 99 s | 40.60 MB |
| `it3` | off · 4 · 3 | 247.9 | 144.6 (2→7) | 4.3 · 4.3 · 4.3 | 1.1 · 1.1 · 1.1 | 98 s | 40.46 MB |
| `it1c8` | off · 8 · 1 | 196.0 | 125.6 (4→11) | 4.5 · 4.4 · 5.3 | 1.3 · 1.3 · 1.3 | 112 s | 43.26 MB |

`base`'s warm cpu of 59.8/60.3/59.7 reproduces the previous round's
60.6/62.5/69.7, and its cold 171.9 reproduces 161.0 — so this is the same
instrument measuring the same thing, on a box that was much quieter this time.

## What the grid says

**1. The warm rebuild is 13x cheaper without fat LTO, and `opt-level` does not
matter to it.** 4.3-5.3 cpu at `opt` 0, 1, 2 *and* 3, against 59.8 for `base`.
Fat LTO redoes whole-program codegen on every edit; that is the entire cost.
This is the number that governs the owner's actual wait, because the
edit-rebuild loop is what a session does dozens of times.

**2. `opt-level = 0` is disqualified, and it is the only one that is.** Plan cpu
**10.0/10.3/10.4 against 1.3** — 7.7x, which is precisely CLAUDE.md's "debug is
4-7x slower at runtime". The oil plan on the 1.4 GB explored map takes **966 s
against 107 s**, 9x. The binary is **120.9 MB**, four times `base`. That is the
runtime floor the brief asked about, and it is real — but it belongs to
`opt-level = 0` specifically, **not to dropping LTO**, which is what the old
guidance implicitly blamed.

**3. `opt-level = 1` costs nothing at runtime.** Plan cpu 1.3/1.3/1.3 against
`base`'s 1.3/1.1/1.3; oil 108 s against 107 s. One notch of optimisation
recovers the whole 7.7x.

**4. `opt` 2 and 3 buy a further ~15% of runtime for ~30% more cold cpu.** Plan
1.1 against 1.3, oil 98-99 s against 108, for cold 248-256 against 194. Warm is
unchanged, so this trade is paid once per fresh target directory and earned back
over roughly 270 plan invocations. Both are defensible; `opt = 1` is the cheaper
default and `opt = 2` the better one if the loop is plan-heavy.

**5. `codegen-units` 4 → 8 is free in CPU and better in wall.** 196.0 vs 194.3
cold cpu (+0.9%, noise) for 125.6 s vs 170.6 s wall. **This is unlike the
previous round's `cu = 16` result**, which cost 5x the CPU — that was measured
at *thin LTO*, where cu multiplies LTO work. Without LTO there is no such
multiplication, so the `jobs`-vs-`codegen-units` collision that round 2 warned
about does not arise here. `cu = 4` remains the conservative choice; 8 is
measured and safe.

## Round 5: can `cargo build` carry the loop?

The owner's third framing makes this the real question. `[profile.dev]` ships
`opt-level = 0` with `[profile.dev.package."*"] opt-level = "z"` — dependencies
optimised for *size*. Four dev configurations, same harness, same gate (**all
four plan byte-identically to `base`**, so eleven configurations now pass it,
and a dev build plans identically to a fat-LTO release build):

| dev profile | ours · deps · debug | cold cpu | **warm cpu** ×3 | **plan cpu** ×3 | binary |
|---|---|---:|---|---|---:|
| `dtoday` (as shipped) | 0 · z · on | 229.3 | 5.1 · 5.0 · 5.3 | **6.2 · 5.9 · 5.9** | 260.9 MB |
| **`d1`** | **1 · 2 · on** | 694.5 | 5.0 · 4.8 · 4.7 | **1.2 · 1.2 · 1.2** | 270.3 MB |
| `d1nodbg` | 1 · 2 · **off** | 578.0 | 4.4 · 4.4 · 4.3 | 1.2 · 1.2 · 1.3 | **42.4 MB** |
| `d2` | 2 · 3 · on | 857.0 | 5.4 · 5.0 · 4.9 | 1.1 · 1.1 · — | 293.0 MB |

**The answer is yes, and it needs one line of configuration.** `d1`'s plan cpu
is **1.2**, against `base`'s 1.3 — a `cargo build` binary plans *slightly
faster than today's shipped `cargo build --release` binary*. Today's dev
profile is **6.2**, i.e. 4.8x slower, which is where CLAUDE.md's "debug is 4-7x
slower" comes from. **The whole gap is `opt-level = 0` on our own crates**, and
one notch closes it.

**Warm rebuild is ~5 cpu for every dev configuration**, the same as every
`lto = false` release configuration. So `cargo build` and a de-LTO'd
`cargo build --release` cost the same to iterate on; the only thing that
separates them now is `debug_assertions` and debug info.

**Debug info costs 117 cpu and 228 MB.** `d1` 694.5 cpu / 270.3 MB against
`d1nodbg` 578.0 / 42.4 MB. Warm and plan are unaffected. That is the price of
being able to use a debugger, stated so it can be chosen rather than inherited.

### The cold column has a confound, and it is worth naming

`d1`'s cold cpu of 694.5 against `dtoday`'s 229.3 is **not** "opt 2 on deps
costs 3x". **sccache keys on compile flags.** `dtoday` and `base` are the
*shipped* flag sets, so their dependencies were already in the shared cache
from other worktrees; every other configuration in rounds 4 and 5 was novel and
paid a fully uncached dependency tree. The cache accumulated **19,081 Rust
misses** across this session and its hit rate fell from 70% (recorded
yesterday) to 31%.

So **every cold-cpu comparison across configurations here is biased in favour
of today's settings**, and the true steady-state cold cost of the alternatives
is lower than measured. **Warm and plan are immune**: sccache refuses
incremental compilation, so workspace crates are never cached, and a warm
rebuild only recompiles workspace crates. That is a third reason the warm
column, not the cold one, carries every conclusion in this note.

A corollary worth writing down: **adopting any new profile costs one uncached
dependency build per worktree, once.** It is a real cost and it is paid once.

## A published number that does not survive re-measurement

Branch `fat-lto-at-opt-three` (`bde664b6`) says `opt-level = 3` at fat LTO is
"**35% SMALLER** — 19.19 MB against 29.67". Measured here like-for-like, both
from a plain `cargo build --release`:

```
base (opt "s")   29,691,784
fat3 (opt 3)     35,156,856     +18.4% LARGER
```

The 19.19 MB binary exists and is on disk in `.worktrees/opt3-fat`. It contains
**no `swagger-ui` strings**, so it was built without the `restapi` feature —
compared against a 29.67 MB *default-feature* build. **The size claim compares
two different feature sets.** The build-cost half of that commit does reproduce
(283.1 cpu here against 268.9 there, +65% over `base`'s 171.9), and the
plan-speed half is roughly right (1.0-1.4 against 1.1-1.3, i.e. no clear win at
this map size rather than the claimed 2x).

The general shape, which this repo keeps paying for: **a size or speed number is
meaningless without the feature set beside it**, and `--no-default-features
--features cli,lua` is a *different binary* from `cargo build --release`.

## What landed, and what each command should use

Only `[profile.dev]` changed. **`[profile.release]` is untouched**, because the
one change proposed for it does not survive measurement (above).

```toml
[profile.dev]
opt-level = 1              # was 0 -- the whole runtime floor lives here
[profile.dev.package."*"]
opt-level = 2              # was "z" -- optimised deps are nearly free
```

| command | profile | why |
|---|---|---|
| `just serve`, `just factorio`, `just lua`, `just lua-connect` | **dev** | `--release` dropped; plans as fast, rebuilds 13x cheaper |
| `just headless` | dev | already was |
| `cargo test --workspace` | dev | unchanged command, now on a faster profile |
| the offline `plan` / `score-map` loop | dev | 1.2 s cpu, against 1.3 s on the old release binary |
| **`just bench`, and any measured run** | **release** | **a measured run must be built the way the deliverable is** |
| `cargo release`, CI artefacts | release | unchanged: fat LTO, `cu = 1`, `opt-level = "s"` |

**The one rule that does not change**: a number you are going to quote comes
from a `--release` build. That constraint did not disappear when the default
flipped; it stopped being implicit, which is why `just bench` now carries a
comment saying so rather than relying on the flag being noticed.

## The measurement I contaminated, and how

**`cargo test --workspace` cost per profile is UNMEASURED, and it is my fault.**
While round 5's test phase was running `dtoday`, I ran `apply-dev.sh 1 2 true`
in the same worktree to set up a `tokio-console` check. That rewrote
`Cargo.toml` mid-flight, so `dtoday`'s `cargo test` saw a changed profile and
**rebuilt the whole workspace during the run step** -- 472 `Compiling` lines in
a log that should have had none, and 1,414.9 cpu against `d1`'s 52.2.

**The 27x that appeared to be a result is an artifact of my own edit**, and the
apparent control (sum of per-block finish times, 53.3 s vs 52.4 s) is `d1`
measured against `d1`, because the rebuilt binary carried d1's settings. Both
numbers are void.

Two things worth keeping from it. **The tell was that the two runs had the same
slowest test to two decimal places** (35.33 s and 35.32 s) while their totals
differed 27-fold -- a ratio that large with identical per-test times is not a
property of the code, and checking the log for `Compiling` lines found the cause
in one command. And the general rule: **do not touch a shared file while a sweep
that reads it is running**, which is the same hazard as editing a bash script
that bash is executing, met twice in one session.

What does stand: **`cargo test --workspace` is green on the landed profile** --
2,965 tests, 110 `test result: ok` blocks, 0 failures, exit code taken from the
command itself.

## What I did not measure

- **`cargo test --workspace` cost per profile** — attempted and contaminated,
  see above. It is plausibly the most-run command in the repo and its number
  still belongs here. Re-running it needs nothing but an undisturbed tree.
- **`d1nodbg` and `d2` test costs** — the sweep was still running them when this
  was written; whatever they say, they are subject to the same caution.
- **Whether `debug = 0` on `[profile.dev]` is worth taking.** It saves 117 cpu
  cold and 228 MB of binary and costs nothing warm or at runtime — but it takes
  away the debugger, which is the entire reason the dev profile carries debug
  info. Not landed; it is a preference, not a measurement, and the numbers are
  in the table for whoever wants to decide it.
- **The flow-graph oracle** at these configurations. The previous round measured
  it; a 2.9 GB dump makes it page-cache sensitive and it was not worth a
  confounded number.
- **A quiet box for the first hour.** `base` and `fat3` cold were taken at load
  13-31 while `it2`/`it3` were taken at 2-7. Their *wall* numbers are therefore
  not comparable across that boundary; their cpu numbers are, and the cpu column
  is what every conclusion above rests on.
