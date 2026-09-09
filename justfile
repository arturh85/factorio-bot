# Vite dev server on :8080, proxying /api to a `just serve` on :7492
start:
    cd app; pnpm run start

# the real thing: axum serving the built SPA and the API on :7492
#
# `viewer` is the `factorio-bot` feature alias for exactly this combination
# (cli, lua, restapi -- no repl, and deliberately no tokio-console, whose
# fixed debug port would otherwise contend with a game run started
# alongside this). `--all-features` also builds this but drags tokio-console
# in with it -- use this recipe or `--features viewer` instead.
serve *ARGS:
    cargo run --no-default-features --features viewer -- serve --web-root app/dist {{ARGS}}

repl *ARGS:
    cargo repl {{ARGS}}

factorio *ARGS:
    cargo run --no-default-features --features cli,repl -- start -v {{ARGS}}

lua SCRIPT *ARGS:
    cargo run --no-default-features --features cli,lua -- lua {{SCRIPT}} {{ARGS}}

# The benchmark seed. Fixed, written down, and deliberately NOT chosen for
# being a good map.
#
# Searching for a seed that scores well finds one with ore and water near
# spawn, after which every timing flatters us -- and stops being comparable to
# the ~9 minute manual solo baseline, which was not run on an optimised map.
# What benchmarking needs is a fixed, representative seed plus honesty about
# which seed produced a number. This one is the date the discipline started.
BENCHMARK_SEED := "31337"

# A reproducible benchmark run on BENCHMARK_SEED.
#
# DESTRUCTIVE. `--new` deletes workspace/server/saves/level.zip and everything
# built on it, because that is the ONLY way a seed takes effect: `--seed`
# without `--new` is silently ignored on a workspace that already has a map
# (see "Reproducible runs" in CLAUDE.md). Passing the seed without the deletion
# would be worse than passing neither -- the run would look controlled and not
# be.
#
# The seed has NOT been validated by a run yet. A map whose nearest shoreline
# does not fit a pump/boiler/engine has genuinely refused a run here; confirm
# this one produces a viable map before quoting any number against it.
# The roster a quoted benchmark uses. FOUR, stated here rather than inherited.
#
# `--clients` defaults to 1 in the CLI (app/src-tauri/src/cli/lua.rs), and this
# recipe used to pass no client count at all -- so `just bench` was a ONE-BOT
# run while reading as the project's benchmark. Its first ever execution
# (2026-09-06, run-1788693799-62453) produced `roster: [1]`, `plan_created.bots
# = [1]`, and a 7:22 automation milestone that is not comparable to the 6:05
# four-bot record. Nothing failed and nothing stalled: one client was all that
# was ever requested.
#
# That is the failure this repo warns about from the other direction --
# "a one-bot run misread as a four-bot regression cost a good commit a revert"
# -- and the defaulting is what made it silent. A benchmark must state its
# roster, because the roster is half of what the number means.
BENCH_CLIENTS := "4"

# THE ONE RECIPE THAT KEEPS `--release`, and it keeps it on purpose.
#
# Every other recipe here dropped `--release` on 2026-09-07 because a plain
# `cargo build` now plans as fast as a release binary and rebuilds 13x cheaper
# (docs/superpowers/notes/2026-09-07-what-a-build-profile-costs-the-loop.md).
# A MEASURED run is the exception: it has to be built the way the deliverable
# is built, or the number describes a binary nobody ships. That constraint did
# not go away when the default changed -- it just became the thing that has to
# be said out loud, which is what this comment is for.
#
# Same reasoning as BENCH_CLIENTS above: state it, do not inherit it.
bench SCRIPT *ARGS:
    cargo run --release --no-default-features --features cli,lua -- lua {{SCRIPT}} --seed {{BENCHMARK_SEED}} --new --clients {{BENCH_CLIENTS}} {{ARGS}}

# Headless: bots are server-side characters, no graphical client, world at
# HEADLESS_SPEED. Seconds to start instead of minutes; same record, no video.
# Use it to iterate; use `bench` (clients, 1x, filmable) for a number you will
# quote.
#
# **10, raised from 5 on 2026-09-07 because 5 was never chosen.** It was a
# hard-coded constant with no recorded justification, and the box was never
# asked what it could deliver. Measured on this machine, same script and map,
# four headless bots, whole-run ticks over wall seconds:
#
#   speed   game ticks   wall    delivered      tick cost vs 5x
#     5x       36,253    122 s   297 of 300      --
#    10x       36,811     63 s   584 of 600     +1.5%
#    20x       38,320     34 s  1127 of 1200    +5.7%
#
# So 10x nearly HALVES iteration wall time for a tick cost small enough to be
# run-to-run noise. 20x is real and usable -- it held 94% with two cargo builds
# running -- but its +5.7% is probably the lag waits' sleep granularity, and a
# game-time measurement is what this project quotes, so it is opt-in rather
# than the default: `just headless script.lua --game-speed 20` still works.
#
# One sample per speed. If a decision rests on the tick cost, take more.
HEADLESS_SPEED := "10"

headless SCRIPT *ARGS:
    cargo run --no-default-features --features cli,lua -- lua {{SCRIPT}} --headless --bots 4 --game-speed {{HEADLESS_SPEED}} {{ARGS}}

# The replan check: plan the science cell offline, apply what it built to the
# world, and plan it AGAIN from the standing world -- no Factorio, seconds.
#
# Every baseline this project takes is an offline plan from the t=0 dump, and
# a t=0 dump has no factory in it. On 2026-09-09 a fix moved all eight
# baselines correctly and refused in the live run with the exact blocker it
# had been written to remove: "a cell ALREADY MAKES copper-plate", a sentence
# only a replan can say. Run this beside the baselines for any change that
# reads standing entities -- siting, routing, reservations, recovery.
#
# `--all` because the script plans `goal.all{sustain, producing}` as ONE
# bundle, and a bundle holds one conjunct's "already standing" refusal back
# while the other expands; two sequential --goals would refuse on the replan
# for a reason the run never sees. `--done-by <tick>` cuts the first plan the
# way a truncated batch would (the live replans followed a failed take, not a
# finished plan); without it the whole plan is applied. `--fail <label>` is
# the live shape -- that action fails and its dependency cone is abandoned --
# and the one that reproduces run-1788923927-04849's refusal with a69ae64c
# reverted. `--standing-from-run <run> --at-tick <T>` instead starts from
# what a finished run's record says stood. The same checks run as
# `crates/planner/tests/replan_on_standing_world.rs` and `replan_haul.rs`.
replan-check *ARGS:
    cargo run --no-default-features --features cli,lua -- plan --world workspace/scripts/map.json --bots 1,2,3,4 --all --replan 1 --goal sustain:copper-plate:15:36000 --goal producing:automation-science-pack:6 {{ARGS}}

# Fast iteration: connect to already-running Factorio (start with 'just factorio' first)
lua-connect SCRIPT *ARGS:
    cargo run --no-default-features --features cli,lua -- lua --connect {{SCRIPT}} {{ARGS}}

# Verify only -- never rewrites a file. Safe to run on a dirty tree, and safe
# when more than one person or agent is working in the same checkout.
test:
    cargo fmt --all -- --check
    cargo clippy --workspace --tests -- --deny warnings
    cargo test --workspace --quiet
    cargo build --release

# Apply the fixes `test` only reports. Rewrites files across the whole
# workspace, so run it on a tree whose uncommitted changes are all yours:
# `clippy --fix --allow-dirty` deliberately overrides the guard that would
# otherwise refuse, and `fmt --all` does not ask either.
fix:
    cargo fmt --all
    cargo clippy --fix --workspace --tests --allow-dirty

# Where did a run's time go? Point it at an archived run directory.
#
# Reads only, and tolerates a run that is still being written, so it is safe
# to aim at a live game run. `just analyse` with no argument is the whole
# archive, one line each; pass a directory for the full accounting.
analyse *ARGS:
    python3 tools/run_analysis.py {{ if ARGS == "" { "--all --summary" } else { ARGS } }}
