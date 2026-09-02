# 2026-09-02 — samples.jsonl survives an unfinished run

**Status:** done. Commit `da46eeb4` on `master` (the branch was `master` at
commit time, not `feat/axum-server` — the working tree had moved on since the
task was framed).

## Spec vs code

`docs/superpowers/specs/2026-09-01-run-record-enrichment-design.md` §3.1 says
ingestion "runs at the same two moments `archive_frames` runs — milestone
boundaries and `finish`". It did not: `ingest_samples`/`archive_frames` had
exactly one call site each, both inside `RunRecorder::finish`
(`crates/core/src/record/mod.rs`). Confirmed by grepping every reference to
both functions across the workspace before touching anything. **The
milestone-boundary half was never implemented** — the spec described an
intention the code did not follow, not a regression.

## Approach taken

Ingest at milestone boundaries as well as `finish`, using the existing
milestone-boundary hook: `record.keyframe()` (`crates/scripting_lua/src/globals/record.rs`),
which the Lua supervisor already calls from `Sup:_close` on every milestone
close (`scripts/supervisor.lua`). This was the design's own original plan and
the lowest-cost place to hang it — no new call site to wire into the
supervisor loop, and it reuses the same "no recording running" /
never-fails-the-loop return-value discipline `record.keyframe()` already has.

Chose this over incremental-on-every-record() (too fine-grained, cost) and
over external salvage-after-death (the mod truncates
`script-output/botbridge/samples.jsonl` on the *next* run's frame-capture
start, not on this run's death, so the file does survive an abnormal end
long enough to be salvaged — but only until someone starts another run, and a
salvage pass would still have to reinvent the run-id/tick filtering
`ingest_samples` already does).

Trade-off accepted: a run that dies before its *first* milestone closes still
loses its samples. Everything from the first closed milestone onward
survives.

## Avoiding duplicates

`ingest_samples_incremental` (`crates/core/src/record/samples.rs`) takes a
byte offset and returns the new one. `RunRecorder` persists it
(`samples_offset`) across calls, alongside a running `samples_count` used for
the manifest. Each call:

1. Seeks to the remembered offset (resets to 0 defensively if the source is
   now shorter than that — the mod only truncates for a *new* run, so this
   should never happen while this run is live, but the run-id filter still
   protects against leakage if it somehow did).
2. Reads to EOF, then only considers bytes up to the last `\n` — an
   in-progress final line is left for the next call rather than treated as
   complete, so a read racing the mod's write can't skip half a sample.
3. Parses and validates schema for the whole new chunk *before* writing
   anything (a schema failure aborts with nothing appended and the offset
   left untouched, so retrying doesn't re-admit lines a partial success
   already wrote).
4. Appends (not rewrites) matching lines to the run's `samples.jsonl`.

The run-id/tick exclusion filter is untouched. Two new tests cover this
directly: `repeated_ingestion_from_the_returned_offset_does_not_duplicate_lines`
and `ingesting_samples_twice_before_finish_does_not_duplicate_lines`.

## Cost on a long run

Bounded by what's new since the last call, not the whole file: no
re-reading or re-serialising of already-archived samples at every milestone,
which is what a naive "call the old full-file `ingest_samples` at every
milestone" would have cost on a long run (spec's own sizing table: ~1,667 bot
samples / ~700 KB by the end of a 28-minute run). Each call now costs
roughly one file open, seek, and a read of only the bytes appended since the
last boundary.

## Test summary

`cargo test --workspace`: 44 binaries, 0 failures (one added run:
`a_recorder_killed_before_finish_still_has_samples_on_disk` proves the core
gap is closed; `ingesting_samples_twice_before_finish_does_not_duplicate_lines`
and the two `samples.rs` dedup/partial-line tests prove no duplication).
`cargo fmt` and `cargo clippy --workspace --all-features --all-targets --
--deny warnings` both clean.

## Files touched

- `crates/core/src/record/samples.rs` — new `ingest_samples_incremental` +
  `IngestProgress`, replacing the old full-file `ingest_samples`.
- `crates/core/src/record/mod.rs` — `RunRecorder` gains `samples_offset` /
  `samples_count` and an `ingest_samples` method; `finish()` now calls it
  instead of the old free function.
- `crates/scripting_lua/src/globals/record.rs` — `record.keyframe()` now also
  ingests samples (even on the "nothing placed yet" early-return path, since
  sample ingestion doesn't depend on placements); `record.finish()` reuses
  the same `workspace` path.
