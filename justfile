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
    cargo run --release --no-default-features --features viewer -- serve --web-root app/dist {{ARGS}}

repl *ARGS:
    cargo repl {{ARGS}}

factorio *ARGS:
    cargo run --release --no-default-features --features cli,repl -- start -v {{ARGS}}

lua SCRIPT *ARGS:
    cargo run --release --no-default-features --features cli,lua -- lua {{SCRIPT}} {{ARGS}}

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
bench SCRIPT *ARGS:
    cargo run --release --no-default-features --features cli,lua -- lua {{SCRIPT}} --seed {{BENCHMARK_SEED}} --new {{ARGS}}

# Headless: bots are server-side characters, no graphical client, world at
# 5x. Seconds to start instead of minutes; same record, no video. Use it to
# iterate; use `bench` (clients, 1x, filmable) for a number you will quote.
headless SCRIPT *ARGS:
    cargo run --no-default-features --features cli,lua -- lua {{SCRIPT}} --headless --bots 4 --game-speed 5 {{ARGS}}

# Fast iteration: connect to already-running Factorio (start with 'just factorio' first)
lua-connect SCRIPT *ARGS:
    cargo run --release --no-default-features --features cli,lua -- lua --connect {{SCRIPT}} {{ARGS}}

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
