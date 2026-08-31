# Vite dev server on :8080, proxying /api to a `just serve` on :7492
start:
    cd app; pnpm run start

# the real thing: axum serving the built SPA and the API on :7492
serve *ARGS:
    cargo run --release --no-default-features --features cli,lua,restapi -- serve --web-root app/dist {{ARGS}}

repl *ARGS:
    cargo repl {{ARGS}}

factorio *ARGS:
    cargo run --release --no-default-features --features cli,repl -- start -v {{ARGS}}

lua SCRIPT *ARGS:
    cargo run --release --no-default-features --features cli,lua -- lua {{SCRIPT}} {{ARGS}}

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
