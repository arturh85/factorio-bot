# Run Anatomy Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the run page the record it was designed for: provenance, manifest coverage, a persisted replay, savepoints and tick/kind slices over HTTP, mirrored through the OpenAPI seam, with the provenance chips and replay counts rendered and the two parked clock items closed.

**Architecture:** Five additive routes in `crates/server/src/runs.rs`, each reading a file the recorder already writes (`provenance.json`, `manifest.json`, `savepoints/*.json`) or one new file (`replay.json`, written by the Lua runtime's `LiveRecord` at run end). Every new type crosses the snapshot seam: `utoipa::ToSchema` on the Rust type, snapshot regenerated, `types.ts` mirrored, `objectContract<T>` declared, `pnpm lint` green. The frontend store fetches the new streams as independent enrichments and the headline chips read them; `CursorBar` and `LaneBand` move onto the analysis clock.

**Tech Stack:** Rust (axum, utoipa, serde), Vue 3 + Pinia + vitest, the OpenAPI snapshot seam (`crates/server/tests/openapi.rs` ↔ `app/src/api/openapi.snapshot.json` ↔ `app/src/api/openapi.contract.spec.ts`).

**Spec:** `docs/superpowers/specs/2026-09-08-run-anatomy-design.md` — §2.2 Phase 2 routes, §1.1 headline chips, §4 error handling, §6 Phase 2 row. Phase 1's note (`docs/superpowers/notes/2026-09-08-run-anatomy-phase-1.md`) lists the parked items this plan closes.

## Global Constraints

- **Every cargo command runs under `nix develop -c`** from the repo root, e.g. `nix develop -c cargo test -p factorio-bot-server --features lua`. Never `cargo fmt --all`; format one file with `rustfmt --edition 2024 <file>`.
- **Do not run cargo or pnpm while another session's measured Factorio run is live on this box.** The controller gates each task on the peer session's clearance; an implementer told to wait reports and waits.
- **`crates/core/tests/` and `crates/server/tests/suite.rs` are consolidated targets.** New Rust unit tests go in `#[cfg(test)] mod tests` inside the source file. Server route tests go in the existing `crates/server/tests/runs.rs` (already in `suite.rs`); do not add files under `tests/`.
- **The seam, in this order, for every wire change:** add `utoipa::ToSchema` → `UPDATE_OPENAPI_SNAPSHOT=1 nix develop -c cargo test -p factorio-bot-server --features lua --test openapi` → copy nothing by hand (the test writes `app/src/api/openapi.snapshot.json`) → mirror in `app/src/api/types.ts` → declare in `app/src/api/openapi.contract.spec.ts` → `cd app && pnpm lint && pnpm vitest run src/api`. `pnpm lint` is part of the seam (vitest does not type-check).
- Commit with explicit paths only (`git add -- <paths>`; `git commit -F <msgfile> -- <paths>`), messages via a quoted heredoc file. Never `git add -A`, `--amend`, `git stash`, `git checkout -- <file>`. Before every commit run `git rev-parse --show-toplevel`; it must print the worktree path.
- Absent is not zero: a missing `provenance.json` is a 404 the client renders as "not captured"; `samples_lag_ticks` is `null` for a run with no samples; a replay that was never written is a 404, not an empty document.
- Never join anything on action `id` across a `plan_created`. Replay steps are joined to nothing in this phase; they are counted.
- Colours from theme tokens only; component specs start with `// @vitest-environment jsdom`; new Vue components use multi-word names.
- Coverage gate (90/80/90/90 over `src/api`, `src/store`, `src/lib`, `src/composables`, `src/components/ui`) stays enforced.

---

## File structure

**Rust — modify**
- `crates/core/src/record/provenance.rs` — `ToSchema` on `Provenance`, `GitProvenance`.
- `crates/core/src/graph/entity_graph.rs` — `ToSchema` on `ResourceFingerprint` (derive line only).
- `crates/core/src/record/savepoint.rs` — `ToSchema` on `Savepoint`, `ModFingerprint`.
- `crates/server/src/runs.rs` — `RunSummary` fields; routes `provenance`, `replay`, `savepoints`; query slices on `samples` and `map`.
- `crates/server/tests/runs.rs` — tests for all of the above.
- `crates/scripting_lua/src/globals/record.rs` — `LiveRecord::write_replay`.
- `crates/scripting_lua/src/globals/goal/run.rs` — `emit_replay` also persists.

**Frontend — modify**
- `app/src/api/openapi.snapshot.json` (generated), `app/src/api/types.ts`, `app/src/api/openapi.contract.spec.ts`, `app/src/api/client.ts`.
- `app/src/store/runsStore.ts` (+ spec) — `provenance`, `replay`, `savepoints` enrichments.
- `app/src/components/run/RunHeadline.vue` (+ spec) — real chips.
- `app/src/components/run/CursorBar.vue`, `LaneBand.vue` (+ specs) — the analysis clock.
- `app/src/pages/RunPage.vue` (+ spec) — wiring, replay counts, savepoint command.
- `docs/superpowers/notes/2026-09-09-run-anatomy-phase-2.md` — new.

**Deliberately not in this phase (ruled):** folding `RunAnalysisPage.vue` into the run page (D3) needs replay steps joined to lanes, which has no safe key today (`bot_step_index` is schedule order, lanes are dispatch order); the persisted replay is the prerequisite and lands here, the fold-in is Phase 3 work. Client-side use of the sample slices is not needed while every band reads every kind; the slices land for large archives and tooling.

---

### Task 1: `GET /api/v1/runs/{id}/provenance`

**Files:**
- Modify: `crates/core/src/record/provenance.rs` (derives on `Provenance`, `GitProvenance`), `crates/core/src/graph/entity_graph.rs` (derive on `ResourceFingerprint`), `crates/server/src/runs.rs`
- Test: `crates/server/tests/runs.rs`

**Interfaces:**
- Produces: `GET /api/v1/runs/{id}/provenance` → `200 Provenance` (the file verbatim) or `404 {"message": "run <id> recorded no provenance"}`. `Provenance` gains `utoipa::ToSchema` (fields as in the struct: `schema, run_id, started_unix, started_tick, seed?, map_exchange_string?, map?: ResourceFingerprint, factorio?, mods?: {string: string}, git?: GitProvenance, profile, roster_requested: u32[], workspace?, resumed_from?, bot_mode?, game_speed?, peaceful?`).

- [ ] **Step 1: Write the failing tests** (append to `crates/server/tests/runs.rs`)

```rust
const PROVENANCE: &str = r#"{"schema":1,"run_id":"alpha","started_unix":1000,"started_tick":3242,
  "seed":"31337","map_exchange_string":null,"map":{"digest":"c161fa3f437221d0","tiles":{"iron-ore":940}},
  "factorio":"2.1.17","mods":{"base":"2.1.17","BotBridge":"0.0.1"},
  "git":{"commit":"492e513a261bde8f4c433ecdd4e749918f9a6160","dirty":false,"source":"working-tree-at-run-start"},
  "profile":"release","roster_requested":[1,2,3,4],"workspace":"/w","resumed_from":null,
  "bot_mode":"clients","game_speed":1.0,"peaceful":null}"#;

#[tokio::test]
async fn provenance_is_served_verbatim() {
    let ws = workspace("prov");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    std::fs::write(ws.join("runs/alpha/provenance.json"), PROVENANCE).unwrap();
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/provenance").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["seed"], "31337");
    assert_eq!(body["git"]["dirty"], false);
    assert_eq!(body["mods"]["BotBridge"], "0.0.1");
    assert_eq!(body["map"]["tiles"]["iron-ore"], 940);
    // Present-and-null survives the round trip: `None` is an answer.
    assert!(body.get("map_exchange_string").is_some());
    assert!(body["map_exchange_string"].is_null());
}

#[tokio::test]
async fn a_run_without_provenance_is_a_404_not_an_empty_object() {
    let ws = workspace("noprov");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/provenance").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["message"].as_str().unwrap().contains("recorded no provenance"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test -p factorio-bot-server --features lua --test suite provenance`
Expected: FAIL — 404 with `no such route` / JSON not-found body for the first test (the route does not exist), so `status == 404 != 200`.

- [ ] **Step 3: Implement**

`crates/core/src/record/provenance.rs`: change the two derive lines to
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Provenance { … }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GitProvenance { … }
```
(`utoipa` is already a dependency of `factorio-bot-core`; `Lane` derives it the same way.) `ResourceFingerprint` in `crates/core/src/graph/entity_graph.rs:73` gets `utoipa::ToSchema` added to its existing derive. If `BTreeMap<String,String>` needs it, `ToSchema` handles maps as `additionalProperties`.

`crates/server/src/runs.rs`: add the import `use factorio_bot_core::record::provenance::{Provenance, read_provenance};` and the handler:

```rust
/// What a run was launched with -- the fields that decide whether two runs
/// may be compared at all.
///
/// A 404, not an empty object, when `provenance.json` is missing: the file is
/// written at run *start* since 2026-09-06, so its absence means an older run
/// and the client must show "not captured", never a default.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/provenance",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id")),
    responses(
        (status = 200, body = Provenance),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_provenance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Provenance>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    read_provenance(&dir)
        .map(Json)
        .ok_or_else(|| ErrorResponse::not_found(format!("run {id} recorded no provenance")))
}
```
and `.routes(routes!(get_run_provenance))` in `router()`. Check `read_provenance` is `pub` and exported from `record/mod.rs` (`pub mod provenance;` exists; if `read_provenance` is not re-exported, import it by its module path as written).

- [ ] **Step 4: Run to verify pass**

Run: `nix develop -c cargo test -p factorio-bot-server --features lua --test suite provenance` — Expected: PASS (2 tests).

- [ ] **Step 5: Regenerate the snapshot and confirm the seam fails from the TS side**

```bash
UPDATE_OPENAPI_SNAPSHOT=1 nix develop -c cargo test -p factorio-bot-server --features lua --test openapi
git diff --no-ext-diff --stat -- app/src/api/openapi.snapshot.json
cd app && pnpm vitest run src/api/openapi.contract.spec.ts; cd ..
```
Expected: the snapshot changes (new path, new schemas `Provenance`, `GitProvenance`, `ResourceFingerprint`), and the contract spec FAILS because the new schemas have no contract — that is the seam working. Task 6 mirrors them; this task commits the Rust side and the snapshot together.

- [ ] **Step 6: Commit**

```bash
rustfmt --edition 2024 crates/server/src/runs.rs crates/core/src/record/provenance.rs
cat > /tmp/msg <<'EOF'
feat(server): `GET /runs/{id}/provenance` serves the file that decides comparability

A 404 for a run recorded before provenance existed, never an empty object:
the client renders "not captured" chips from it.
EOF
git add -- crates/core/src/record/provenance.rs crates/core/src/graph/entity_graph.rs crates/server/src/runs.rs crates/server/tests/runs.rs app/src/api/openapi.snapshot.json
git commit -F /tmp/msg -- crates/core/src/record/provenance.rs crates/core/src/graph/entity_graph.rs crates/server/src/runs.rs crates/server/tests/runs.rs app/src/api/openapi.snapshot.json
```

---

### Task 2: `RunSummary` carries `samples`, `map`, `samples_lag_ticks`

**Files:**
- Modify: `crates/server/src/runs.rs`
- Test: `crates/server/tests/runs.rs`

**Interfaces:**
- Produces: `RunSummary` gains `samples: Option<usize>`, `map: Option<usize>`, `samples_lag_ticks: Option<u64>` — from `Manifest.samples`, `Manifest.map`, `Manifest.samples_lag_ticks`; all `None` for an unfinished run. Present-and-null, like the existing fields.

- [ ] **Step 1: Write the failing test**

```rust
const MANIFEST_WITH_COVERAGE: &str = r#"{"run_id":"alpha","started_unix":1000,"finished_unix":1100,
  "outcome":"done","elapsed_ticks":400,"events":3,"splits":1,"samples":2674,"map":24,"samples_lag_ticks":0}"#;

#[tokio::test]
async fn a_summary_carries_the_manifest_coverage_counts() {
    let ws = workspace("coverage");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST_WITH_COVERAGE), None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["summary"]["samples"], 2674);
    assert_eq!(body["summary"]["map"], 24);
    assert_eq!(body["summary"]["samples_lag_ticks"], 0);
}

#[tokio::test]
async fn an_old_manifest_reports_lag_as_null_and_counts_as_zero() {
    // `samples`/`map` are `#[serde(default)]` on Manifest (0 = no such file),
    // `samples_lag_ticks` is Option (None = no samples). The summary must keep
    // that distinction rather than flatten it.
    let ws = workspace("oldmanifest");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    let (_, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha").await;
    assert_eq!(body["summary"]["samples"], 0);
    assert!(body["summary"]["samples_lag_ticks"].is_null());
}

#[tokio::test]
async fn an_unfinished_run_has_null_coverage() {
    let ws = workspace("unfinished-cov");
    seed_run(&ws, "beta", MILESTONES, None, None);
    let (_, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/beta").await;
    assert!(body["summary"]["samples"].is_null());
    assert!(body["summary"]["map"].is_null());
    assert!(body["summary"]["samples_lag_ticks"].is_null());
}
```

- [ ] **Step 2: Run to verify failure** — `nix develop -c cargo test -p factorio-bot-server --features lua --test suite coverage` — Expected: FAIL (`body["summary"]["samples"]` is `Null`).

- [ ] **Step 3: Implement** in `crates/server/src/runs.rs`

Add to `RunSummary`:
```rust
    /// Archived sample lines. `None` for an unfinished run; `Some(0)` for a
    /// finished run with no samples file.
    pub samples: Option<usize>,
    /// Lines in `map.jsonl`, same convention.
    pub map: Option<usize>,
    /// Ticks of the run's span the samples do NOT cover (see `Manifest`).
    /// `None` when the run has no samples at all or never finished.
    pub samples_lag_ticks: Option<u64>,
```
`unfinished()` sets all three to `None`; `from_manifest()` sets `samples: Some(manifest.samples)`, `map: Some(manifest.map)`, `samples_lag_ticks: manifest.samples_lag_ticks`.

- [ ] **Step 4: Run to verify pass** — same filter — Expected: PASS (3 tests); the whole `suite` still green: `nix develop -c cargo test -p factorio-bot-server --features lua --test suite`.

- [ ] **Step 5: Regenerate the snapshot**

`UPDATE_OPENAPI_SNAPSHOT=1 nix develop -c cargo test -p factorio-bot-server --features lua --test openapi`

- [ ] **Step 6: Commit**

```bash
rustfmt --edition 2024 crates/server/src/runs.rs
cat > /tmp/msg <<'EOF'
feat(server): a run summary says how much of the run its samples cover

`samples`, `map` and `samples_lag_ticks` were in the manifest and dropped at
the API boundary; `samples_lag_ticks` is the caveat on every number a page
shows.
EOF
git add -- crates/server/src/runs.rs crates/server/tests/runs.rs app/src/api/openapi.snapshot.json
git commit -F /tmp/msg -- crates/server/src/runs.rs crates/server/tests/runs.rs app/src/api/openapi.snapshot.json
```

---

### Task 3: Tick and kind slices on `/samples` and `/map`

**Files:**
- Modify: `crates/server/src/runs.rs`
- Test: `crates/server/tests/runs.rs`

**Interfaces:**
- Produces: `GET /runs/{id}/samples?from=&to=&kind=` and `GET /runs/{id}/map?from=&to=` — `from`/`to` are inclusive tick bounds (`Option<u64>`), `kind` is the sample `kind` string (`bots` | `force` | `machines`). Filtering happens after the full read (the reader is line-based; a seekable index is not in scope), so the win is payload size, not disk time. `skipped` is unaffected by filters.

- [ ] **Step 1: Write the failing tests**

```rust
const SAMPLES: &str = concat!(
    r#"{"schema":3,"tick":600,"run":"alpha","kind":"bots","bots":[]}"#, "\n",
    r#"{"schema":3,"tick":600,"run":"alpha","kind":"force","research":null,"techs_unlocked":0,"production":{"made":{},"consumed":{}},"power":{"generated_kw":0.0,"consumed_kw":0.0,"satisfaction":1.0,"networks":{}},"pollution":null}"#, "\n",
    r#"{"schema":3,"tick":900,"run":"alpha","kind":"machines","machines":{},"truncated":0}"#, "\n",
    r#"{"schema":3,"tick":1200,"run":"alpha","kind":"force","research":null,"techs_unlocked":0,"production":{"made":{},"consumed":{}},"power":{"generated_kw":0.0,"consumed_kw":0.0,"satisfaction":1.0,"networks":{}},"pollution":null}"#, "\n",
);

#[tokio::test]
async fn samples_can_be_sliced_by_tick_and_kind() {
    let ws = workspace("slice");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    std::fs::write(ws.join("runs/alpha/samples.jsonl"), SAMPLES).unwrap();
    let st = || state_with_workspace(&ws);
    let (_, all) = get_json(st(), "/api/v1/runs/alpha/samples").await;
    assert_eq!(all["samples"].as_array().unwrap().len(), 4);
    let (_, window) = get_json(st(), "/api/v1/runs/alpha/samples?from=700&to=1200").await;
    let ticks: Vec<u64> = window["samples"].as_array().unwrap().iter().map(|s| s["tick"].as_u64().unwrap()).collect();
    assert_eq!(ticks, vec![900, 1200], "inclusive bounds");
    let (_, force) = get_json(st(), "/api/v1/runs/alpha/samples?kind=force").await;
    assert_eq!(force["samples"].as_array().unwrap().len(), 2);
    assert!(force["samples"].as_array().unwrap().iter().all(|s| s["kind"] == "force"));
    let (_, both) = get_json(st(), "/api/v1/runs/alpha/samples?kind=force&to=600").await;
    assert_eq!(both["samples"].as_array().unwrap().len(), 1);
}

const MAP_LINES: &str = concat!(
    r#"{"tick":10,"kind":"keyframe","bounds":{"left":0,"top":0,"right":1,"bottom":1},"game":[],"model":[],"divergence":[]}"#, "\n",
    r#"{"tick":500,"kind":"placed","bot":1,"intent":{"name":"stone-furnace","position":{"x":1,"y":2},"direction":0},"actual":{"name":"stone-furnace","position":{"x":1,"y":2},"direction":0},"drift":null}"#, "\n",
    r#"{"tick":900,"kind":"placed","bot":2,"intent":{"name":"wooden-chest","position":{"x":3,"y":2},"direction":0},"actual":{"name":"wooden-chest","position":{"x":3,"y":2},"direction":0},"drift":null}"#, "\n",
);

#[tokio::test]
async fn the_map_can_be_sliced_by_tick() {
    let ws = workspace("mapslice");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    std::fs::write(ws.join("runs/alpha/map.jsonl"), MAP_LINES).unwrap();
    let (_, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/map?from=100&to=600").await;
    let ticks: Vec<u64> = body["map"].as_array().unwrap().iter().map(|r| r["tick"].as_u64().unwrap()).collect();
    assert_eq!(ticks, vec![500]);
}
```

- [ ] **Step 2: Run to verify failure** — `nix develop -c cargo test -p factorio-bot-server --features lua --test suite slice` — Expected: FAIL (unknown query params are ignored, so the sliced call returns all 4 / all 3).

- [ ] **Step 3: Implement**

```rust
/// Narrows a stream to a tick window. Both bounds inclusive; either may be
/// absent. The whole file is still read -- the record is line-oriented and
/// has no index -- so this saves the wire and the browser, not the disk.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct TickWindow {
    /// First tick to include.
    pub from: Option<u64>,
    /// Last tick to include.
    pub to: Option<u64>,
}

impl TickWindow {
    fn contains(&self, tick: u64) -> bool {
        self.from.is_none_or(|f| tick >= f) && self.to.is_none_or(|t| tick <= t)
    }
}

/// `TickWindow` plus the sample `kind` (`bots`, `force`, `machines`).
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct SampleFilter {
    pub from: Option<u64>,
    pub to: Option<u64>,
    /// Return only samples of this `kind`.
    pub kind: Option<String>,
}
```
In `get_run_samples`: add `Query(filter): Query<SampleFilter>`, `params(("id" = String, Path, description = "the run id"), SampleFilter)`, and after the read:
```rust
    let window = TickWindow { from: filter.from, to: filter.to };
    let samples = read
        .samples
        .into_iter()
        .filter(|s| window.contains(s.tick))
        .filter(|s| filter.kind.as_deref().is_none_or(|k| sample_kind(s) == k))
        .collect();
```
with
```rust
/// The wire name of a sample's kind, without a second serialisation: the
/// enum's `#[serde(tag = "kind")]` is the contract, and matching the variants
/// here keeps this in step with it at compile time.
fn sample_kind(sample: &Sample) -> &'static str {
    match sample.kind {
        SampleKind::Bots { .. } => "bots",
        SampleKind::Force { .. } => "force",
        SampleKind::Machines { .. } => "machines",
        SampleKind::Unknown => "unknown",
    }
}
```
(import `SampleKind` from `factorio_bot_core::record`; if the variant list differs, match what the enum has and let the compiler tell you). In `get_run_map`: add `Query(window): Query<TickWindow>`, `params(... , TickWindow)`, and `.filter(|r| window.contains(r.tick))` over `read.records`. `is_none_or` needs Rust 1.82+; the workspace edition is 2024, so it is available.

- [ ] **Step 4: Run to verify pass** — same filter — Expected: PASS (2 tests). Also add both operations to `QUERY_OPERATIONS` in `crates/server/tests/openapi.rs`: `("/api/v1/runs/{id}/samples", "get", &[("from", false), ("to", false), ("kind", false)])` and `("/api/v1/runs/{id}/map", "get", &[("from", false), ("to", false)])`; run `nix develop -c cargo test -p factorio-bot-server --features lua --test openapi` and expect it to pass after the snapshot regenerates in the next step.

- [ ] **Step 5: Regenerate the snapshot** — `UPDATE_OPENAPI_SNAPSHOT=1 nix develop -c cargo test -p factorio-bot-server --features lua --test openapi`.

- [ ] **Step 6: Commit**

```bash
rustfmt --edition 2024 crates/server/src/runs.rs
cat > /tmp/msg <<'EOF'
feat(server): `/samples` and `/map` take a tick window, `/samples` a kind

The reader is line-oriented so the disk still pays for the whole file; the
wire and the browser do not. A 25-minute run's machines-only slice is a
fraction of its bots samples.
EOF
git add -- crates/server/src/runs.rs crates/server/tests/runs.rs crates/server/tests/openapi.rs app/src/api/openapi.snapshot.json
git commit -F /tmp/msg -- crates/server/src/runs.rs crates/server/tests/runs.rs crates/server/tests/openapi.rs app/src/api/openapi.snapshot.json
```

---

### Task 4: The replay is persisted and served

**Files:**
- Modify: `crates/scripting_lua/src/globals/record.rs` (`LiveRecord::write_replay`), `crates/scripting_lua/src/globals/goal/run.rs` (`emit_replay` persists), `crates/server/src/runs.rs` (route)
- Test: `crates/scripting_lua/src/globals/goal/run.rs` `#[cfg(test)]` (uses the existing `live_record()` helper), `crates/server/tests/runs.rs`

**Interfaces:**
- Produces: `LiveRecord::write_replay(&self, json: &str) -> bool` writes `<run dir>/replay.json`, returns `false` when no recording is running; `emit_replay(sink, sched, log, refused, live: Option<&LiveRecord>)`; `GET /runs/{id}/replay` → `200` with the document as `serde_json::Value` (schema `object`, the TS side already narrows with `parseReplay`) or `404 "run <id> has no replay"`. `pub const REPLAY_FILE: &str = "replay.json"` in `crates/core/src/record/mod.rs`.

- [ ] **Step 1: Write the failing tests**

In `run.rs`'s tests module, next to the existing replay tests (look at how `live_record()` returns `(LiveRecord, TempDir, PathBuf)` and how a run is started with a `live: Some(...)` in the heartbeat tests around line 2870):

```rust
    #[tokio::test]
    async fn the_replay_is_written_beside_the_events() {
        let (live, _tmp, run_dir) = live_record();
        // Reuse the smallest run fixture the neighbouring replay tests build
        // (a one-step schedule against the stub actuator); pass `Some(live)`
        // as the run's `live` argument exactly as the heartbeat tests do.
        let sink = RecordingSink::default();
        run_one_step_with(Some(Arc::new(sink.clone())), Some(live)).await;
        let on_disk = std::fs::read_to_string(run_dir.join("replay.json")).expect("replay.json written");
        let on_wire = only_replay(&sink);
        let parsed: Value = serde_json::from_str(&on_disk).unwrap();
        assert_eq!(parsed, on_wire, "the file is the document the sink received, byte for byte in meaning");
    }
```
If no `run_one_step_with(sink, live)` helper exists, add one beside `live_record()` that wraps whatever the neighbouring replay test calls (read those tests; the helper's body is their setup lines with `sink` and `live` as parameters). A run without a recorder must keep writing nothing: `a_run_with_no_recorder_runs_exactly_as_before` already exists and must stay green.

Server test:
```rust
#[tokio::test]
async fn a_persisted_replay_is_served_and_its_absence_is_a_404() {
    let ws = workspace("replay");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    let (status, _) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/replay").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    std::fs::write(ws.join("runs/alpha/replay.json"),
        r#"{"planned_makespan":100,"refused":null,"steps":[],"unmatched_walks":[]}"#).unwrap();
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/replay").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["planned_makespan"], 100);
}
```

- [ ] **Step 2: Run to verify failure** — `nix develop -c cargo test -p factorio-bot-scripting-lua replay_is_written` and `nix develop -c cargo test -p factorio-bot-server --features lua --test suite replay` — Expected: FAIL (no file; 404 on both calls).

- [ ] **Step 3: Implement**

`crates/core/src/record/mod.rs`: `pub const REPLAY_FILE: &str = "replay.json";` next to the other file-name constants.

`record.rs` (`impl LiveRecord`):
```rust
    /// Writes the run's replay document beside its events, overwriting an
    /// earlier one: a script that runs several `goal.run`s keeps the last,
    /// which is the one whose batch closed the run. Returns whether it was
    /// written -- `false` means no recording is running.
    pub fn write_replay(&self, json: &str) -> bool {
        let guard = self.slot.lock();
        let Some(recorder) = guard.as_ref() else {
            return false;
        };
        let path = recorder.dir().join(factorio_bot_core::record::REPLAY_FILE);
        if let Err(err) = std::fs::write(&path, json) {
            factorio_bot_core::tracing::error!(error = %err, path = %path.display(), "the replay could not be written");
            return false;
        }
        true
    }
```
Add `#[derive(Clone)]` to `LiveRecord` if it is not already `Clone` (both fields are `Arc`s).

`run.rs`: `emit_replay` gains `live: Option<&LiveRecord>`; after `Ok(json) => sink.replay(&json)`, add `if let Some(live) = live { live.write_replay(&json); }` — the sink call stays first so a disk failure cannot delay the stream. At the call site, clone `live` before it is moved into the heartbeat (`let replay_live = live.clone();`) and pass `replay_live.as_ref()`.

`runs.rs`:
```rust
/// The executor's replay -- planned against observed per step, with evidence.
///
/// Written by the run at its end since Phase 2 of Run Anatomy; a run archived
/// before that, or one that never ran a plan, has none, and that is a 404
/// rather than an empty document: an empty replay would read as a run that
/// planned nothing.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/replay",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id")),
    responses(
        (status = 200, body = serde_json::Value, description = "the replay document, as the executor serialised it"),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_replay(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    let bytes = std::fs::read(dir.join(factorio_bot_core::record::REPLAY_FILE))
        .map_err(|_| ErrorResponse::not_found(format!("run {id} has no replay")))?;
    serde_json::from_slice(&bytes)
        .map(Json)
        .map_err(|err| ErrorResponse::internal(format!("replay.json is not valid JSON: {err}")))
}
```
plus `.routes(routes!(get_run_replay))`.

- [ ] **Step 4: Run to verify pass** — both commands from Step 2, then `nix develop -c cargo test -p factorio-bot-scripting-lua` in full — Expected: PASS.

- [ ] **Step 5: Regenerate the snapshot** — `UPDATE_OPENAPI_SNAPSHOT=1 nix develop -c cargo test -p factorio-bot-server --features lua --test openapi`.

- [ ] **Step 6: Commit**

```bash
rustfmt --edition 2024 crates/scripting_lua/src/globals/record.rs crates/scripting_lua/src/globals/goal/run.rs crates/server/src/runs.rs
cat > /tmp/msg <<'EOF'
feat(record): the replay is written beside the events and served per run

Until now it lived only on the job's SSE stream and in a job record capped
at fifty, so the evidence for every run older than that was gone.
EOF
git add -- crates/core/src/record/mod.rs crates/scripting_lua/src/globals/record.rs crates/scripting_lua/src/globals/goal/run.rs crates/server/src/runs.rs crates/server/tests/runs.rs app/src/api/openapi.snapshot.json
git commit -F /tmp/msg -- crates/core/src/record/mod.rs crates/scripting_lua/src/globals/record.rs crates/scripting_lua/src/globals/goal/run.rs crates/server/src/runs.rs crates/server/tests/runs.rs app/src/api/openapi.snapshot.json
```

---

### Task 5: `GET /api/v1/runs/{id}/savepoints`

**Files:**
- Modify: `crates/core/src/record/savepoint.rs` (derives), `crates/server/src/runs.rs`
- Test: `crates/server/tests/runs.rs`

**Interfaces:**
- Produces: `RunSavepointsResponse { savepoints: Vec<Savepoint> }`, ascending by `milestone_index`; empty list (200) when the directory is absent. `Savepoint` and `ModFingerprint` gain `utoipa::ToSchema`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn savepoints_are_listed_in_milestone_order_and_absent_is_empty() {
    let ws = workspace("savepoints");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/savepoints").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["savepoints"].as_array().unwrap().len(), 0);
    let dir = ws.join("runs/alpha/savepoints");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("milestone-2.json"), r#"{"schema":1,"run_id":"alpha","milestone_index":2,"tick":900,"created_unix":1050,"bytes":10,"file":"milestone-2.zip","mods":null}"#).unwrap();
    std::fs::write(dir.join("milestone-1.json"), r#"{"schema":1,"run_id":"alpha","milestone_index":1,"tick":300,"created_unix":1020,"bytes":10,"file":"milestone-1.zip","mods":{"version":"0.0.1","digest":"f3200cfb","files":6}}"#).unwrap();
    let (_, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/savepoints").await;
    let idx: Vec<u64> = body["savepoints"].as_array().unwrap().iter().map(|s| s["milestone_index"].as_u64().unwrap()).collect();
    assert_eq!(idx, vec![1, 2]);
    assert_eq!(body["savepoints"][0]["mods"]["digest"], "f3200cfb");
}
```

- [ ] **Step 2: Run to verify failure** — `nix develop -c cargo test -p factorio-bot-server --features lua --test suite savepoints_are_listed` — Expected: FAIL (404 route).

- [ ] **Step 3: Implement**

`savepoint.rs`: add `utoipa::ToSchema` to the derives of `ModFingerprint` (line ~169) and `Savepoint` (line ~341).

`runs.rs`:
```rust
/// `GET /api/v1/runs/{id}/savepoints` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RunSavepointsResponse {
    /// Ascending by milestone index. Each names its `.zip` relative to the
    /// run's `savepoints/` directory; `--resume-from <run>:<index>` is the
    /// command that uses one.
    pub savepoints: Vec<Savepoint>,
}

/// The milestone savepoints a run wrote.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/savepoints",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id")),
    responses(
        (status = 200, body = RunSavepointsResponse),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_savepoints(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunSavepointsResponse>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?.join(SAVEPOINTS_DIR);
    let mut savepoints: Vec<Savepoint> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| std::fs::read(e.path()).ok())
                .filter_map(|bytes| serde_json::from_slice::<Savepoint>(&bytes).ok())
                .collect()
        })
        .unwrap_or_default();
    savepoints.sort_by_key(|s| s.milestone_index);
    Ok(Json(RunSavepointsResponse { savepoints }))
}
```
Import `factorio_bot_core::record::savepoint::{SAVEPOINTS_DIR, Savepoint}`; register the route.

- [ ] **Step 4: Run to verify pass** — Expected: PASS. Then the whole server suite once: `nix develop -c cargo test -p factorio-bot-server --features lua`.

- [ ] **Step 5: Regenerate the snapshot** — `UPDATE_OPENAPI_SNAPSHOT=1 nix develop -c cargo test -p factorio-bot-server --features lua --test openapi`. Then clippy for the crates touched in Tasks 1–5: `nix develop -c cargo clippy -p factorio-bot-core -p factorio-bot-server -p factorio-bot-scripting-lua --all-features --all-targets -- --deny warnings`.

- [ ] **Step 6: Commit**

```bash
rustfmt --edition 2024 crates/server/src/runs.rs crates/core/src/record/savepoint.rs
cat > /tmp/msg <<'EOF'
feat(server): `GET /runs/{id}/savepoints` lists what `--resume-from` can start from
EOF
git add -- crates/core/src/record/savepoint.rs crates/server/src/runs.rs crates/server/tests/runs.rs app/src/api/openapi.snapshot.json
git commit -F /tmp/msg -- crates/core/src/record/savepoint.rs crates/server/src/runs.rs crates/server/tests/runs.rs app/src/api/openapi.snapshot.json
```

---

### Task 6: Mirror the seam in TypeScript

**Files:**
- Modify: `app/src/api/types.ts`, `app/src/api/openapi.contract.spec.ts`, `app/src/api/client.ts`

**Interfaces:**
- Produces in `types.ts`:
```ts
export interface ResourceFingerprint { digest: string; tiles: Record<string, number> }
export interface GitProvenance { commit: string; dirty: boolean; source: string }
export interface Provenance {
    schema: number; run_id: string; started_unix: number; started_tick: number;
    seed: string | null; map_exchange_string: string | null; map: ResourceFingerprint | null;
    factorio: string | null; mods: Record<string, string> | null; git: GitProvenance | null;
    profile: string; roster_requested: number[]; workspace: string | null; resumed_from: string | null;
    bot_mode: string | null; game_speed: number | null; peaceful: boolean | null;
}
export interface ModFingerprint { version: string | null; digest: string; files: number }
export interface Savepoint { schema: number; run_id: string; milestone_index: number; tick: number; created_unix: number; bytes: number; file: string; mods: ModFingerprint | null }
export interface RunSavepointsResponse { savepoints: Savepoint[] }
```
  and `RunSummary` gains `samples: number | null; map: number | null; samples_lag_ticks: number | null`.
- Produces in `client.ts`: `getRunProvenance(id): Promise<Provenance>`, `getRunReplay(id): Promise<unknown>` (the caller narrows with `parseReplay`), `getRunSavepoints(id): Promise<RunSavepointsResponse>`, and optional `opts` on `getRunSamples(id, opts?: {from?: number; to?: number; kind?: 'bots' | 'force' | 'machines'})` and `getRunMap(id, opts?: {from?: number; to?: number})` passed as `query`.

- [ ] **Step 1: Run the contract spec to see the seam red** — `cd app && pnpm vitest run src/api/openapi.contract.spec.ts` — Expected: FAIL naming the schemas without contracts (`Provenance`, `GitProvenance`, `ResourceFingerprint`, `Savepoint`, `ModFingerprint`, `RunSavepointsResponse`) and the new paths without callers.

- [ ] **Step 2: Mirror**

Add the interfaces above to `types.ts` next to `RunSummary`, with a doc comment on `Provenance` repeating the `None`-means-not-captured rule. Add to `openapi.contract.spec.ts`: the type imports; path entries for `/api/v1/runs/{id}/provenance` (caller `getRunProvenance`, response `Provenance`), `/replay` (caller `getRunReplay`, response schema: whatever the snapshot names for a bare object — read the regenerated snapshot's `responses.200.content` for that path and match it), `/savepoints` (caller `getRunSavepoints`, response `RunSavepointsResponse`); `query: [['from', false], ['to', false], ['kind', false]]` on `/samples` and `[['from', false], ['to', false]]` on `/map`; contracts:
```ts
    Provenance: objectContract<Provenance>({
        schema: {required: true, type: 'integer'},
        run_id: {required: true, type: 'string'},
        started_unix: {required: true, type: 'integer'},
        started_tick: {required: true, type: 'integer'},
        seed: {required: true, type: 'string', nullable: true},
        map_exchange_string: {required: true, type: 'string', nullable: true},
        map: {required: true, ref: 'ResourceFingerprint', nullable: true},
        factorio: {required: true, type: 'string', nullable: true},
        mods: {required: true, type: 'object', nullable: true},
        git: {required: true, ref: 'GitProvenance', nullable: true},
        profile: {required: true, type: 'string'},
        roster_requested: {required: true, arrayOf: 'integer'},
        workspace: {required: true, type: 'string', nullable: true},
        resumed_from: {required: true, type: 'string', nullable: true},
        bot_mode: {required: true, type: 'string', nullable: true},
        game_speed: {required: true, type: 'number', nullable: true},
        peaceful: {required: false, type: 'boolean', nullable: true}
    }),
```
and the corresponding entries for `GitProvenance`, `ResourceFingerprint` (`tiles: {required: true, type: 'object'}`), `ModFingerprint`, `Savepoint`, `RunSavepointsResponse`, and the three new `RunSummary` fields (`required: false, type: 'integer', nullable: true` — match how utoipa emitted `Option` fields for the existing ones). Where the helper's vocabulary (`type`, `ref`, `arrayOf`, `nullable`, `required`) does not express an emitted shape (e.g. `additionalProperties` for maps), read how an existing map field such as `BotSample.inventory` is declared and copy it. Add the three client functions and the `opts` parameters.

- [ ] **Step 3: Verify the seam green from both ends** — `cd app && pnpm lint && pnpm vitest run src/api` — Expected: PASS.

- [ ] **Step 4: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(app): mirror the provenance, replay, savepoints and slice routes through the seam
EOF
git add -- app/src/api/types.ts app/src/api/openapi.contract.spec.ts app/src/api/client.ts
git commit -F /tmp/msg -- app/src/api/types.ts app/src/api/openapi.contract.spec.ts app/src/api/client.ts
```

---

### Task 7: The store fetches provenance, replay and savepoints

**Files:**
- Modify: `app/src/store/runsStore.ts`, `app/src/store/runsStore.spec.ts`

**Interfaces:**
- Produces state `provenance: Provenance | null`, `provenanceError: string | null`, `replay: Replay | null`, `replayError: string | null`, `savepoints: Savepoint[]`; getters `replayCounts(): {steps: number; abandoned: number; lost: number; failed: number; pending: number; believed: number} | null` (null without a replay) and `sampleLag(): number | null` = `detail.summary.samples_lag_ticks` when the summary carries it, else the client-side `lagTicks` (Task 7 of Phase 1) — the manifest's number outranks a derivation. `parseReplay` from `@/api/replay.ts` narrows the replay; a document it rejects sets `replayError`.

- [ ] **Step 1: Write the failing tests** (append to `runsStore.spec.ts`; mock `getRunProvenance`, `getRunReplay`, `getRunSavepoints` in the shared `beforeEach` with `mockRejectedValue(new ApiError(404, 'not_found', null, ''))` by default)

```ts
describe('phase 2 enrichments', () => {
    it('a 404 on provenance is "not captured", not an error for the run', async () => {
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.detail).not.toBeNull();
        expect(store.provenance).toBeNull();
        expect(store.provenanceError).toContain('/provenance');
    });
    it('keeps a served provenance and the replay counts', async () => {
        vi.mocked(client.getRunProvenance).mockResolvedValue({schema: 1, run_id: 'run-1', started_unix: 1, started_tick: 0, seed: '31337', map_exchange_string: null, map: null, factorio: '2.1.17', mods: {base: '2.1.17'}, git: {commit: 'abc', dirty: false, source: 'working-tree-at-run-start'}, profile: 'release', roster_requested: [1, 2], workspace: null, resumed_from: null, bot_mode: 'clients', game_speed: 1, peaceful: null});
        vi.mocked(client.getRunReplay).mockResolvedValue({planned_makespan: 10, refused: null, unmatched_walks: [], steps: [
            {index: 0, bot: 1, bot_step_index: 0, what: {kind: 'act', action: 1, label: 'craft 1 pipe'}, planned_start_tick: 0, planned_end_tick: 5, observed_start_tick: 0, observed_end_tick: 6, status: 'success', attempt_number: 1, evidence: {kind: 'measured'}, error: null},
            {index: 1, bot: 1, bot_step_index: 1, what: {kind: 'walk', to: {x: 1, y: 2}}, planned_start_tick: 5, planned_end_tick: 9, observed_start_tick: 6, observed_end_tick: 9, status: 'success', attempt_number: 1, evidence: {kind: 'believed', why: 'ticks measured, arrival not'}, error: null},
            {index: 2, bot: 1, bot_step_index: 2, what: {kind: 'act', action: 2, label: 'place x'}, planned_start_tick: 9, planned_end_tick: 10, observed_start_tick: null, observed_end_tick: null, status: 'abandoned', attempt_number: 0, evidence: {kind: 'measured'}, error: 'abandoned: predecessor 1 failed'}
        ]});
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.provenance?.seed).toBe('31337');
        expect(store.replayCounts).toEqual({steps: 3, abandoned: 1, lost: 0, failed: 0, pending: 0, believed: 1});
    });
    it('prefers the manifest lag over the derived one', async () => {
        vi.mocked(client.getRun).mockResolvedValue({summary: {...summary('run-1'), samples: 10, map: 2, samples_lag_ticks: 42}, splits: []} as RunDetail);
        const store = useRunsStore();
        await store.openRun('run-1');
        expect(store.sampleLag).toBe(42);
    });
});
```
Adjust the replay literal to whatever `parseReplay` requires (read `app/src/api/replay.ts`; `status` values are lowercase strings in the wire — check `ReplayStatus`, and if `abandoned` is not yet in the union, that is the peer's `9f55d023`/later change already on master: use the value the union has and count it).

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/store/runsStore.spec.ts` — Expected: FAIL (`provenance` undefined). Also update `summary()` in the spec to include `samples: null, map: null, samples_lag_ticks: null` (the type now requires them) — `pnpm lint` will insist.

- [ ] **Step 3: Implement** — state and errors alongside the existing enrichments; three more entries in `Promise.allSettled` (`getRunProvenance(id)`, `getRunReplay(id)`, `getRunSavepoints(id)`) with the `enrichmentUnavailable` pattern (`'provenance', '/provenance'`, `'replay', '/replay'`, `'savepoints', '/savepoints'`); on a fulfilled replay run `parseReplay(value)` and store the result, setting `replayError` to its message when it throws or returns null (read what `parseReplay` does on bad input); getters:

```ts
        replayCounts() {
            if (this.replay === null) return null;
            const s = this.replay.steps;
            const count = (status: string) => s.filter((step) => step.status === status).length;
            return {steps: s.length, abandoned: count('abandoned'), lost: count('lost'), failed: count('failed'), pending: count('pending'),
                believed: s.filter((step) => step.evidence.kind === 'believed').length};
        },
        sampleLag(): number | null {
            const fromManifest = this.detail?.summary.samples_lag_ticks;
            if (fromManifest !== undefined && fromManifest !== null) return fromManifest;
            const win = this.window;
            return win === null ? null : lagTicks(win.hi, this.samples);
        },
```
Reset all new state in `openRun` and in its `catch`.

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/store && pnpm lint` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(app): the runs store fetches provenance, the replay and savepoints as enrichments
EOF
git add -- app/src/store/runsStore.ts app/src/store/runsStore.spec.ts
git commit -F /tmp/msg -- app/src/store/runsStore.ts app/src/store/runsStore.spec.ts
```

---

### Task 8: Real provenance chips, replay counts, savepoint command

**Files:**
- Modify: `app/src/components/run/RunHeadline.vue`, `RunHeadline.spec.ts`, `app/src/pages/RunPage.vue`, `RunPage.spec.ts`

**Interfaces:**
- `RunHeadline` props become `{summary: RunSummary; provenance: Provenance | null; provenanceError: string | null; lagTicks: number | null; headline: string; roster: number[]; replayCounts: {steps; abandoned; lost; failed; pending; believed} | null; savepoints: Savepoint[]}`.
- Chips: `seed` → `provenance.seed ?? 'not captured'`; `mode` → `bot_mode`; `speed` → `${game_speed}×`; `commit` → first 8 chars of `git.commit` plus ` dirty` when `git.dirty`; `profile`; `mods` → `${Object.keys(mods).length} mods` with the full `name version` list in `title`; `map` → `map.digest`. Each chip has `data-state="present"` when the value exists, `"absent"` with the text `not captured` when the field is null, and when `provenance === null` every chip is absent and one extra chip `provenance` shows `provenanceError`'s text (`not captured` if a 404). A `plan` chip: `176 steps · 1 abandoned · 0 lost · 1 believed` (omit zero counts except `steps`; `abandoned > 0` gets `data-state="truncated"` and the warn colour — it means the plan's tail never ran). A `resume` chip per savepoint: `--resume-from <run_id>:<milestone_index>` in a `<code>` the user can copy.

- [ ] **Step 1: Write the failing tests** (replace the Phase 1 `RunHeadline.spec.ts` body)

```ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import RunHeadline from './RunHeadline.vue';

const summary = {run_id: 'run-1', finished: true, started_unix: 1, finished_unix: 2, outcome: 'done', elapsed_ticks: 10, events: 1, splits: 1, samples: 5, map: 1, samples_lag_ticks: 0};
const provenance = {schema: 1, run_id: 'run-1', started_unix: 1, started_tick: 0, seed: '31337', map_exchange_string: null, map: {digest: 'c161fa3f437221d0', tiles: {}}, factorio: '2.1.17', mods: {base: '2.1.17', BotBridge: '0.0.1'}, git: {commit: '492e513a261bde8f4c433ecdd4e749918f9a6160', dirty: true, source: 'working-tree-at-run-start'}, profile: 'release', roster_requested: [1, 2, 3, 4], workspace: null, resumed_from: null, bot_mode: 'clients', game_speed: 1, peaceful: null};
const base = {summary, lagTicks: 0, headline: 'rates: …', roster: [1, 2, 3, 4], savepoints: [], replayCounts: null};

describe('RunHeadline', () => {
    it('fills the chips from provenance and marks a dirty commit', () => {
        const w = mount(RunHeadline, {props: {...base, provenance, provenanceError: null}});
        expect(w.get('[data-chip="seed"]').attributes('data-state')).toBe('present');
        expect(w.get('[data-chip="seed"]').text()).toContain('31337');
        expect(w.get('[data-chip="commit"]').text()).toContain('492e513a dirty');
        expect(w.get('[data-chip="mods"]').text()).toContain('2 mods');
        expect(w.get('[data-chip="mods"]').attributes('title')).toContain('BotBridge 0.0.1');
        expect(w.get('[data-chip="map"]').text()).toContain('c161fa3f437221d0');
    });
    it('a null field inside a present provenance is "not captured" for that chip only', () => {
        const w = mount(RunHeadline, {props: {...base, provenance: {...provenance, seed: null}, provenanceError: null}});
        expect(w.get('[data-chip="seed"]').attributes('data-state')).toBe('absent');
        expect(w.get('[data-chip="mode"]').attributes('data-state')).toBe('present');
    });
    it('a missing provenance file marks every chip absent and says why', () => {
        const w = mount(RunHeadline, {props: {...base, provenance: null, provenanceError: 'provenance unavailable — this server does not provide /provenance'}});
        expect(w.get('[data-chip="seed"]').attributes('data-state')).toBe('absent');
        expect(w.get('[data-chip="provenance"]').text()).toContain('does not provide /provenance');
    });
    it('shows the plan counts and flags an abandoned tail', () => {
        const w = mount(RunHeadline, {props: {...base, provenance, provenanceError: null, replayCounts: {steps: 176, abandoned: 48, lost: 0, failed: 1, pending: 0, believed: 52}}});
        const plan = w.get('[data-chip="plan"]');
        expect(plan.text()).toContain('176 steps');
        expect(plan.text()).toContain('48 abandoned');
        expect(plan.text()).not.toContain('0 lost');
        expect(plan.attributes('data-state')).toBe('truncated');
    });
    it('offers one resume command per savepoint', () => {
        const w = mount(RunHeadline, {props: {...base, provenance, provenanceError: null, savepoints: [{schema: 1, run_id: 'run-1', milestone_index: 1, tick: 300, created_unix: 1, bytes: 10, file: 'milestone-1.zip', mods: null}]}});
        expect(w.get('[data-chip="resume"] code').text()).toBe('--resume-from run-1:1');
    });
});
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/components/run/RunHeadline.spec.ts` — Expected: FAIL.

- [ ] **Step 3: Implement `RunHeadline.vue`**

```vue
<script setup lang="ts">
/**
 * The first line, and the chips that say whether this run may be compared
 * with another. A chip is present when the record answers, "not captured"
 * when the field is null, and every chip is absent -- with the reason shown --
 * when there is no provenance at all. Absence is drawn, never defaulted.
 */
import {computed} from 'vue';
import {Provenance, RunSummary, Savepoint} from '@/api/types';

const props = defineProps<{
    summary: RunSummary; provenance: Provenance | null; provenanceError: string | null;
    lagTicks: number | null; headline: string; roster: number[];
    replayCounts: {steps: number; abandoned: number; lost: number; failed: number; pending: number; believed: number} | null;
    savepoints: Savepoint[];
}>();

interface Chip { key: string; value: string | null; title?: string }

const chips = computed<Chip[]>(() => {
    const p = props.provenance;
    if (p === null) return ['seed', 'mode', 'speed', 'commit', 'profile', 'mods', 'map'].map((key) => ({key, value: null}));
    const mods = p.mods === null ? null : Object.entries(p.mods);
    return [
        {key: 'seed', value: p.seed},
        {key: 'mode', value: p.bot_mode},
        {key: 'speed', value: p.game_speed === null ? null : `${p.game_speed}×`},
        {key: 'commit', value: p.git === null ? null : `${p.git.commit.slice(0, 8)}${p.git.dirty ? ' dirty' : ''}`},
        {key: 'profile', value: p.profile},
        {key: 'mods', value: mods === null ? null : `${mods.length} mods`, title: mods?.map(([n, v]) => `${n} ${v}`).join(', ')},
        {key: 'map', value: p.map?.digest ?? null}
    ];
});

const plan = computed(() => {
    const c = props.replayCounts;
    if (c === null) return null;
    const parts = [`${c.steps} steps`];
    for (const [k, n] of [['abandoned', c.abandoned], ['lost', c.lost], ['failed', c.failed], ['pending', c.pending], ['believed', c.believed]] as const) {
        if (n > 0) parts.push(`${n} ${k}`);
    }
    return {text: parts.join(' · '), truncated: c.abandoned > 0};
});
</script>

<template>
  <header class="flex flex-wrap items-baseline gap-x-6 gap-y-3 border-b border-divider px-5 pb-3 pt-4">
    <h2 class="text-xl font-semibold">{{ summary.run_id }}</h2>
    <div class="flex flex-wrap">
      <span data-chip="roster" data-state="present" class="mb-1.5 mr-1.5 rounded border border-divider bg-surface px-2 py-1 font-mono text-xs text-ink-muted">
        roster <b class="font-medium text-ink">{{ roster.join(' ') }}</b>
      </span>
      <span v-if="provenance === null" data-chip="provenance" data-state="absent"
            class="mb-1.5 mr-1.5 rounded border border-dashed border-warn/60 px-2 py-1 font-mono text-xs text-warn-dark">
        provenance <i>{{ provenanceError ?? 'not captured' }}</i>
      </span>
      <span v-for="c in chips" :key="c.key" :data-chip="c.key" :data-state="c.value === null ? 'absent' : 'present'" :title="c.title"
            class="mb-1.5 mr-1.5 rounded border px-2 py-1 font-mono text-xs text-ink-muted"
            :class="c.value === null ? 'border-dashed border-divider' : 'border-divider bg-surface'">
        {{ c.key }} <b v-if="c.value !== null" class="font-medium text-ink">{{ c.value }}</b><i v-else>not captured</i>
      </span>
      <span data-chip="samples" :data-state="lagTicks === null ? 'absent' : 'present'" class="mb-1.5 mr-1.5 rounded border border-divider bg-surface px-2 py-1 font-mono text-xs text-ink-muted">
        <!-- lag <= 0: the last sample is at or past the run's end -- coverage, not a gap. -->
        samples <b class="font-medium text-ink">{{ lagTicks === null ? 'not sampled' : lagTicks <= 0 ? 'cover to end' : `lag ${lagTicks.toLocaleString()} ticks` }}</b>
      </span>
      <span v-if="plan" data-chip="plan" :data-state="plan.truncated ? 'truncated' : 'present'"
            class="mb-1.5 mr-1.5 rounded border px-2 py-1 font-mono text-xs"
            :class="plan.truncated ? 'border-warn/60 bg-warn/10 text-warn-dark' : 'border-divider bg-surface text-ink-muted'">
        plan <b class="font-medium" :class="plan.truncated ? 'text-warn-dark' : 'text-ink'">{{ plan.text }}</b>
      </span>
      <span v-for="s in savepoints" :key="s.milestone_index" data-chip="resume" data-state="present"
            class="mb-1.5 mr-1.5 rounded border border-divider bg-surface px-2 py-1 font-mono text-xs text-ink-muted"
            :title="`milestone ${s.milestone_index} at tick ${s.tick}, ${(s.bytes / 1048576).toFixed(1)} MB`">
        resume <code class="select-all text-ink">--resume-from {{ s.run_id }}:{{ s.milestone_index }}</code>
      </span>
    </div>
    <p class="basis-full text-sm text-ink-muted"><b class="font-medium text-ink">{{ headline }}</b></p>
  </header>
</template>
```

`RunPage.vue`: pass `:provenance="store.provenance" :provenance-error="store.provenanceError" :lag-ticks="store.sampleLag" :replay-counts="store.replayCounts" :savepoints="store.savepoints"`; replace the page's own `lag` computed with `store.sampleLag` (keep `CoverageBand`'s `run-end` as it is). `RunPage.spec.ts`: set `store.provenance` to the fixture-like object above and assert the `seed` chip reads `31337`; set `store.replay` (parsed) with one abandoned step and assert the `plan` chip has `data-state="truncated"`.

- [ ] **Step 4: Run to verify pass** — `cd app && pnpm vitest run src/components/run/RunHeadline.spec.ts src/pages/RunPage.spec.ts && pnpm lint` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
feat(app): provenance chips, plan counts and resume commands on the run page

An `abandoned` count above zero is the signal that a plan's tail never ran;
before the replay was persisted those steps were written nowhere.
EOF
git add -- app/src/components/run/RunHeadline.vue app/src/components/run/RunHeadline.spec.ts app/src/pages/RunPage.vue app/src/pages/RunPage.spec.ts
git commit -F /tmp/msg -- app/src/components/run/RunHeadline.vue app/src/components/run/RunHeadline.spec.ts app/src/pages/RunPage.vue app/src/pages/RunPage.spec.ts
```

---

### Task 9: The cursor readout and the idle share read the analysis clock

**Files:**
- Modify: `app/src/components/run/CursorBar.vue`, `CursorBar.spec.ts`, `LaneBand.vue`, `LaneBand.spec.ts`, `app/src/pages/RunPage.vue`

**Interfaces:**
- `CursorBar` gains a required `clock: TickScale`; its readout is `formatGameTime(clock, cursor)`; the range input keeps `scale` for its bounds.
- `LaneBand`'s `idlePct` denominator becomes `clock.to - clock.from` and `idleIntervals(lanes, b, clock)` (the analysis window), so the percentage is the share of the run the headline talks about; positions stay on `scale`.

- [ ] **Step 1: Write the failing tests**

`CursorBar.spec.ts`, add:
```ts
    it('reads game time off the analysis clock, not the trimmed axis', () => {
        const w = mount(CursorBar, {props: {scale: {from: 3417, to: 25224}, clock: {from: 3242, to: 25224}, cursor: 21242, playing: false, rate: 300}});
        expect(w.text()).toContain('5:00'); // (21242-3242)/60 = 300 s; off the axis it would read 4:57
    });
```
and add `clock: scale` to the existing mounts. `LaneBand.spec.ts`, add:
```ts
    it('idle share is measured over the analysis window', () => {
        const trimmed = {from: run.lo + 175, to: run.hi};
        const w = mount(LaneBand, {props: {scale: trimmed, clock: {from: run.lo, to: run.hi}, cursor: run.lo, lanes: run.lanes, events: run.events}});
        expect(w.text()).toContain('idle 32%'); // 6984 / 21982, the tool's figure
    });
```

- [ ] **Step 2: Run to verify failure** — `cd app && pnpm vitest run src/components/run/CursorBar.spec.ts src/components/run/LaneBand.spec.ts` — Expected: FAIL (`clock` unknown / 4:57 / a different percentage).

- [ ] **Step 3: Implement** — `CursorBar`: `defineProps<{scale: TickScale; clock: TickScale; cursor: number; playing: boolean; rate: number}>()` and `formatGameTime(clock, cursor)`. `LaneBand`: in `idlePct`, `const span = props.clock.to - props.clock.from;` and `idleIntervals(props.lanes, b, props.clock)`; update the prop doc to say the share is of the analysis window. `RunPage.vue`: `<CursorBar :scale="scale" :clock="clock" …>`.

- [ ] **Step 4: Run to verify pass** — the two specs plus `src/pages/RunPage.spec.ts` and `pnpm lint` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg <<'EOF'
fix(app): the cursor readout and the idle share read the analysis clock

The last two numbers on the page measured against the trimmed axis.
EOF
git add -- app/src/components/run/CursorBar.vue app/src/components/run/CursorBar.spec.ts app/src/components/run/LaneBand.vue app/src/components/run/LaneBand.spec.ts app/src/pages/RunPage.vue
git commit -F /tmp/msg -- app/src/components/run/CursorBar.vue app/src/components/run/CursorBar.spec.ts app/src/components/run/LaneBand.vue app/src/components/run/LaneBand.spec.ts app/src/pages/RunPage.vue
```

---

### Task 10: Gate, browser check, note

**Files:**
- Create: `docs/superpowers/notes/2026-09-09-run-anatomy-phase-2.md`

- [ ] **Step 1: Full gate** (only when the controller says the box is quiet)

```bash
nix develop -c cargo clippy --workspace --all-features --all-targets -- --deny warnings
nix develop -c cargo test -p factorio-bot-core -p factorio-bot-server -p factorio-bot-scripting-lua --features factorio-bot-server/lua
cd app && pnpm lint && pnpm run test:coverage && pnpm run build:web
```
Report every exit code. A red step is reported red.

- [ ] **Step 2: Browser check** — the controller builds the SPA and serves it (`pnpm start` from the worktree's `app/`, port 8080, proxying `/api` to the viewer on 7492 — the viewer must be a binary that includes these routes; if the running viewer is old, the chips read the 404 text `does not provide /provenance`, which is itself the correct degradation and worth a screenshot). Record what the chips read for `run-1788696619-00325`.

- [ ] **Step 3: Write the note**

```markdown
# Run Anatomy, Phase 2 — the record reaches the page

2026-09-09. Implements the Phase 2 row of
`docs/superpowers/specs/2026-09-08-run-anatomy-design.md`.

## What landed
- Routes: `/runs/{id}/provenance` (404 when absent), `/runs/{id}/replay`
  (persisted by the run at its end as `replay.json`), `/runs/{id}/savepoints`,
  `?from=&to=` on `/samples` and `/map`, `?kind=` on `/samples`; `RunSummary`
  carries `samples`, `map`, `samples_lag_ticks`.
- The page: provenance chips (absence drawn as "not captured", per field), a
  plan chip from the replay (`abandoned > 0` flagged as a truncated tail),
  one `--resume-from` command per savepoint; the cursor readout and the idle
  share now read the analysis clock.

## What the fixture run reads
<the chips as observed in the browser, and whether the viewer binary served the new routes>

## Not in this phase (ruled)
- Folding `/runs/:id/analysis` into the page: needs replay steps joined to
  lanes, and there is no safe key (schedule order vs dispatch order). Phase 3.
- Client use of the sample slices: every band reads every kind today.
- Heatmap rect merging and keyboard access; plateau annotation; the
  `truncated > 0` count.
```
Fill the angle-bracket section from Step 2; do not leave it.

- [ ] **Step 4: Commit**

```bash
cat > /tmp/msg <<'EOF'
docs(notes): Run Anatomy Phase 2, the record reaches the page
EOF
git add -- docs/superpowers/notes/2026-09-09-run-anatomy-phase-2.md
git commit -F /tmp/msg -- docs/superpowers/notes/2026-09-09-run-anatomy-phase-2.md
```

---

## Self-review

- **Spec §2.2 table**: provenance route → T1; RunSummary fields → T2; slices → T3; persisted replay + route → T4; savepoints → T5; seam → T6 (every task regenerates the snapshot so the TS side goes red until T6 mirrors it — deliberate).
- **Spec §1.1 chips**: T8 (seed, roster/mode, speed, commit+dirty, profile, mods, coverage) — `map` digest added beyond the spec list as it is the only identity for an unseeded map.
- **§4 error handling**: 404 → "not captured"/`enrichmentUnavailable` in T7/T8; replay parse failure → `replayError`.
- **Parked Phase 1 items**: CursorBar clock and LaneBand denominator → T9; evidence per lane and the analysis fold-in are named as Phase 3 with the reason.
- **Type consistency**: `Provenance`/`Savepoint` field names identical in Rust (T1/T5), TS (T6), store (T7), chips (T8); `replayCounts` shape identical in T7 and T8; `clock: TickScale` matches Phase 1's prop name.
- **Constraint check**: no new files under `crates/*/tests/`; all cargo under `nix develop -c`; every wire change goes through the snapshot.
