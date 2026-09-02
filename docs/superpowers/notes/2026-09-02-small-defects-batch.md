# Four small defects, batched — 2026-09-02

Four independently-diagnosed defects, each handed over already understood and
each left unfixed by whoever found it because it sat outside their boundary.
Three are fixed here, one commit each. The fourth is real, understood, and
**not touched**, because its only site is outside this agent's file boundary
too — the reason it keeps getting deferred is structural, not accidental.

## Defect 1 — the supervisor printed a SUM under the label `failed=`

**Real.** `scripts/supervisor.lua` set the transition's `failed` to
`obs.failed + obs.lost + obs.walks_failed + obs.walks_lost` and returned
`obs.lost` separately as `lost`, so `scripts/research_run.lua` printed
`failed=1 lost=1` for a run in which **one** action was lost and nothing else
went wrong. Two numbers, one event, counted twice. It misdirected a live
diagnosis on the day it was found: the reader dispatched work on the belief
that there were two separate problems.

Commit: `db5cc0bd`.

**What changed vs what was renamed.** Nothing about the loop's *decisions*
changed. The sum still exists and still does the one job it was doing — it
decides `any_failures`, which is what separates a `stuck` halt from a
`stuck_silent` one — but it is now a local named `trouble` and never leaves the
function. All four counts leave it separately: `failed`, `lost`,
`walks_failed`, `walks_lost`, each normalised to a number (a `nil` printed as
`"nil"` reads as "unknown" where "none" is the fact).

The two pre-existing tests that pin the walk semantics
(`walks_that_failed_are_failures_and_the_halt_carries_their_error`,
`walks_the_run_lost_track_of_are_failures_too`) **pass untouched**, which is
the evidence that the split preserved behaviour rather than quietly undoing
the reason the sum was introduced. No existing test needed updating: the sum
only ever appeared under the name `failed`, and every existing assertion on
`failed` used a fixture where the sum and the term were both zero. So this was
in the end a rename plus a split, not a semantic change — but the printed line
it produces is a different line.

**Presentation, and why.** Four counts are two axes: *the game judged the
attempt and said no* versus *the game acknowledged it and never answered*, for
**actions** versus for **walks**. A flat list of four `x=n` pairs invites the
original misreading back in a new form, because `failed` and `walks_failed` are
the same distinction named twice. So the line groups by axis:

```
ran: success=1 pending=3 actions(failed=0 lost=1) walks(failed=0 lost=1)
```

Compact, greppable, and the grouping carries the fact that the second pair is
the first pair asked about walks. Collapsing was rejected outright — that is
the defect.

**Test evidence.** Two new tests in `crates/scripting_lua/src/supervisor_lib.rs`
(`the_four_trouble_counts_reach_the_driver_separately`,
`a_failed_walk_is_visible_on_the_transition_that_reported_it`), both verified
**RED first**: `failed` came back as `2` where `0` was required, and
`walks_failed` was absent. Green after the fix; the whole crate is green
(230 tests), including `research_run_lib`'s smoke test, which drives the real
shipped `research_run.lua` against a stub.

**Not verified.** The printed line itself has no test. `research_run_lib.rs`
executes `research_run.lua` but captures `record.*` calls, not `print` output,
so the format string is exercised but not asserted on. Adding a print sink to
that harness is worth doing and was out of scope here.

**Left alone.** `docs/superpowers/specs/2026-08-31-supervisor-loop-design.md`
line 245 discusses `obs.failed`/`obs.lost` — the observation's own fields,
which did not change — so it is still accurate. `scripts/smelt_run.lua` and
`scripts/exec_smoke.lua` read `obs.failed` directly off an observation and are
unaffected.

## Defect 2 — `XXX on_some_entity_created` — REAL, NOT FIXED, needs a decision

**Real, and worse than reported.** One site:

```
crates/core/src/factorio/world.rs:349
    info!("XXX on_some_entity_created {:?}", &entity);
```

`crates/core/src/lib.rs` has `#[macro_use] pub extern crate paris`, and
`world.rs` imports no `tracing`, so this `info!` is **paris** — it goes to
**stdout**, into the narration stream a user reads while the tool runs, not to
the diagnostics stream on stderr. It is not merely log volume; it is in the
channel reserved for things a person is meant to read. It fires from
`crates/core/src/process/output_parser.rs:334`, once per entity creation
reported by the mod. It was introduced by `4d576b15 "restructed code"`, a
restructuring commit — leftover debugging, as diagnosed.

**Why it is still there.** `crates/core/src/factorio/world.rs` is outside this
agent's file boundary, and the boundary rule says report before editing any
file not on the list. That is the third or fourth time this one line has been
deferred for the same reason.

**The change, ready to apply.** Deleting the line is preferred over demoting
it: the entity goes straight into `entity_graph` on the next statement, so a
full `{:?}` dump of every created entity tells a debugger nothing an
`entity_graph` query would not answer better. If it is kept, it must become

```rust
tracing::debug!("on_some_entity_created {:?}", &entity);
```

with `tracing` named explicitly at the site — never a `#[macro_use]` for
`tracing` in that crate, or a half-converted file compiles cleanly with no way
to tell which logging system a given line called. The string carries no paris
colour markup, so nothing needs stripping.

**No test.** A test asserting a log line is absent would pin the absence of a
diagnostic, which is not a behaviour worth freezing.

## Defect 3 — `roll-seed` could not work, and still cannot

**Real, and the path was the second thing wrong with it.**

Commit: `cda10781`.

`crates/scripting_lua/src/roll_best_seed.rs:75` built `format!("plans/{}.lua",
plan_name)` and canonicalized it. That path resolved against the process CWD;
there is no `plans/` in the repo (`workspace/plans` was a duplicate extraction,
removed as dead in `15d49278`, though a stale copy survives in the local
workspace). So `canonicalize` returned `Err` and the `if !lua_path.exists()`
panic under it was unreachable — dead code, as diagnosed.

**What it actually did.** Investigated before fixing, as asked, and the
subcommand is a stub:

1. It prepared `--parallel` Factorio instances (default **four**, minutes of
   archive extraction each) *before* reaching the broken path.
2. The worker loop — the thing that rolls seeds and scores them — has been
   commented out for a long time. `join_handles` was declared, never pushed to,
   and joined empty.
3. `score_seed`, the fitness function, was **deleted** with the old task-graph
   planner, which is why the loop stayed commented out.

So `roll_seed` could only ever return `Ok(None)`, and the CLI could only ever
print "no seed found". Making the path resolve would have bought: the same four
instance setups, a file read whose contents nothing consumes, and then "no seed
found" — a loud failure traded for a silent one. That is precisely the
"polishing a path into dead code" the brief warned against, so it was not done.

**What was done instead — gated, not removed.** The subcommand refuses
immediately, ahead of settings loading and ahead of any instance preparation,
with a message naming why; `--help` shows `UNIMPLEMENTED:`; and the 184 dead
lines behind it (`roll_best_seed.rs`, including ~100 lines of commented-out
loop) are deleted. The clap surface is kept deliberately — it is the interface
any resurrection would implement — and the design record moved into the
module comment on `app/src-tauri/src/cli/roll_seed.rs`: resurrecting it needs a
fitness function for the current planner (`Schedule::makespan` is the plausible
candidate) plus plumbing to read a `Schedule` back out of the handle-based Lua
runtime. Design work, not a path fix.

**Gated rather than removed for a concrete reason.** `README.md:57` lists
`- [x] Seed rolling (factorio-bot roll-seed --map ...)` as a done feature, and
`docs/devguide/architecture.md:58` names `roll-seed` among the CLI's
subcommands. Both are outside this agent's file boundary. Removing the
subcommand would have left two documents describing a command that no longer
exists; gating leaves them describing a command that exists and says what it
is. **`README.md`'s checked box is still wrong and should be corrected by
whoever owns it.**

**Test evidence.** No automated test. The pre-fix behaviour cannot be
exercised in one: reaching the broken path requires `setup_factorio_instance`
to extract Factorio archives into `workspace/`, which a test must not do and
which this agent was told not to touch. Verified statically (no `plans/` in the
repo; `join_handles` never pushed) and end-to-end after the fix:
`cargo run --no-default-features --features cli,lua -- roll-seed --map dummy
--name nonexistent` returns the refusal immediately, and `ls -la workspace/`
is byte-identical before and after.

## Defect 4 — `ActionSettled`'s doc comment was omitted on purpose

**Real.** The account of settle semantics in `crates/core/src/record/mod.rs`
was a `//` block opening with a paragraph explaining its own omission: utoipa
copies `///` into the OpenAPI schema and `app/src/api/openapi.snapshot.json`
pins it, so documenting the variant cost a regeneration in a tree the author
was not editing.

Commit: `11a00089`. Promoted to `///`, the self-explaining paragraph removed
(it became false the moment the block was promoted), snapshot regenerated.

**Test evidence.** The seam fired from the Rust end **RED first**: before
regenerating, `the_committed_openapi_snapshot_matches_the_published_spec`
failed with the full published-vs-snapshot diff. After
`UPDATE_OPENAPI_SNAPSHOT=1`, all 10 tests in that file pass and the snapshot
diff is exactly one line.

**Mirror: checked, not assumed.** `app/src/api/openapi.contract.spec.ts` passes
**untouched** (241 tests). The contract ties published schema *shape* to
declarations in `app/src/api/types.ts`, and a description carries no shape, so
no mirror was needed — confirmed by running it rather than reasoned about.

## Gates

`cargo fmt --check`, `cargo clippy --workspace --all-features --all-targets
--deny warnings`, `cargo test --workspace`, and the frontend's `pnpm lint` and
`pnpm run test:coverage` were run under `nix develop -c`. Results are in the
handover report.

Two pre-existing warnings appear under the narrower `--no-default-features
--features cli,lua` selection used while working on defect 3 — dead `Context`
fields and a needless `return` in `app/src-tauri/src/lib.rs:111`. Both are
artefacts of that feature selection (the fields are read when `restapi` is on),
both predate this work, and neither appears under the workspace gate.
