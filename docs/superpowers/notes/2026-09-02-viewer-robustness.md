# 2026-09-02 — viewer robustness: three defects

## Status

Done. All three defects fixed, tests added, all required gates green:
`cargo fmt --check`, `cargo clippy --workspace --all-features --all-targets
-- --deny warnings`, `cargo test --workspace` (default features), `pnpm
lint`, `pnpm run test:coverage` (96.66/92.7/96.87 stmts/branch/lines, all
above the 90/90/80 gate), `pnpm run build:web`.

Note: `clippy --workspace --tests` (no `--features` flag, i.e. default
features only, which excludes `restapi`) shows two pre-existing failures
(`dead_code` on `Context`'s `restapi_handle`/`app_settings`/`settings_path`
fields, and a `needless_return` in `lib.rs:111`) unrelated to this change —
neither touched file nor line is part of my diff. The actual required gate,
`--all-features`, is clean.

## Commit SHAs

- `765a743f` — fix(runs): degrade the run viewer per-stream instead of
  failing on one missing route
- `3a85b5e6` — fix(server): don't let tokio-console's port bind failure
  abort the process
- `eb45cebb` — build: add a `viewer` feature alias for building the axum
  server

(All landed on the branch that was checked out as HEAD when I started, which
turned out to be `master`, not `feat/axum-server` as the initial branch
snapshot claimed — the repo had already moved on by many commits, including
several from what looks like an unrelated overnight research session,
before I touched anything. I did not switch or create branches; I only
committed onto HEAD as instructed.)

## Defect 1 — page degrades per stream

`runsStore.openRun()` now fetches `getRun(id)` alone and lets its failure be
the fatal one (`store.error`, run truly can't be opened). The four
enrichments — frames, lanes, samples, map — are fetched with
`Promise.allSettled` instead of `Promise.all`. Each failure sets that stream
to empty and records a message in its own field (`frameError`, `lanesError`,
`sampleError`, `mapError`) via a new `enrichmentUnavailable()` helper that
distinguishes a 404 ("this server does not provide `/samples`") from any
other failure (network error, 500 — the raw message).

`RunsPage.vue` shows each message next to the panel it would have filled:
the lanes block, the frame picker/image block, the map, and each of the
three world-state panels (research/production/inventory, which all share
`sampleError`) each check their own error first, before falling back to
their previous "genuinely empty" text. Styled with a new `.stream-warning`
class (amber, boxed) distinct from the red `.runs__error` used for "the run
itself couldn't be opened" — so a missing stream reads as a partial problem,
not a broken page.

Tests added in `runsStore.spec.ts`: one where `getRunSamples` rejects (404)
and the run still opens (detail/frames/bounds intact, only `samples` empty
and `sampleError` set); one where `getRunSamples` (404) and `getRunMap`
(generic error) both fail simultaneously and each error names its own
stream/route/reason without bleeding into the other, while untouched streams
(`lanesError`, `frameError`) stay `null`.

## Defect 2 — console port is now configurable / non-fatal

Yes, configurable — it already was, just undocumented here:
`TOKIO_CONSOLE_BIND=host:port` (upstream `console_subscriber` env var,
`127.0.0.1:6669` by default). `Context::new` (`app/src-tauri/src/context.rs`)
now resolves that same address itself and probes it with a throwaway
`TcpListener::bind` *before* calling `console_subscriber::init()`. On success
it drops the probe and proceeds normally. On failure (port taken) it prints
a warning to stderr naming the address and `TOKIO_CONSOLE_BIND`, and falls
back to the same plain `tracing_subscriber::fmt` setup a non-tokio-console
build uses (factored into `init_plain_tracing()`), instead of letting
`console_subscriber`'s own `.expect()` panic — which this workspace's
`panic = "abort"` release profile turns into a whole-process abort.
Verified by holding port 6669 with a Python socket and running a
`tokio-console`-featured debug build: it now warns and exits 0 instead of
aborting.

## Defect 3 — documented viewer build

Added `viewer = ["cli", "lua", "restapi"]` to
`app/src-tauri/Cargo.toml`'s `[features]` — a single guessable name for the
combination `just serve` actually needs (no `repl`, and deliberately no
`tokio-console`). `just serve` now runs
`cargo run --release --no-default-features --features viewer -- serve
--web-root app/dist`. `CLAUDE.md`'s build section had it backwards —
"Production build: `cargo build --release --all-features`" was the exact
command that drags `tokio-console` in; default features
(`restapi, repl, cli, lua`) already cover a plain `cargo build --release`
with no flag, so that's now the documented production build, with an
explanatory paragraph on why `--all-features` is the wrong reach and where
`viewer` fits.

## One-line test summary

Rust: `cargo test --workspace` — all suites pass (per-crate totals listed in
the run output; the one Lua-harness "RUN FINISHED state=crashed" line seen
mid-run is an intentional fixture case, not a failure). Frontend:
48 files / 783 tests pass via `pnpm run test:coverage`, coverage
96.66%/92.7%/96.87% stmts/branch/lines (gate: 90/90/80), including the two
new `runsStore.spec.ts` cases for defect 1.
