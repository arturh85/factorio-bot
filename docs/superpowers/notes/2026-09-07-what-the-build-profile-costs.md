# What the build profile costs, measured

2026-09-07, worktree `.worktrees/build-speed`, branch
`measure-what-the-build-profile-costs`, off `master` at `76709a2b`.
Three hypotheses were put: add a fast linker, loosen the release profile, and
reconsider `opt-level = "s"`. **One of the three is a clear win, one is a
trade this repo should probably refuse, and one is not a win at all.**

## The correctness gate, first

An optimisation setting must not change a plan. It did not.

| goal | actions / makespan |
|---|---|
| `researched:automation` | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,457 |
| `producing:logistic-science-pack:6` | 441 / 47,478 |
| `gathered:crude-oil` (on `map-31337-explored.json`) | 2,115 / 317,283 |

All four hold on **all four** configurations. Stronger than the headline
numbers: the full `--steps` output is **byte-identical across all four
configurations**, 3,851 lines compared (241 + 414 + 593 + 2,603), after
stripping the `tracing` stderr lines that carry timestamps.

## The measurements

`jobs = 4` throughout, sccache on, seed-31337 `map.json`. **Load average is
given beside every timing (1-min / 5-min, 20 cores), because these are
wall-clock measurements on a box several agents build on.**

Wall **and** cpu (user+sys of the process tree) are both reported. This repo's
standing rule is that a measurement a wall clock can move is a broken
instrument and should be bound in something invariant; for a build, total CPU
work is the closest thing to that invariant. It is not perfect — sccache
compiles in a server process outside the measured tree, and memory-bandwidth
contention still inflates it — but **it moved by percent where wall moved by
multiples**, which is exactly what a run needs to survive this box.

| config | linker | lto | cu | opt | cold wall / cpu (load) | warm wall (load) | warm cpu | binary |
|---|---|---|---|---|---|---|---|---|
| `base` (today) | GNU ld | fat | 1 | s | 104.3 / 161.0 (3.5→4.2) | 58.3 (4.2) · 63.5 (2.9) · 122.7 (11.2) | **60.6 · 62.5 · 69.7** | 29,667,496 |
| `mold` | mold | fat | 1 | s | 374.3 / 174.1 (19.5→12.0) | 163.9 (12.0) · 89.6 (27.8) · 152.4 (28.6) | **66.2 · 65.2 · 71.3** | 29,679,088 |
| `thin` | mold | thin | 16 | s | 316.8 / 346.4 (23.8→34.4) | 120.8 (34.4) · 62.5 (25.9) · 123.0 (33.2) | **210.6 · 216.7 · 208.2** | 44,005,384 |
| `thin3` | mold | thin | 16 | 3 | 267.4 / 431.7 (19.0→33.6) | 56.1 (33.6) · 23.6 (32.5) · **22.2** (30.5) | **311.9 · 316.7 · 310.4** | 42,580,704 |

Runtime probes, same run:

| config | `plan` cpu (s) | oracle cpu (s) | oracle wall (load) |
|---|---|---|---|
| `base` | 2.1 · 2.2 · 2.0 | 41.0 · 34.4 | 56.8 (22.5) · 37.8 (18.6) |
| `mold` | 1.1 · 1.1 · 1.1 | 33.2 · 34.0 | 37.0 (18.9) · 37.0 (10.6) |
| `thin` | 1.4 · 1.2 · 1.2 | 37.9 · 46.4 | 119.4 (60.5) · 63.4 (33.5) |
| `thin3` | 1.2 · 1.1 · 1.0 | 32.9 · 30.9 | 40.4 (42.8) · 59.2 (31.3) |

**A worked example of why cpu is quoted.** `base warm3` is 122.7 s of wall
against 58.3 s for `base warm1` — the same rebuild, twice as slow — while cpu
moved 60.6 → 69.7, i.e. 15%. The load went 4 → 16 between them. Wall alone
would have reported a 2x regression in a configuration that did not change.

## 1. A fast linker: measured, and it is NOT the win

`mold` 2.42.0 was added to the dev shell and pointed at through
`build.rustflags`. It is genuinely doing the linking — `readelf -p .comment`
on the built binary says `mold 2.42.0 (compatible with GNU ld)`, and the
`base` binary says nothing, so the two arms really are different.

**Warm-rebuild cpu is 66.2 / 65.2 / 71.3 with mold against 60.6 / 62.5 / 69.7
without.** That is within noise, and if anything slightly worse. Cold cpu is
174.1 against 161.0. The binary is 11,592 bytes larger.

The reason is structural rather than a fact about mold: **under `lto = true`
with `codegen-units = 1` there is almost no ELF linking to speed up.** The
work is LLVM's fat-LTO codegen inside rustc, which produces essentially one
huge object; the linker then has little left to do. A fast linker pays when
there are many objects to combine, which is precisely the configuration this
repo does not use.

**This contradicts the brief that commissioned it**, which put a fast linker
first as "usually the single largest build-time win". That is true of most
Rust workspaces and false of this one, and the reason it is false is the same
setting that section 2 is about.

Kept anyway, at zero cost: mold stays in `flake.nix` and stays wired up,
because it becomes the win the moment `codegen-units` is loosened, and because
it costs 3.3 s of shell evaluation once (fetched from cache.nixos.org) and
nothing thereafter.

### The trap in wiring it up, and the proof it was avoided

`.cargo/config.toml` carries `build.rustflags = ["--cfg", "tokio_unstable"]`.
**Cargo does not merge `build.rustflags` with `target.<triple>.rustflags` — the
target one wins outright and the build one is ignored entirely**, so adding a
`[target.x86_64-unknown-linux-gnu]` block for the linker would have silently
dropped `--cfg tokio_unstable` and broken `tokio-console` with no build error.

Avoided by extending `build.rustflags` instead, so both flags live in one
array and composition never arises. Proven positively rather than assumed, in
one command:

```
rustc --cfg tokio_unstable -C link-arg=-fuse-ld=mold main.rs -o t1
./t1                              -> tokio_unstable REACHED rustc
readelf -p .comment t1            -> mold 2.42.0 (compatible with GNU ld)
```

Both halves at once: the cfg reached rustc **and** mold did the link.
`cargo build --release --features tokio-console` also completes (6m 21s).

## 2. `codegen-units` is the whole story — and it collides with `jobs = 4`

The number that explains everything above is **parallelism, cpu ÷ wall**, read
off the least-contended repeat of each configuration:

| config | warm cpu ÷ wall |
|---|---|
| `base` (cu = 1, fat) | **~1.0** |
| `thin` (cu = 16) | ~3.5 |
| `thin3` (cu = 16) | **~14** |

**Today's warm rebuild is effectively serial.** `codegen-units = 1` plus fat
LTO means one core does the work while the other nineteen idle, which is why
`jobs = 4` costs this repo nothing today and why mold has nothing to fix.

Loosening it works, and works hard: `thin3`'s warm rebuild is **22.2 s at load
30** against `base`'s **58.3 s at load 4**. That is roughly 3x faster measured
under conditions six times worse — the true ratio is larger, not smaller.

**And here is the finding that is about this repo rather than about
compilers.** `jobs` caps cargo's parallel **rustc processes**. `codegen-units`
multiplies threads **inside each one**. They are different limits and the
first does not bound the second. So a `cu = 16` build is not a 4-job build:
it is one that reached ~14x parallelism and **~310 cpu-seconds per warm
rebuild against ~62 today, 5x the total CPU**.

`jobs = 4` exists because five concurrent agent builds once asked for 100
parallel jobs on a 20-core box and drove load to 85 with 18 GB of swap. Five
concurrent `cu = 16` builds would ask for something in the same
neighbourhood while appearing to respect the cap — the config file would still
read `jobs = 4`.

**So this change is a clear win solo and a clear loss in our actual working
pattern**, and those are two different questions:

- one agent, quiet box: 3x faster iteration, take it;
- five agents, which is the normal case here: 5x the CPU each, and the cap
  that was put there to stop exactly this no longer binds.

Stated in those terms because the owner can decide it and the reasoning
survives the decision. **Not landed on that basis** — the numbers are here,
the call is not mine. If it is taken, `codegen-units` wants to be roughly
`jobs`, not 16, so that the two limits compose instead of multiplying; that
configuration was not measured and should be before it ships.

Note also `thin` (opt `s`) is a **44.0 MB** binary against `base`'s 29.7 MB —
+48%. Thin LTO and 16 codegen units both reduce cross-module optimisation, and
the size shows it.

## 3. `opt-level = "s"` is optimising for the wrong thing, and does not even win at it

Nobody knew why `"s"` was chosen, so it was treated as possibly deliberate.
The comparison is `thin` versus `thin3` — same linker, same `lto`, same
`codegen-units`, differing only in `opt-level`:

| | `opt-level = "s"` | `opt-level = 3` |
|---|---|---|
| `plan` cpu (median of 3) | 1.2 s | **1.1 s** |
| oracle cpu (mean of 2) | 42.2 s | **31.9 s** |
| **binary size** | 44,005,384 | **42,580,704** |

**`opt-level = 3` is faster on both runtime probes AND 1.42 MB (3.2%)
smaller.** The setting chosen to save size costs size here. That is not the
usual outcome and it is worth restating plainly: at `lto = "thin"` with
`codegen-units = 16`, `"s"` is dominated on every axis measured.

The oracle figure is the weaker of the two — `"s"`'s two readings were taken at
load 60 and 33 against `3`'s at 42 and 31, and it reads a 2.9 GB dump where
page-cache state matters. Treat "roughly 20-25% faster" as directional. The
`plan` figure is tighter: three repeats each, all with the map warm, and the
size figure is exact and load-independent.

**The gap, stated rather than papered over: this was measured only at
`thin`/`cu = 16`.** The configuration that would decide the *shipped* profile —
fat LTO, `codegen-units = 1`, `opt-level = 3` — was never built. Optimisation
levels interact with LTO, so this result should not be transferred to the
shipping profile without measuring it there. **That is the one build worth
doing next, and it is a single cold build.**

## What I would and would not change

- **Keep mold.** It costs nothing, it is correctly wired, and it is
  prerequisite to any future `codegen-units` change. Do not expect it to speed
  anything up today, because measured, it does not.
- **Do not loosen `[profile.release]` for shipping.** The binary is the
  deliverable; fat LTO and `cu = 1` are right for it, and the plans are
  identical either way so there is no correctness argument for changing it.
- **Two profiles, if the owner wants the iteration win** — `[profile.release]`
  stays as it is for shipping, and a separate faster profile
  (`[profile.quick]`, inheriting release) carries `lto = "thin"` and a
  `codegen-units` chosen to compose with `jobs`. `just serve`, `cargo repl` and
  the offline `plan`/`score-map` loop would default to it; `just bench` and any
  measured run would stay on `release`, because a measured run must be built
  the way the deliverable is.
- **Measure `opt-level = 3` at fat/cu1 before deciding the shipped profile.**
  The evidence that `"s"` is a mistake is real but was gathered at the wrong
  LTO setting to act on for shipping.

## What was not measured, and why

- **Round 3, the interleaved re-run, did not finish.** Round 2 measured the
  four configurations back to back while the box's load drifted from 3 to 34 —
  so `base` was measured on a quiet box and `thin`/`thin3` on a busy one, and
  every cross-configuration **wall** comparison in the table above is
  confounded with load in the direction that *understates* the loosened
  configurations. The cpu column is what the conclusions rest on; it is
  reproducible to a few percent within each configuration across three
  repeats. Round 3 (per-configuration persistent target directories, warm
  rebuilds cycled A-B-C-D so each cycle shares whatever the box is doing)
  completed only `base cold` — 432.8 s wall / 178.7 cpu at load 22 — before
  it was stopped. `measure/round3.sh` is committed and will run as-is.
- **The box never went quiet.** The five-minute load was 8-12 at the start and
  22-34 for most of the run. No timing here was taken on an idle machine and
  none should be quoted as if it were.
- **`opt-level = 3` at fat LTO / `cu = 1`** — the shipping-profile question,
  above.
- **`codegen-units` matched to `jobs`** — the configuration that would actually
  be proposed, as opposed to the 16 that was tested.

The harness is in `measure/`: `apply.sh` rewrites the two files (preserving
Cargo.toml's CRLF, or every line reads as changed), `bench.sh` times with
bash's builtin `time` — **NixOS has no `/usr/bin/time`, and reaching for it
cost one whole round, four configurations, rc=127, zero seconds** — and
`round2.sh` / `round3.sh` drive it.
