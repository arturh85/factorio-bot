# Frontend Transport Swap and Tauri Removal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Vue SPA talk to the axum HTTP API only — every `invoke()` replaced, script output streamed live over SSE — and then delete Tauri from the Rust crate, the npm package, the Nix devShell and CI.

**Architecture:** Three layers on the frontend. (1) `app/src/api/` is the whole transport: one `request()` over `fetch`, one typed function per route, one SSE consumer. (2) The five Pinia stores keep their public shape (getters and action names that views already call) and swap their bodies onto the client — four stores convert, `restapiStore` is deleted outright because "start/stop the REST API" is meaningless once the server *is* the API. (3) Views lose the two Tauri native file pickers and the `open_in_browser` shim. Then the `gui` feature, the `tauri*` dependency set and the `@tauri-apps/*` packages come out, with `types.ts` generation moved from `app/src-tauri/build.rs` to a new `crates/server/build.rs` so it survives.

**Tech Stack:** Vue 3.5, Pinia 4, vue-router 5 (hash history), Vite 8, vitest 4, ESLint 10 flat config, Tailwind v4, PrimeVue 4.5.5, TypeScript 5.9, pnpm 10. Rust: axum 0.8, utoipa 5.5, tokio.

**Spec:** `docs/superpowers/specs/2026-08-29-webserver-replaces-tauri-gui-design.md`

**Predecessors:**
- `docs/superpowers/plans/2026-08-29-axum-server-replaces-rocket.md` (plan 1, complete)
- `docs/superpowers/plans/2026-08-30-shared-settings-and-serve-command.md` (plan 2, complete)
- `docs/superpowers/plans/2026-08-30-management-api-reads-and-mutations.md` (plan 3, complete)
- `docs/superpowers/plans/2026-08-30-script-execution-jobs-and-sse.md` (plan 4, **in flight in another session**). This plan consumes plan 4's `POST /api/v1/scripts/execute`, `GET /api/v1/jobs`, `GET /api/v1/jobs/{id}` and `GET /api/v1/jobs/{id}/events` and must not start before plan 4's Task 7 is merged.

---

## Global Constraints

Copied verbatim from the brief. Every task's requirements implicitly include this section.

- `panic = "abort"` in the release profile: any `.unwrap()` reachable from a request is a remote process kill.
- No authentication, by explicit decision.
- Commits go to `master`; no feature branches.
- Rust: `cargo clippy --workspace --all-features --all-targets -- --deny warnings` and `cargo fmt --all -- --check` must pass.
- Frontend: `pnpm run lint` and `pnpm run test:coverage` must pass. **The package manager is pnpm, not yarn** — yarn was removed today. Vite is 8, vitest 4, ESLint 10 flat config, Tailwind v4, PrimeVue 4.
- **`git diff` in this repo lies to greps.** `diff.external` is set to difftastic, so `git diff` emits no `+`/`-` prefixed lines and any `grep "^+"` over it returns 0 — indistinguishable from "found nothing". Measured on a known one-line removal: `git diff … | grep -c "^-rusttype"` gives **0**, `git diff --no-ext-diff … | grep -c` gives **1**. `git show` and `git log` are *not* affected (they disable the external driver unless `--ext-diff` is passed), which is why review packages built with `git show` are genuine unified diffs. Pass `--no-ext-diff` to every `git diff` regardless — a verification command that cannot fail is worse than a wrong answer, because a wrong answer gets challenged and a false green does not.
- Cargo and pnpm both need the Nix devShell: `nix develop --command bash -c 'eval "$(mise env -s bash)"; <command>'`

### Plan-specific constraints

- **The ESLint config bans double quotes and trailing commas** (`app/eslint.config.mjs`: `quotes: ['error', 'single']`, `comma-dangle: ['error', 'never']`). Every TypeScript snippet in this plan is already written that way; keep it that way.
- **`app/src/models/types.ts` is generated and eslint-ignored.** Never hand-edit it. Hand-written DTOs go in `app/src/api/types.ts`.
- **Frontend commands run from `app/`**, e.g. `nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint'`.
- **Do not touch, in any task:** `crates/planner/`, `crates/executor/`, `crates/core/src/{scripts,draw,test_utils}.rs`, or anything under `crates/scripting_lua/`. Other agents hold uncommitted work there.

---

## Collisions with other agents (read before starting)

1. **`app/src-tauri/build.rs` has uncommitted changes from another agent.** Two tasks touch it: **Task 2** changes exactly one token on line 93 (`PrimeVueTreeNode::type_script_ify()` → `ScriptTreeNode::type_script_ify()`), and **Task 13** moves the whole `typescriptify()` function out. Both must read the working-tree file first, never the committed version.

   The uncommitted change (verified with `git status` while writing this plan: `M app/src-tauri/build.rs`) is small — it hoists `#[cfg(feature = "restapi")] use factorio_bot_core::settings::RestApiSettings;` above the `settings::*` glob inside `typescriptify()`. Task 13 copies that whole function into a new `crates/server/build.rs`; copying the committed version instead would silently revert the other agent's fix. Task 13 also *deletes* code from `build.rs`; if the file has moved on again by then, rebase the deletion onto whatever is there rather than restoring this plan's snapshot.
2. **Plan 4 is editing `crates/server/src/state.rs`, `crates/server/src/manage/mod.rs` and `crates/server/tests/openapi.rs`.** Task 1 edits all three and Task 2 appends to `tests/openapi.rs`. Re-read each file before editing and add alongside plan 4's changes; never replace the file wholesale.
3. **Plan 4 introduces the first templated path in the spec (`/api/v1/jobs/{id}`).** `crates/server/tests/openapi.rs::no_operation_publishes_a_path_parameter` currently asserts `!path.contains('{')` for every path in the document and will fail once that route lands. That is plan 4's test to fix, not this plan's — but if you hit it, the fix is to exempt templated paths from the premise assertion while keeping the `in: path` check for untemplated ones.

---

## Findings from research that shape this plan

**Every `invoke()` call site, and the route that replaces it.** Nineteen call sites across five stores plus two Vue components.

| Store / component | `invoke()` command | Replacement |
|---|---|---|
| `appStore.fileExists` | `file_exists` | `GET /api/v1/fs/exists?path=` → `{ exists }` |
| `appStore.loadSettings` | `load_settings` | `GET /api/v1/settings` → `AppSettings` |
| `appStore._updateSettings` | `update_settings` | `PUT /api/v1/settings` → `AppSettings` |
| `appStore.maximizeWindow` | `maximize_window` | **No route. Desktop-only; deleted.** (It also had a latent bug: it assigned its `void` result over `this.settings`.) |
| `appStore.openInBrowser` | `open_in_browser` | **No route. Deleted**; becomes `<a target="_blank" rel="noopener">`. |
| `instanceStore.checkInstanceState` | `is_instance_started` | `GET /api/v1/instance` → `InstanceStatus` |
| `instanceStore.startInstances` | `start_instances` | **NO ROUTE EXISTS.** See below. |
| `instanceStore.stopInstances` | `stop_instances` | `POST /api/v1/instance/stop` → `204` |
| `rconStore.execute` | `execute_rcon` | `POST /api/v1/rcon` `{ command }` → `204` |
| `restapiStore.init` | `is_restapi_started` | **No route. Store deleted.** |
| `restapiStore.startRestApi` | `start_restapi` | **No route. Store deleted.** |
| `restapiStore.stopRestApi` | `stop_restapi` | **No route. Store deleted.** |
| `restapiStore.startRestApi` | `is_port_available` | **No route. Store deleted.** |
| `scriptStore.loadScriptsInDirectory` | `load_scripts_in_directory` | `GET /api/v1/scripts?path=` → `ScriptTreeNode[]` |
| `scriptStore.loadScriptFile` | `load_script` | `GET /api/v1/scripts/file?path=` → `{ code }` |
| `scriptStore.setCode` | `save_script` | `PUT /api/v1/scripts/file?path=` `{ code }` → `204` |
| `scriptStore.executeCode` | `execute_code` | `POST /api/v1/scripts/execute` `{ code, language }` → `202 { job_id }` + SSE |
| `scriptStore.executeScript` | `execute_script` | `POST /api/v1/scripts/execute` `{ path }` → `202 { job_id }` + SSE |
| `SettingsPage.vue:90,104` | `@tauri-apps/plugin-dialog` `open()` | **No browser equivalent** (these pick *server-side* paths). Becomes a plain text input validated through `GET /api/v1/fs/exists`. |

**Factorio startup blocks the async runtime, wherever it is called from.** `FactorioInstance::start` is `async`, but it reaches `extract_archive` (`crates/core/src/process/io_utils.rs:86`) — a plain synchronous `fn` that unpacks the whole archive in 8-10 minutes on a first run, with no yield points. Every caller inherits this: the CLI, the REPL, `roll_best_seed`, and now the new HTTP route. Task 1 fixes it once, in `crates/core`, by putting the single blocking call on `spawn_blocking`. This is the same defect plan 4 fixes for script execution in its Task 4; it was deferred here on the grounds that there was no start endpoint yet to attach it to.

**The one real gap: `POST /api/v1/instance/start` does not exist.** The spec's endpoint table lists it (`202` + job id), but neither plan 3 nor plan 4 implemented it — `crates/server/src/manage/mod.rs` registers `get_instance` and `stop_instance` only, and `grep -rn "instance/start" crates app` returns nothing. Without it the Start button in `ProcessControl.vue` has no backend at all and the browser app cannot launch Factorio. Task 1 adds it. It cannot be a synchronous handler: first-run archive extraction takes 8-10 minutes (see `CLAUDE.md`), so it answers `202` and spawns the start, and `GET /api/v1/instance` grows `starting` and `last_error` fields the UI polls.

**Two smaller gaps, deliberately left open:**
- `POST` and `DELETE /api/v1/scripts/file` (create / delete a script) exist server-side and have no UI. Out of scope; the client wrapper exposes them so a later plan has them ready.
- Autostart (`settings.gui.enable_autostart`) fires today from `App.vue`'s `onMounted`, i.e. "whenever anyone opens a tab". The spec moves it to server startup. `cli/serve.rs` does not do that yet. Task 11 removes the client-side trigger; **server-side autostart is not implemented by this plan** and is called out as follow-up work, not silently dropped.

**Error-body shape is `{ message, code }`**, not the `{ error, detail }` the spec sketched (`crates/server/src/error.rs`). Plan 4's Task 6 documents its 409 body as `{ "code": 409, "error": "…", "running_job_id": "2" }`, which spells the text field `error` instead. Task 3's parser accepts either key rather than betting on one.

**`typescript-definitions` survives Tauri removal.** It is a normal dependency of `crates/core` (`crates/core/Cargo.toml:37`), and the derives live on the core types. Only the *invocation* is Tauri-coupled: `typescriptify()` in `app/src-tauri/build.rs` runs under `#[cfg(feature = "gui")]`, so deleting the `gui` feature silently stops regenerating `app/src/models/types.ts`. Task 13 moves it to `crates/server/build.rs` before Task 14 deletes the feature.

**Transport selection today is vestigial.** `VITE_HTTP_HANDLER=NATIVE` (set by `native:serve` and `tauri:build`) only makes `app/src/plugins/configure-ynetwork.js` install a `ynetwork` request runner backed by `@tauri-apps/plugin-http`. Nothing in the app calls `YNetwork` — the stores all use `invoke()`. The file, the env var and the `ynetwork` dependency are dead weight and are deleted in Task 12.

**Not this plan's problem — noted and moved on:** the templates still carry PrimeFlex 1/2 grid classes (`p-grid`, `p-col-12`, `p-formgrid`, `p-field-radiobutton`) that resolve to no rules under the installed stack, so those pages render as stacked blocks. PrimeFlex is archived upstream. Fixing the layout is the *next* plan (a shadcn-vue / Tailwind redesign). Do not restyle anything here; keep the existing class names on markup you touch so the redesign has one diff to make.

---

## Scope: the app is four pages, not eleven

`app/src/router.ts` declares eleven routes. Verified against the components they resolve to, only **four contain any logic or make any backend call**, and only five are reachable from the sidebar menu (`App.vue:54-66`). Nobody should later mistake a placeholder for lost work, so here is the whole inventory:

| Route | Component | State |
|---|---|---|
| `/` | `Dashboard.vue` | **Live.** Reads `appStore` client count. Touched only through the store. |
| `/settings` | `SettingsPage.vue` | **Live.** The two Tauri file pickers and the REST API card live here. Task 11. |
| `/rcon` | `RconPage.vue` | **Live.** `rconStore` only; no file change needed beyond the store. |
| `/script` | `ScriptPage.vue` | **Live.** Editor + tree + output. Task 10 adds the unmount hook. |
| `/tasks` | `TasksPage.vue` → `GanttChart.vue` | **Placeholder.** `GanttChart.vue`'s entire template is `<div>TODO</div>`; the rest of the file is a commented-out mermaid string. In the menu. |
| `/instances` | `GameInstances.vue` | **Placeholder.** A static card reading "List of Setup Instances:". Commented out of the menu. |
| `/empty` | `EmptyPage.vue` | **Placeholder.** Not in the menu. |
| `/factorioMods` | `EmptyPage.vue` | **Placeholder.** Not in the menu. |
| `/restApiDocss` | `EmptyPage.vue` | **Placeholder.** Not in the menu. |
| `/luaApiDocss` | `EmptyPage.vue` | **Placeholder.** Not in the menu. |
| `/workspace` | `EmptyPage.vue` | **Placeholder.** Not in the menu. |

**Consequence for this plan:** the only Vue files it edits are `SettingsPage.vue`, `App.vue`, `ScriptPage.vue` and `ScriptTree.vue`. The seven placeholder routes carry zero `invoke()` calls and zero HTTP calls, so the transport swap does not reach them. They are **not deleted here** — deciding what replaces them belongs to the redesign plan, and deleting them now would only make that plan's diff harder to read. Task 17's browser check screenshots the three live pages and ignores the placeholders on purpose.

---

## Decision: hand-written client, spec-pinned by a contract test

**Hand-write a thin `fetch` wrapper (`app/src/api/`) rather than generating a client from `/openapi.json`, and pin it to the spec with a contract test:** the surface is sixteen management endpoints whose domain payloads are *already* generated from the same Rust structs the handlers serialize (`app/src/models/types.ts`), so `openapi-typescript` would buy type-safety we mostly have while adding a codegen toolchain plus a spec-dump step in CI to keep checked-in output honest — the contract test in Task 5 gets the "spec is the source of truth" guarantee for a tenth of the machinery.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/src/types.rs` (modify) | `PrimeVueTreeNode` becomes `ScriptTreeNode`; fields and `#[schema(no_recursion)]` unchanged. |
| `crates/server/src/manage/scripts.rs` (modify) | Four `ScriptTreeNode` call sites. |
| `app/src/components/ScriptTree.vue` (modify) | Renamed type import; no behaviour change. |
| `crates/core/src/process/instance_setup.rs` (modify) | The synchronous `extract_archive` call moves onto `spawn_blocking`, so a first-run start stops parking an async worker for 8-10 minutes. Fixes the CLI and REPL paths too. |
| `crates/server/src/manage/instance.rs` (modify) | Gains `start_instance`; `InstanceStatus` gains `starting` and `last_error`. |
| `crates/server/src/state.rs` (modify) | `AppState` gains `starting: Arc<AtomicBool>` and `last_start_error: Arc<RwLock<Option<String>>>`. |
| `crates/server/src/manage/mod.rs` (modify) | Registers `start_instance`. |
| `crates/server/tests/manage_instance.rs` (modify) | HTTP tests for the new route. |
| `crates/server/tests/openapi.rs` (modify) | `/api/v1/instance/start` in the route list. |
| `crates/server/build.rs` (create) | `typescriptify()`, moved out of `app/src-tauri/build.rs`. |
| `app/src/api/http.ts` (create) | `apiBase`, `buildUrl`, `ApiError`, `request<T>` — the only place `fetch` is called. |
| `app/src/api/types.ts` (create) | Hand-written DTOs for server types that are not in the generated `models/types.ts`. |
| `app/src/api/client.ts` (create) | One exported function per route. No state, no error swallowing. |
| `app/src/api/jobEvents.ts` (create) | `subscribeJobEvents` — the SSE state machine. |
| `app/src/api/openapi.contract.spec.ts` (create) | Asserts every path/method `client.ts` uses exists in the committed OpenAPI snapshot. |
| `app/src/api/openapi.snapshot.json` (create) | The spec, dumped from the server by a pnpm script and committed. |
| `app/src/store/appStore.ts` (modify) | Settings + `fileExists` over HTTP; `maximizeWindow`, `openInBrowser`, `updateEnableRestApi` gone. |
| `app/src/store/instanceStore.ts` (modify) | `starting`/`lastError` from `GET /api/v1/instance`; start is fire-and-poll. |
| `app/src/store/rconStore.ts` (modify) | `POST /api/v1/rcon`; the swallowed-error bug fixed. |
| `app/src/store/scriptStore.ts` (modify) | Script CRUD over HTTP; execution over `POST /scripts/execute` + SSE. |
| `app/src/store/restapiStore.ts` (delete) | No routes back it; the server *is* the API. |
| `app/src/plugins/configure-ynetwork.js` (delete) | Dead Tauri fetch runner. |
| `app/src/pages/SettingsPage.vue` (modify) | Text inputs instead of native pickers; plain links; REST API card becomes a Swagger UI link. |
| `app/src/App.vue` (modify) | Bootstrap without `maximizeWindow`/restapi/autostart; 2 s instance poll. |
| `app/vite.config.mts` (modify) | Dev proxy to `127.0.0.1:7492`; coverage thresholds. |
| `app/package.json` (modify) | `@tauri-apps/*`, `ynetwork`, `cross-env`, `concurrently` out; `tauri:*` scripts out; `playwright-core` in. |
| `app/e2e/smoke.mjs` (create) | Headless-Chromium browser check with screenshots. |
| `app/src-tauri/src/gui/` (delete) | The nineteen `#[tauri::command]` handlers. |
| `app/src-tauri/src/repl/gui.rs` (delete) | REPL `gui` command. |
| `app/src-tauri/Cargo.toml` (modify) | `gui`/`custom-protocol` features and the `tauri*`/`open`/`port_scanner`/`typescript-definitions` deps out. |
| `app/src-tauri/{tauri.conf.json,capabilities/,gen/,icons/}` (delete) | Tauri bundling inputs. |
| `flake.nix` (modify) | webkitgtk/gtk3/soup/… out; `chromium` in for the browser check. |
| `.github/workflows/test.yml`, `.github/workflows/publish.yml` (modify) | No tauri-cli, no webkit apt line, no bundler. |
| `justfile` (modify) | `just start` runs Vite; a `just serve` runs the server. |

---

## Task 1: Fill the missing route — `POST /api/v1/instance/start`

The Start button has no backend. This is the one server-side gap found while mapping `invoke()` calls, and the browser app is unusable without it.

**Files:**
- Modify: `crates/core/src/process/instance_setup.rs:65-68`
- Modify: `crates/server/src/state.rs`
- Modify: `crates/server/src/manage/instance.rs`
- Modify: `crates/server/src/manage/mod.rs`
- Test: `crates/core/src/process/instance_setup.rs` (unit)
- Modify: `crates/server/tests/manage_instance.rs`
- Modify: `crates/server/tests/openapi.rs`

**Interfaces:**
- Consumes: `AppState { instance: SharedFactorioInstance, settings: SharedAppSettings, settings_path: PathBuf }` and `AppState::new(instance, settings)` from `crates/server/src/state.rs`; `factorio_bot_core::process::process_control::{FactorioInstance, FactorioParams}`; `crate::error::ErrorResponse::{conflict, internal}`.
- Produces:
  - `AppState.starting: std::sync::Arc<std::sync::atomic::AtomicBool>`
  - `AppState.last_start_error: std::sync::Arc<tokio::sync::RwLock<Option<String>>>`
  - `InstanceStatus { started: bool, starting: bool, client_count: u8, server_port: Option<u16>, rcon_port: Option<u16>, last_error: Option<String> }`
  - `StartAccepted { accepted: bool }`
  - `pub async fn start_instance(State(AppState)) -> Result<(StatusCode, Json<StartAccepted>), ErrorResponse>` at `POST /api/v1/instance/start`
  - Behaviour change in `factorio_bot_core::process::instance_setup::setup_factorio_instance`: the archive extraction runs on the blocking pool. Its signature does not change, so `crates/scripting_lua/src/roll_best_seed.rs:49` and `crates/core/src/process/process_control.rs:85,102` need no edit.

**Before you start:** plan 4 is editing `state.rs`, `manage/mod.rs` and `tests/openapi.rs`. Re-read each one and add to it; do not paste over it.

### Where the blocking work goes, and why

`FactorioInstance::start` is `async`, but on a first run it reaches
`extract_archive` (`crates/core/src/process/io_utils.rs:86`) — a plain
synchronous `fn` that decompresses the whole Factorio archive and writes it
out, taking 8-10 minutes and never yielding. Whatever thread awaits that call
is held for the entire time. `tokio::spawn` would put it on an async worker,
so on the default multi-threaded runtime one worker disappears for ten
minutes; combined with plan 4's script execution, a first-run start plus a
couple of long scripts starve the runtime and `/api/v1/health` stops
answering.

**Fix it in `crates/core`, by wrapping only the synchronous `extract_archive`
call in `tokio::task::spawn_blocking`, and leave the route's `tokio::spawn`
alone** — the defect belongs to the core function rather than to the HTTP
handler, so fixing it at the route would leave the identical trap for the CLI
(`cli/start.rs`), the REPL and `roll_best_seed`, all of which call the same
path today. `setup_factorio_instance` is `async` and cannot itself be handed
to `spawn_blocking` (which takes a synchronous closure); the one genuinely
blocking call inside it can.

The route keeps `tokio::spawn` because what it wraps is, after this fix, an
ordinary async future whose long pole is on the blocking pool where it
belongs.

- [ ] **Step 1: Write the failing tests**

Append to `crates/server/tests/manage_instance.rs`:

```rust
#[tokio::test]
async fn starting_when_an_instance_already_runs_is_a_conflict() {
    let instance: SharedFactorioInstance = Arc::new(RwLock::new(Some(empty_factorio_instance())));
    let response = build_router(state_with(instance), None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/instance/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_second_start_while_one_is_in_flight_is_a_conflict() {
    // Occupy the starting slot directly rather than racing two real starts:
    // the contract under test is the refusal, not the scheduler's timing.
    let state = state_with(FactorioInstance::new_shared());
    state
        .starting
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/instance/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn instance_status_reports_the_in_flight_start() {
    // The UI polls this flag instead of holding a request open for the 8-10
    // minutes a first-run archive extraction takes.
    let state = state_with(FactorioInstance::new_shared());
    state
        .starting
        .store(true, std::sync::atomic::Ordering::SeqCst);
    *state.last_start_error.write().await = Some("previous attempt exploded".into());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .uri("/api/v1/instance")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status["started"], false);
    assert_eq!(status["starting"], true);
    assert_eq!(status["last_error"], "previous attempt exploded");
}
```

And in `crates/server/tests/openapi.rs`, add `"/api/v1/instance/start"` to the path list in `openapi_json_lists_every_route` and `("/api/v1/instance/start", "post")` to the `(path, method)` list in the same test.

- [ ] **Step 2: Write the failing runtime-starvation test**

A test that only asserts the 202 comes back passes just as happily with the
blocking bug in place. This one asserts the runtime made progress on other
tasks *while the extraction ran*, on a single-worker runtime so the result is
decisive rather than lucky: with the extraction on the worker the ticker gets
exactly zero ticks, with it on the blocking pool it gets dozens.

Append to `crates/core/src/process/instance_setup.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Builds a `.tar.xz` in the shape `extract_archive` expects on unix: a
    /// single top-level `factorio/` directory containing `data/`.
    ///
    /// Slow-by-file-count rather than slow-by-size: 20 000 tiny files make
    /// `Archive::unpack` do 20 000 file creations, which is syscall-bound and
    /// reliably takes well over a hundred milliseconds, while compressing to
    /// almost nothing and costing the test no meaningful disk.
    fn build_slow_archive(path: &Path) {
        let file = File::create(path).expect("create archive");
        let encoder = xz2::write::XzEncoder::new(file, 1);
        let mut builder = tar::Builder::new(encoder);
        let payload = [b'x'; 32];
        for index in 0..20_000 {
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    format!("factorio/data/file-{index}.bin"),
                    &payload[..],
                )
                .expect("append file");
        }
        builder.into_inner().expect("finish tar").finish().expect("finish xz");
    }

    /// The defect this guards against: `extract_archive` is a synchronous
    /// `fn` doing minutes of file IO on a first run. Awaited directly it
    /// parks whatever thread runs it, so on the server a single first-run
    /// start takes an async worker out of service for the whole extraction
    /// and `/api/v1/health` stops answering.
    ///
    /// `worker_threads = 1` is what makes this decisive: `spawn_blocking`
    /// uses the blocking pool, which exists independently of the worker
    /// count, so the ticker keeps running; a direct call occupies the one
    /// and only worker and the ticker cannot tick at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn extracting_an_archive_does_not_park_the_async_worker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace");
        let archive = dir.path().join("factorio.tar.xz");
        build_slow_archive(&archive);

        let ticks = Arc::new(AtomicUsize::new(0));
        let ticker = {
            let ticks = ticks.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    ticks.fetch_add(1, Ordering::SeqCst);
                }
            })
        };

        // The call fails partway through — there is no Factorio binary in
        // this synthetic archive — and that is fine: extraction happens
        // early, and the assertion is about the runtime, not the result.
        let _ = setup_factorio_instance(
            workspace.to_str().expect("utf-8 workspace path"),
            archive.to_str().expect("utf-8 archive path"),
            &RconSettings::new(4321, "foobar", None),
            None,
            "server",
            true,
            false,
            None,
            None,
            true,
        )
        .await;

        // Snapshot before aborting: the ticker would otherwise keep counting
        // after the call returns and mask a fully blocked extraction.
        let observed = ticks.load(Ordering::SeqCst);
        ticker.abort();

        assert!(
            observed >= 3,
            "the runtime made no progress while the archive extracted ({observed} ticks); \
             the extraction is blocking an async worker"
        );
    }
}
```

- [ ] **Step 3: Run both sets and watch them fail**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test manage_instance --test openapi'
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core extracting_an_archive_does_not_park -- --nocapture'
```

Expected: the server tests fail to compile (`AppState` has no field `starting`); the core test compiles and **fails** on `0 ticks`. If it passes before the fix, the archive is extracting too fast to observe — raise the file count to 60 000 rather than weakening the assertion.

- [ ] **Step 4: Move the archive extraction off the async worker**

In `crates/core/src/process/instance_setup.rs`, replace lines 65-68:

```rust
    let readdir = instance_path.read_dir().into_diagnostic()?;
    if readdir.count() == 0 {
        extract_archive(factorio_archive_path, instance_path, workspace_path)?;
    }
```

with:

```rust
    let readdir = instance_path.read_dir().into_diagnostic()?;
    if readdir.count() == 0 {
        // `extract_archive` is a plain synchronous `fn`: on a first run it
        // decompresses and writes out the entire Factorio archive, which
        // takes 8-10 minutes and never yields. Awaiting it directly parks
        // whatever thread runs it — on the server that is one of a small
        // number of async workers, and a start plus a couple of long scripts
        // is enough to stop `/api/v1/health` answering. `spawn_blocking` puts
        // it on the blocking pool, which exists for exactly this.
        let archive = factorio_archive_path.to_owned();
        let target = instance_path.to_path_buf();
        let workspace = workspace_path.to_path_buf();
        tokio::task::spawn_blocking(move || extract_archive(&archive, &target, &workspace))
            .await
            // A `JoinError` means the extraction panicked or was cancelled.
            // Turn it into an ordinary error: `?`-ing a panic back onto this
            // thread would abort the whole process under `panic = "abort"`.
            .map_err(|err| miette!("archive extraction task failed: {err}"))??;
    }
```

`miette!` and `IntoDiagnostic` are already imported in this file; `std::sync::Arc` and `Path`/`PathBuf` are too.

- [ ] **Step 5: Run the core test and watch it pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core extracting_an_archive_does_not_park -- --nocapture'
```

Expected: PASS, with a tick count in the dozens.

- [ ] **Step 6: Add the two state fields**

In `crates/server/src/state.rs`:

```rust
use factorio_bot_core::app_settings::SharedAppSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::RwLock;
```

Add to the `AppState` struct, after `settings_path`:

```rust
    /// True between `POST /api/v1/instance/start` accepting and the spawned
    /// start finishing, win or lose. Starting Factorio takes 12-17 seconds
    /// normally and 8-10 minutes when the archive still has to be extracted,
    /// so the request cannot wait for it: it answers 202 and the client polls
    /// `GET /api/v1/instance`. `AtomicBool` rather than a lock because the
    /// slot is claimed with a single `compare_exchange` — a read-then-write
    /// pair would let two simultaneous requests both start Factorio.
    pub starting: Arc<AtomicBool>,
    /// Why the last start attempt failed. Set by the spawned task, cleared
    /// when a new attempt is accepted. Without it a failed background start
    /// is invisible to the browser: `starting` simply goes false again.
    pub last_start_error: Arc<RwLock<Option<String>>>,
```

And in `AppState::new`, after `settings_path`:

```rust
            starting: Arc::new(AtomicBool::new(false)),
            last_start_error: Arc::new(RwLock::new(None)),
```

- [ ] **Step 7: Extend `InstanceStatus` and add the handler**

In `crates/server/src/manage/instance.rs`, replace the `InstanceStatus` struct and `get_instance` with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstanceStatus {
    pub started: bool,
    /// A start accepted by `POST /api/v1/instance/start` is still running.
    pub starting: bool,
    pub client_count: u8,
    pub server_port: Option<u16>,
    pub rcon_port: Option<u16>,
    /// Why the last start attempt failed, or `null`.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StartAccepted {
    pub accepted: bool,
}

/// Reports whether a Factorio instance is running
#[utoipa::path(
    get,
    path = "/api/v1/instance",
    tag = "Admin",
    responses(
        (status = 200, body = InstanceStatus),
    )
)]
pub async fn get_instance(State(state): State<AppState>) -> ApiResult<InstanceStatus> {
    let starting = state.starting.load(Ordering::SeqCst);
    let last_error = state.last_start_error.read().await.clone();
    let instance = state.instance.read().await;
    let status = match instance.as_ref() {
        Some(instance) => InstanceStatus {
            started: true,
            starting,
            client_count: instance.client_count,
            server_port: instance.server_port,
            rcon_port: Some(instance.rcon_port),
            last_error,
        },
        None => InstanceStatus {
            started: false,
            starting,
            client_count: 0,
            server_port: None,
            rcon_port: None,
            last_error,
        },
    };
    Ok(Json(status))
}

/// Starts a Factorio instance in the background.
///
/// Answers `202` and never blocks: `FactorioInstance::start` runs the
/// synchronous archive extraction on first use, which takes 8-10 minutes, and
/// a browser (or any proxy in front of it) would time out long before that.
/// Progress is observed through `GET /api/v1/instance`.
#[utoipa::path(
    post,
    path = "/api/v1/instance/start",
    tag = "Admin",
    responses(
        (status = 202, body = StartAccepted),
        (status = 409, body = crate::error::ErrorResponse),
    )
)]
pub async fn start_instance(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<StartAccepted>), ErrorResponse> {
    // A courtesy check, not a gate: two requests arriving together can both
    // pass this before either reaches the `compare_exchange` below. It earns
    // its place only by giving the common single-request case the accurate
    // message ("already started" rather than "already starting").
    if state.instance.read().await.is_some() {
        return Err(ErrorResponse::conflict("instance already started"));
    }
    // *This* is what serialises concurrent starts. `compare_exchange` claims
    // the slot in one atomic step; a read-then-write pair would leave a
    // window in which two requests both see `false` and both spawn a start,
    // and the second one's Factorio would fail on the first one's port and
    // lock files.
    if state
        .starting
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(ErrorResponse::conflict("instance is already starting"));
    }
    *state.last_start_error.write().await = None;

    let settings = state.settings.clone();
    let instance = state.instance.clone();
    let starting = state.starting.clone();
    let last_start_error = state.last_start_error.clone();

    tokio::spawn(async move {
        let (factorio_settings, params) = {
            let settings = settings.read().await;
            let seed = settings.factorio.seed.to_string();
            let map_exchange_string = settings.factorio.map_exchange_string.to_string();
            let params = FactorioParams {
                client_count: settings.factorio.client_count,
                recreate: settings.factorio.recreate,
                seed: if seed.is_empty() { None } else { Some(seed) },
                map_exchange_string: if map_exchange_string.is_empty() {
                    None
                } else {
                    Some(map_exchange_string)
                },
                ..FactorioParams::default()
            };
            (settings.factorio.clone(), params)
        };

        match FactorioInstance::start(&factorio_settings, params).await {
            Ok(started) => {
                // Publish the instance before clearing `starting`, for the
                // same reason the error arm publishes its message first: a
                // poller that catches `starting == false` with neither an
                // instance nor an error would report "not started, no
                // problem" for a start that actually succeeded.
                *instance.write().await = Some(started);
            }
            Err(err) => {
                tracing::error!("failed to start factorio instance: {err:?}");
                // Written *before* `starting` is cleared. The other order has
                // a window in which a poll sees `starting == false` and
                // `last_error == None` and concludes the start succeeded —
                // the failure would be invisible until the next attempt.
                *last_start_error.write().await = Some(format!("{err:?}"));
            }
        }
        // Released on every path, including the error path: leaving it set
        // would wedge the server into permanent 409s with nothing running.
        starting.store(false, Ordering::SeqCst);
    });

    Ok((StatusCode::ACCEPTED, Json(StartAccepted { accepted: true })))
}
```

Add these imports at the top of the same file:

```rust
use factorio_bot_core::process::process_control::{FactorioInstance, FactorioParams};
use std::sync::atomic::Ordering;
```

- [ ] **Step 8: Register the route**

In `crates/server/src/manage/mod.rs`, add after `.routes(routes!(instance::get_instance))`:

```rust
        .routes(routes!(instance::start_instance))
```

- [ ] **Step 9: Run both suites and watch them pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server -p factorio-bot-core'
```

Expected: PASS, including the pre-existing `stopping_takes_the_instance_out_of_shared_state` and the new `extracting_an_archive_does_not_park_the_async_worker`.

- [ ] **Step 10: Prove the `spawn_blocking` is load-bearing (mutation)**

In `instance_setup.rs`, put the direct call back:

```rust
        extract_archive(factorio_archive_path, instance_path, workspace_path)?;
```

Run `cargo test -p factorio-bot-core extracting_an_archive_does_not_park`. It **must** fail with `0 ticks` — that is the whole point of the single-worker runtime. If it still passes, the test is not exercising the extraction at all (check that the tempdir's instance directory really is empty, so the `readdir.count() == 0` branch is taken) and fix the test before restoring the fix. Then restore `spawn_blocking`.

- [ ] **Step 11: Prove the atomic claim is load-bearing (mutation)**

Replace the `compare_exchange` with `if state.starting.load(Ordering::SeqCst) { return Err(...) } state.starting.store(true, Ordering::SeqCst);`. `a_second_start_while_one_is_in_flight_is_a_conflict` still passes — it is not a race test — so this one is checked by inspection rather than by the suite: confirm the two-step version has a window between the load and the store in which two requests both see `false`, note it in the commit message, and restore the `compare_exchange`.

- [ ] **Step 12: Lint and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all && cargo fmt --all -- --check && cargo clippy --workspace --all-features --all-targets -- --deny warnings'
git add crates/core/src/process/instance_setup.rs crates/server
git commit -m "feat(server): add POST /api/v1/instance/start; extract archives off the async runtime"
```

---

## Task 1b: Close the two gaps Task 1's review left open

Both surfaced by Task 1's implementer as concerns "for the plan that owns the frontend". **This is that plan**, so they are tasks rather than notes — a concern deferred to an unnamed future plan is a concern that does not get fixed.

**Files:** `crates/server/src/manage/instance.rs`, `crates/server/tests/manage_instance.rs`

### Gap 1 — a browser-only first run cannot start Factorio at all

`POST /api/v1/instance/start` answers `202`, the detached task fails, and `GET /api/v1/instance` reports `WorkspaceNotFound`. **The cause is not a missing directory** — an earlier draft of this task said it was, and that was wrong. Diagnosed before dispatch:

- `settings.factorio.workspace_path` defaults to **`""`** (`crates/core/src/settings.rs:35`, and `AppSettings.toml:9` documents it as "defaults to local_data_dir/workspace").
- `serve.rs:61-78` resolves that: empty → `factorio_bot_core::paths::workspace_dir()`, reject relative, then `ensure_scripts_dir` — which creates the directory as a side effect of creating `workspace/scripts`. **So the directory does exist after `serve` starts.**
- But `serve` never writes the resolved value back to settings, and `FactorioInstance::start` passes the **raw settings string** down to `setup_factorio_instance`, which does `Path::new("").exists()` → false → `WorkspaceNotFound` (`instance_setup.rs:157-162`).

So the defect is a **second, divergent copy of the resolution rule**, not a missing `create_dir_all`. Adding one would not fix it: creating `Path::new("")` does not help, and the route would still fail.

This is the same defect class as plan 4's finding I3 — two code paths resolving one configured name differently — and it gets the same remedy: **one resolution, in one place, that both callers use.**

- [ ] **Step 1: Extract the resolution.** Move `serve.rs:61-78`'s logic into `crates/core` beside `paths::workspace_dir()` — something like `paths::resolve_workspace(configured: &str) -> Result<PathBuf>`: empty → `workspace_dir()`, relative → error, absolute → as given. `serve.rs` then calls it instead of inlining it.
- [ ] **Step 2: Write the failing test** — a start with `workspace_path = ""` must not report `WorkspaceNotFound`. Assert on `last_error` and on the instance state, **not** on the 202, which is returned before the failure happens and proves nothing.
- [ ] **Step 3: Run it and confirm it fails with `WorkspaceNotFound`** — not with something else. A test that fails for a different reason is not evidence.
- [ ] **Step 4: Have the route resolve through the shared helper** before handing the path to `FactorioInstance::start`.
- [ ] **Step 5: Run it and confirm it passes.**
- [ ] **Step 6: Mutation.** Revert the route to passing the raw settings string. Input class: *a default (empty) `workspace_path`*. The new test must fail by name. **Assert the edit landed first.**

**Check before you build on any of this.** The diagnosis above was derived by reading, not by running the route. If `grep` disagrees with a line number or the resolution has moved, report that rather than working around it.

### Gap 2 — a stop racing an in-flight start publishes an instance the user just cleared

`stop_instance` does not clear `starting` or `last_error`. So: start (202, extraction running) → stop → the detached task completes → it publishes the instance into `state.instance`. The user pressed Stop and got a running game.

- [ ] **Step 1: Write the failing test** — begin a start, stop, let the spawned task complete, assert `GET /api/v1/instance` reports **not started**.
- [ ] **Step 2: Run it, confirm the instance is published anyway.**
- [ ] **Step 3: Fix it.** The spawned task must check, under the same lock that publishes, whether a stop intervened — a flag read before publishing is another read-then-write race, which is the defect Task 1's `compare_exchange` exists to avoid. Reuse that shape rather than inventing a second one.
- [ ] **Step 4: Run it, confirm it passes.**
- [ ] **Step 5: Mutation** — publish unconditionally. Input class: *a stop that arrives while a start is in flight*. The new test must fail by name.

---

## Task 2: Rename `PrimeVueTreeNode` to `ScriptTreeNode`

`GET /api/v1/scripts` publishes its response type as `PrimeVueTreeNode` — a name from a UI component library, with field names (`key`, `label`, `leaf`) that are the PrimeVue `Tree` component's props rather than anything about scripts. **Rename it now:** the OpenAPI snapshot is created two tasks from here and every consumer of the type is being rewritten in this plan anyway, so the rename costs one commit today and becomes a gratuitous breaking API change if it waits until the plan that removes PrimeVue.

Only the schema name moves. The four fields keep their names, so the wire format is byte-identical and no client has to change behaviour — just the identifier it imports.

**Files:**
- Modify: `crates/core/src/types.rs:1149-1165`
- Modify: `crates/server/src/manage/scripts.rs:8,165,174,213`
- Modify: `app/src/components/ScriptTree.vue:5,7,18,20,35,39`
- Modify: `app/src/store/scriptStore.ts:3,43`
- Modify: `app/src-tauri/src/gui/command/script.rs:7,102,132`
- Modify: `app/src-tauri/build.rs:93` — **another agent holds uncommitted changes in this file**
- Modify: `crates/server/tests/manage_scripts.rs` (only if it names the type)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - Rust: `factorio_bot_core::types::ScriptTreeNode { key: String, label: String, leaf: bool, children: Vec<ScriptTreeNode> }`, still deriving `TypeScriptify` and `utoipa::ToSchema`, still carrying `#[schema(no_recursion)]` on `children`.
  - TypeScript: `export type ScriptTreeNode = { key: string; label: string; leaf: boolean; children: ScriptTreeNode [] }` in the generated `app/src/models/types.ts`. Tasks 4 and 10 import this name.
  - OpenAPI: `components.schemas.ScriptTreeNode`; `GET /api/v1/scripts` responds `array` of it.

**Do not drop `#[schema(no_recursion)]`.** The type is self-referential, and the attribute's comment in `types.rs` says exactly what happens without it: utoipa's schema generation recurses forever and **aborts the process with a stack overflow** the first time any route referencing the type builds its schema. A rename that loses the attribute turns `/openapi.json` into a crash.

**`app/src-tauri/build.rs` has another agent's uncommitted changes.** Read the working-tree file, change only the single `PrimeVueTreeNode::type_script_ify()` token on line 93, and leave everything else exactly as you found it. If the file no longer contains that call, the other agent's work has moved it — find where it went rather than reinstating the old shape.

**Note in passing:** this type reaches the browser through *two* independent mechanisms — `typescript_definitions` (via the build script, into `models/types.ts`) and `utoipa` (into `/openapi.json`). Neither knows about the other. That duplication is not resolved here; Task 5's contract test is what stops them drifting apart unnoticed.

- [ ] **Step 1: Write the failing test**

Append to `crates/server/tests/openapi.rs`:

```rust
/// The script listing's response type is named for what it is, not for the
/// UI library that happened to consume it first. A published schema name is
/// part of the API contract, so this is what stops it drifting back.
#[tokio::test]
async fn the_script_listing_publishes_a_script_tree_node_schema() {
    let spec = openapi_spec().await;
    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("schemas object");
    assert!(
        schemas.contains_key("ScriptTreeNode"),
        "spec has no ScriptTreeNode schema: {:?}",
        schemas.keys().collect::<Vec<_>>()
    );
    assert!(
        !schemas.contains_key("PrimeVueTreeNode"),
        "the old ui-library-shaped name is still published"
    );
    let properties = schemas["ScriptTreeNode"]["properties"]
        .as_object()
        .expect("ScriptTreeNode properties");
    for field in ["key", "label", "leaf", "children"] {
        assert!(
            properties.contains_key(field),
            "the wire format must not change: {field} is missing"
        );
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test openapi the_script_listing_publishes'
```

Expected: FAIL — `spec has no ScriptTreeNode schema`.

- [ ] **Step 3: Rename the Rust type**

In `crates/core/src/types.rs`, replace the struct with:

```rust
#[derive(
    Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct ScriptTreeNode {
    pub key: String,
    pub label: String,
    pub leaf: bool,
    // Self-referential (`ScriptTreeNode` -> `ScriptTreeNode`): without
    // `no_recursion`, utoipa's OpenAPI schema generation recurses into this
    // field forever and aborts the process with a stack overflow the first
    // time any route referencing this type builds its schema.
    #[schema(no_recursion)]
    pub children: Vec<ScriptTreeNode>,
}
```

Then update the four call sites in `crates/server/src/manage/scripts.rs` (the import on line 8, the `body = Vec<PrimeVueTreeNode>` in the `#[utoipa::path]` on line 165, the return type on line 174, and the struct literal on line 213) and the three in `app/src-tauri/src/gui/command/script.rs` (lines 7, 102, 132 — this file is deleted in Task 14, but it has to compile until then).

Verify no Rust references remain:

```bash
grep -rn "PrimeVueTreeNode" crates app/src-tauri
```

Expected: only `app/src-tauri/build.rs:93`, handled in the next step.

- [ ] **Step 4: Update the type generator, coordinating with the other agent**

Re-read `app/src-tauri/build.rs` first:

```bash
git diff app/src-tauri/build.rs
grep -n "PrimeVueTreeNode" app/src-tauri/build.rs
```

Change only that one token:

```rust
  output += &ScriptTreeNode::type_script_ify();
```

- [ ] **Step 5: Regenerate the TypeScript types**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --all-features'
git diff app/src/models/types.ts
```

Expected diff: exactly one line, `PrimeVueTreeNode` → `ScriptTreeNode` on both sides of `export type ScriptTreeNode = { … children: ScriptTreeNode [] }`. If more lines moved, another type changed underneath you — inspect before continuing.

- [ ] **Step 6: Update the two frontend consumers**

In `app/src/components/ScriptTree.vue` and `app/src/store/scriptStore.ts`, replace every `PrimeVueTreeNode` with `ScriptTreeNode` (six occurrences in the component, two in the store). The store is rewritten wholesale in Task 10 and the code there already uses the new name; this step keeps the tree compiling in between.

```bash
grep -rn "PrimeVueTreeNode" app/src crates
```

Expected: no output.

- [ ] **Step 7: Run the tests and watch them pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server && cd app && pnpm run lint'
```

Expected: PASS, including the pre-existing `manage_scripts` suite — the JSON bodies it asserts on are unchanged, which is the point.

- [ ] **Step 8: Prove `no_recursion` is still load-bearing (mutation)**

Delete the `#[schema(no_recursion)]` attribute and run `cargo test -p factorio-bot-server --test openapi`. Expect a stack overflow or an abort rather than a test failure. Record it, then restore the attribute.

- [ ] **Step 9: Lint and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all && cargo fmt --all -- --check && cargo clippy --workspace --all-features --all-targets -- --deny warnings'
git add crates/core/src/types.rs crates/server app/src-tauri/build.rs app/src-tauri/src/gui/command/script.rs app/src/models/types.ts app/src/components/ScriptTree.vue app/src/store/scriptStore.ts
git commit -m "refactor!: rename PrimeVueTreeNode to ScriptTreeNode in the api schema"
```

---

## Task 3: The HTTP transport core

**Files:**
- Create: `app/src/api/http.ts`
- Create: `app/src/api/http.spec.ts`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `export function apiBase(): string`
  - `export function buildUrl(path: string, query?: Record<string, QueryValue>): string` where `type QueryValue = string | number | boolean | undefined`
  - `export class ApiError extends Error { readonly status: number; readonly code: number | null; readonly body: unknown }`
  - `export interface RequestOptions { method?: string; query?: Record<string, QueryValue>; body?: unknown; signal?: AbortSignal }`
  - `export async function request<T>(path: string, options?: RequestOptions): Promise<T>`

- [ ] **Step 1: Write the failing tests**

`app/src/api/http.spec.ts`:

```ts
import {afterEach, describe, expect, it, vi} from 'vitest';
import {ApiError, buildUrl, request} from './http';

function jsonResponse(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), {
        status,
        headers: {'Content-Type': 'application/json'}
    });
}

afterEach(() => {
    vi.unstubAllGlobals();
});

describe('buildUrl', () => {
    it('drops undefined query values instead of sending the string "undefined"', () => {
        expect(buildUrl('/api/v1/scripts', {path: '/a.lua', bot_count: undefined}))
            .toBe('/api/v1/scripts?path=%2Fa.lua');
    });

    it('encodes reserved characters in query values', () => {
        expect(buildUrl('/api/v1/fs/exists', {path: '/tmp/a b&c'}))
            .toBe('/api/v1/fs/exists?path=%2Ftmp%2Fa+b%26c');
    });

    it('omits the question mark when there is no query', () => {
        expect(buildUrl('/api/v1/settings')).toBe('/api/v1/settings');
    });
});

describe('request', () => {
    it('returns undefined for a 204 rather than choking on an empty body', async () => {
        vi.stubGlobal('fetch', vi.fn(async () => new Response(null, {status: 204})));
        await expect(request<void>('/api/v1/rcon', {method: 'POST', body: {command: '/x'}}))
            .resolves.toBeUndefined();
    });

    it('sends the body as JSON with a content-type', async () => {
        const fetchMock = vi.fn(async () => new Response(null, {status: 204}));
        vi.stubGlobal('fetch', fetchMock);
        await request<void>('/api/v1/rcon', {method: 'POST', body: {command: '/x'}});
        const init = fetchMock.mock.calls[0][1] as RequestInit;
        expect(init.method).toBe('POST');
        expect(init.body).toBe('{"command":"/x"}');
        expect((init.headers as Record<string, string>)['Content-Type']).toBe('application/json');
    });

    it('throws an ApiError carrying the servers message field', async () => {
        vi.stubGlobal('fetch', vi.fn(async () => jsonResponse(400, {message: 'not a file: /x', code: 1})));
        const error = await request('/api/v1/scripts/file', {query: {path: '/x'}}).catch(e => e);
        expect(error).toBeInstanceOf(ApiError);
        expect((error as ApiError).status).toBe(400);
        expect((error as ApiError).code).toBe(1);
        expect((error as ApiError).message).toBe('not a file: /x');
    });

    it('accepts the alternate error key so a 409 from the job registry is readable', async () => {
        // crates/server/src/error.rs serialises `message`; the script-execution
        // plan documents its 409 body with `error`. Neither spelling may
        // degrade to a bare "409 Conflict" in the UI.
        vi.stubGlobal('fetch', vi.fn(async () => jsonResponse(409, {
            error: 'a script is already running',
            code: 409,
            running_job_id: '2'
        })));
        const error = await request('/api/v1/scripts/execute', {method: 'POST', body: {}}).catch(e => e);
        expect((error as ApiError).message).toBe('a script is already running');
        expect(((error as ApiError).body as Record<string, unknown>).running_job_id).toBe('2');
    });

    it('falls back to the status line when the error body is not JSON', async () => {
        vi.stubGlobal('fetch', vi.fn(async () => new Response('<html>502</html>', {status: 502})));
        const error = await request('/api/v1/settings').catch(e => e);
        expect((error as ApiError).status).toBe(502);
        expect((error as ApiError).message).toContain('502');
    });

    it('propagates a network failure rather than swallowing it', async () => {
        vi.stubGlobal('fetch', vi.fn(async () => {
            throw new TypeError('Failed to fetch');
        }));
        await expect(request('/api/v1/settings')).rejects.toThrow('Failed to fetch');
    });
});
```

- [ ] **Step 2: Run the tests and watch them fail**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/api/http.spec.ts'
```

Expected: FAIL — cannot resolve `./http`.

- [ ] **Step 3: Write the implementation**

`app/src/api/http.ts`:

```ts
export type QueryValue = string | number | boolean | undefined;

/**
 * Same-origin in production (the axum server serves both the SPA and the API),
 * overridable for a Vite dev server that is not proxying. The dev proxy in
 * vite.config.mts is the normal path, so this is empty almost always.
 */
export function apiBase(): string {
    return (import.meta.env.VITE_API_BASE as string | undefined) ?? '';
}

export function buildUrl(path: string, query?: Record<string, QueryValue>): string {
    let url = apiBase() + path;
    if (query) {
        const params = new URLSearchParams();
        for (const [key, value] of Object.entries(query)) {
            if (value !== undefined) {
                params.append(key, String(value));
            }
        }
        const search = params.toString();
        if (search.length > 0) {
            url += '?' + search;
        }
    }
    return url;
}

/** A non-2xx answer from the API, with the server's own message if it sent one. */
export class ApiError extends Error {
    readonly status: number;
    readonly code: number | null;
    readonly body: unknown;

    constructor(status: number, message: string, code: number | null, body: unknown) {
        super(message);
        this.name = 'ApiError';
        this.status = status;
        this.code = code;
        this.body = body;
    }
}

export interface RequestOptions {
    method?: string;
    query?: Record<string, QueryValue>;
    body?: unknown;
    signal?: AbortSignal;
}

async function errorFromResponse(response: Response): Promise<ApiError> {
    let body: unknown = null;
    let message = (response.status + ' ' + response.statusText).trim();
    let code: number | null = null;

    const text = await response.text().catch(() => '');
    if (text.length > 0) {
        try {
            body = JSON.parse(text);
        } catch {
            body = text;
        }
    }

    if (body !== null && typeof body === 'object') {
        const record = body as Record<string, unknown>;
        // crates/server/src/error.rs serialises { message, code }; the job
        // registry's 409 is documented with { error, code, running_job_id }.
        // Accept either rather than showing the user a bare status line.
        const detail = record.message ?? record.error;
        if (typeof detail === 'string' && detail.length > 0) {
            message = detail;
        }
        if (typeof record.code === 'number') {
            code = record.code;
        }
    } else if (typeof body === 'string' && body.length > 0 && body.length < 200) {
        message = body;
    }

    return new ApiError(response.status, message, code, body);
}

export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const {method = 'GET', query, body, signal} = options;
    const headers: Record<string, string> = {Accept: 'application/json'};
    const init: RequestInit = {method, signal, headers};
    if (body !== undefined) {
        headers['Content-Type'] = 'application/json';
        init.body = JSON.stringify(body);
    }

    const response = await fetch(buildUrl(path, query), init);
    if (!response.ok) {
        throw await errorFromResponse(response);
    }
    // 204 is the success answer of every mutation route in this API, and
    // `response.json()` on an empty body throws.
    if (response.status === 204) {
        return undefined as T;
    }
    const text = await response.text();
    if (text.length === 0) {
        return undefined as T;
    }
    return JSON.parse(text) as T;
}
```

- [ ] **Step 4: Run the tests and watch them pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/api/http.spec.ts'
```

Expected: 9 passed.

- [ ] **Step 5: Lint and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint'
git add app/src/api/http.ts app/src/api/http.spec.ts
git commit -m "feat(app): add the fetch transport that replaces tauri invoke"
```

---

## Task 4: Typed DTOs and the route client

**Files:**
- Create: `app/src/api/types.ts`
- Create: `app/src/api/client.ts`
- Create: `app/src/api/client.spec.ts`
- Modify: `app/src/models/settings.ts`

**Interfaces:**
- Consumes: `request`, `buildUrl`, `ApiError`, `RequestOptions` from `@/api/http` (Task 3); `ScriptTreeNode`, `FactorioSettings`, `RestApiSettings` from `@/models/types` (generated).
- Produces, from `@/api/types`:
  - `InstanceStatus { started: boolean; starting: boolean; client_count: number; server_port: number | null; rcon_port: number | null; last_error: string | null }`
  - `StartAccepted { accepted: boolean }`
  - `ScriptContent { code: string }`
  - `ExistsResponse { exists: boolean }`
  - `ExecuteRequest { path?: string; code?: string; language?: string; bot_count?: number }`
  - `ExecuteAccepted { job_id: string }`
  - `JobStatus = 'running' | 'succeeded' | 'failed'`
  - `Job { id: string; script: string | null; status: JobStatus; started_at_ms: number; finished_at_ms: number | null; stdout: string; stderr: string; error: string | null }`
- Produces, from `@/api/client` (all `async`):
  - `getSettings(): Promise<AppSettings>`
  - `putSettings(settings: AppSettings): Promise<AppSettings>`
  - `getInstance(): Promise<InstanceStatus>`
  - `startInstance(): Promise<StartAccepted>`
  - `stopInstance(): Promise<void>`
  - `sendRcon(command: string): Promise<void>`
  - `listScripts(path: string): Promise<ScriptTreeNode[]>`
  - `readScript(path: string): Promise<ScriptContent>`
  - `writeScript(path: string, code: string): Promise<void>`
  - `createScript(path: string, code: string): Promise<void>`
  - `deleteScript(path: string): Promise<void>`
  - `pathExists(path: string): Promise<ExistsResponse>`
  - `executeScript(body: ExecuteRequest): Promise<ExecuteAccepted>`
  - `listJobs(): Promise<Job[]>`
  - `getJob(id: string): Promise<Job>`
  - `jobEventsUrl(id: string): string` (synchronous)
- Produces, from `@/models/settings`: `AppSettings { gui: GuiSettings; restapi: RestApiSettings; factorio: FactorioSettings }`

- [ ] **Step 1: Write the failing tests**

`app/src/api/client.spec.ts`:

```ts
import {afterEach, describe, expect, it, vi} from 'vitest';
import * as client from './client';

function capture(status = 200, body: unknown = {}) {
    const fetchMock = vi.fn(async () => new Response(
        status === 204 ? null : JSON.stringify(body),
        {status, headers: {'Content-Type': 'application/json'}}
    ));
    vi.stubGlobal('fetch', fetchMock);
    return fetchMock;
}

afterEach(() => {
    vi.unstubAllGlobals();
});

describe('client', () => {
    it('reads settings from GET /api/v1/settings', async () => {
        const fetchMock = capture(200, {gui: {}, restapi: {}, factorio: {}});
        await client.getSettings();
        expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/settings');
        expect((fetchMock.mock.calls[0][1] as RequestInit).method).toBe('GET');
    });

    it('sends the whole settings object to PUT /api/v1/settings', async () => {
        const fetchMock = capture(200, {gui: {}, restapi: {}, factorio: {}});
        await client.putSettings({gui: {enable_autostart: false, enable_restapi: true}} as never);
        const init = fetchMock.mock.calls[0][1] as RequestInit;
        expect(init.method).toBe('PUT');
        expect(init.body).toContain('enable_autostart');
    });

    it('starts the instance with POST /api/v1/instance/start', async () => {
        const fetchMock = capture(202, {accepted: true});
        await expect(client.startInstance()).resolves.toEqual({accepted: true});
        expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/instance/start');
    });

    it('stops the instance with POST /api/v1/instance/stop', async () => {
        const fetchMock = capture(204);
        await client.stopInstance();
        expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/instance/stop');
        expect((fetchMock.mock.calls[0][1] as RequestInit).method).toBe('POST');
    });

    it('wraps an rcon command in the servers body shape', async () => {
        const fetchMock = capture(204);
        await client.sendRcon('/server-save');
        expect((fetchMock.mock.calls[0][1] as RequestInit).body).toBe('{"command":"/server-save"}');
    });

    it('passes the tree key straight through as the path query parameter', async () => {
        const fetchMock = capture(200, []);
        await client.listScripts('/sub');
        expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/scripts?path=%2Fsub');
    });

    it('writes a script with the path in the query and the code in the body', async () => {
        const fetchMock = capture(204);
        await client.writeScript('/a.lua', 'print(1)');
        expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/scripts/file?path=%2Fa.lua');
        const init = fetchMock.mock.calls[0][1] as RequestInit;
        expect(init.method).toBe('PUT');
        expect(init.body).toBe('{"code":"print(1)"}');
    });

    it('posts an execution request and returns the job id', async () => {
        const fetchMock = capture(202, {job_id: '7'});
        await expect(client.executeScript({path: '/a.lua'})).resolves.toEqual({job_id: '7'});
        expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/scripts/execute');
    });

    it('escapes the job id in the events url', () => {
        expect(client.jobEventsUrl('7')).toBe('/api/v1/jobs/7/events');
    });
});
```

- [ ] **Step 2: Run the tests and watch them fail**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/api/client.spec.ts'
```

Expected: FAIL — cannot resolve `./client`.

- [ ] **Step 3: Write the DTOs**

`app/src/api/types.ts`:

```ts
// Hand-written mirrors of the server types that are NOT generated into
// models/types.ts (those come from crates/core; these live in crates/server).
// app/src/api/openapi.contract.spec.ts is what keeps them honest.

export interface InstanceStatus {
    started: boolean;
    /** A start accepted by POST /api/v1/instance/start is still running. */
    starting: boolean;
    client_count: number;
    server_port: number | null;
    rcon_port: number | null;
    last_error: string | null;
}

export interface StartAccepted {
    accepted: boolean;
}

export interface ScriptContent {
    code: string;
}

export interface ExistsResponse {
    exists: boolean;
}

/** Exactly one of `path` or `code` must be set. */
export interface ExecuteRequest {
    path?: string;
    code?: string;
    language?: string;
    bot_count?: number;
}

export interface ExecuteAccepted {
    job_id: string;
}

export type JobStatus = 'running' | 'succeeded' | 'failed';

export interface Job {
    /** A decimal counter, serialised as a string so it never loses precision. */
    id: string;
    script: string | null;
    status: JobStatus;
    started_at_ms: number;
    finished_at_ms: number | null;
    stdout: string;
    stderr: string;
    error: string | null;
}

export type OutputStream = 'stdout' | 'stderr';
```

- [ ] **Step 4: Type the AppSettings model properly**

Replace the whole of `app/src/models/settings.ts`:

```ts
import {FactorioSettings, RestApiSettings} from '@/models/types';

export type AppSettings = {
    gui: GuiSettings,
    restapi: RestApiSettings,
    factorio: FactorioSettings,
}

export type GuiSettings = {
    enable_autostart: boolean,
    enable_restapi: boolean,
}
```

- [ ] **Step 5: Write the client**

`app/src/api/client.ts`:

```ts
import {buildUrl, request} from './http';
import {AppSettings} from '@/models/settings';
import {ScriptTreeNode} from '@/models/types';
import {
    ExecuteAccepted,
    ExecuteRequest,
    ExistsResponse,
    InstanceStatus,
    Job,
    ScriptContent,
    StartAccepted
} from './types';

export function getSettings(): Promise<AppSettings> {
    return request<AppSettings>('/api/v1/settings');
}

export function putSettings(settings: AppSettings): Promise<AppSettings> {
    return request<AppSettings>('/api/v1/settings', {method: 'PUT', body: settings});
}

export function getInstance(): Promise<InstanceStatus> {
    return request<InstanceStatus>('/api/v1/instance');
}

export function startInstance(): Promise<StartAccepted> {
    return request<StartAccepted>('/api/v1/instance/start', {method: 'POST'});
}

export function stopInstance(): Promise<void> {
    return request<void>('/api/v1/instance/stop', {method: 'POST'});
}

export function sendRcon(command: string): Promise<void> {
    return request<void>('/api/v1/rcon', {method: 'POST', body: {command}});
}

export function listScripts(path: string): Promise<ScriptTreeNode[]> {
    return request<ScriptTreeNode[]>('/api/v1/scripts', {query: {path}});
}

export function readScript(path: string): Promise<ScriptContent> {
    return request<ScriptContent>('/api/v1/scripts/file', {query: {path}});
}

export function writeScript(path: string, code: string): Promise<void> {
    return request<void>('/api/v1/scripts/file', {method: 'PUT', query: {path}, body: {code}});
}

/** Not wired to any UI yet; the route exists and a later plan needs it. */
export function createScript(path: string, code: string): Promise<void> {
    return request<void>('/api/v1/scripts/file', {method: 'POST', query: {path}, body: {code}});
}

/** Not wired to any UI yet; the route exists and a later plan needs it. */
export function deleteScript(path: string): Promise<void> {
    return request<void>('/api/v1/scripts/file', {method: 'DELETE', query: {path}});
}

export function pathExists(path: string): Promise<ExistsResponse> {
    return request<ExistsResponse>('/api/v1/fs/exists', {query: {path}});
}

export function executeScript(body: ExecuteRequest): Promise<ExecuteAccepted> {
    return request<ExecuteAccepted>('/api/v1/scripts/execute', {method: 'POST', body});
}

export function listJobs(): Promise<Job[]> {
    return request<Job[]>('/api/v1/jobs');
}

export function getJob(id: string): Promise<Job> {
    return request<Job>('/api/v1/jobs/' + encodeURIComponent(id));
}

/** EventSource takes a URL, not a fetch call, so this is a URL builder. */
export function jobEventsUrl(id: string): string {
    return buildUrl('/api/v1/jobs/' + encodeURIComponent(id) + '/events');
}
```

- [ ] **Step 6: Run the tests and watch them pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/api/'
```

Expected: 18 passed across `http.spec.ts` and `client.spec.ts`.

- [ ] **Step 7: Lint and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint'
git add app/src/api app/src/models/settings.ts
git commit -m "feat(app): add a typed http client for every management route"
```

---

## Task 5: Pin the client to the OpenAPI spec

The spec is the source of truth; this is how the hand-written client stays honest against it without a codegen toolchain.

**Files:**
- Create: `app/src/api/openapi.snapshot.json`
- Create: `app/src/api/openapi.contract.spec.ts`
- Modify: `app/package.json`
- Modify: `app/eslint.config.mjs`

**Interfaces:**
- Consumes: the sixteen client functions from `@/api/client` (Task 4); `POST /api/v1/instance/start` from Task 1; plan 4's `/api/v1/scripts/execute`, `/api/v1/jobs`, `/api/v1/jobs/{id}`, `/api/v1/jobs/{id}/events`.
- Produces: `app/src/api/openapi.snapshot.json` (committed), and the pnpm script `openapi:snapshot` that regenerates it.

- [ ] **Step 1: Add the snapshot script**

In `app/package.json`, add to `"scripts"`:

```json
    "openapi:snapshot": "curl -sSf http://127.0.0.1:7492/openapi.json -o src/api/openapi.snapshot.json && node -e \"const f='src/api/openapi.snapshot.json';const j=JSON.parse(require('fs').readFileSync(f,'utf8'));require('fs').writeFileSync(f,JSON.stringify(j,null,2)+'\\n')\"",
```

In `app/eslint.config.mjs`, extend the ignore list so the snapshot is never linted:

```js
    globalIgnores(['**/*.d.ts', 'src/models/types.ts', 'src/api/openapi.snapshot.json']),
```

- [ ] **Step 2: Generate the snapshot**

In one terminal:

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo run --no-default-features --features cli,lua,restapi -- serve --bind 127.0.0.1:7492'
```

In another:

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run openapi:snapshot'
```

Then stop the server with Ctrl-C. Confirm the file exists and contains `"/api/v1/instance/start"`:

```bash
grep -c 'instance/start' app/src/api/openapi.snapshot.json
```

Expected: `1` or more. If it is `0`, Task 1 has not landed — stop and fix that first.

- [ ] **Step 3: Write the contract test**

`app/src/api/openapi.contract.spec.ts`:

```ts
import {describe, expect, it} from 'vitest';
import spec from './openapi.snapshot.json';

/**
 * Every (path, method) that app/src/api/client.ts calls. Kept as data rather
 * than derived from the client, because a typo in the client would otherwise
 * be copied faithfully into the expectation and the test would pass.
 *
 * Regenerate the snapshot with `pnpm run openapi:snapshot` against a running
 * `factorio-bot serve`. When this test fails, the server moved and the client
 * has to follow — never the other way round.
 */
const USED: ReadonlyArray<readonly [string, string]> = [
    ['/api/v1/settings', 'get'],
    ['/api/v1/settings', 'put'],
    ['/api/v1/instance', 'get'],
    ['/api/v1/instance/start', 'post'],
    ['/api/v1/instance/stop', 'post'],
    ['/api/v1/rcon', 'post'],
    ['/api/v1/scripts', 'get'],
    ['/api/v1/scripts/file', 'get'],
    ['/api/v1/scripts/file', 'put'],
    ['/api/v1/scripts/file', 'post'],
    ['/api/v1/scripts/file', 'delete'],
    ['/api/v1/fs/exists', 'get'],
    ['/api/v1/scripts/execute', 'post'],
    ['/api/v1/jobs', 'get'],
    ['/api/v1/jobs/{id}', 'get'],
    ['/api/v1/jobs/{id}/events', 'get']
];

const paths = (spec as {paths: Record<string, Record<string, unknown>>}).paths;

describe('the committed openapi snapshot', () => {
    it.each(USED)('serves %s %s', (path, method) => {
        expect(paths[path], 'spec has no path ' + path).toBeDefined();
        expect(paths[path][method], 'spec has no ' + method.toUpperCase() + ' ' + path).toBeDefined();
    });

    it('publishes path as a query parameter on the script routes', () => {
        // utoipa infers parameter_in from the argument type; this crate's
        // ApiQuery wrapper defeats that inference and the default is `path`.
        // A regression there would make every script URL in the client wrong.
        for (const route of ['/api/v1/scripts', '/api/v1/scripts/file', '/api/v1/fs/exists']) {
            const parameters = (paths[route].get as {parameters: Array<{name: string, in: string}>}).parameters;
            const pathParam = parameters.find(p => p.name === 'path');
            expect(pathParam, route + ' publishes no path parameter').toBeDefined();
            expect(pathParam?.in).toBe('query');
        }
    });

    it('describes the instance status fields the ui polls', () => {
        const schema = (spec as {components: {schemas: Record<string, {properties: Record<string, unknown>}>}})
            .components.schemas.InstanceStatus;
        for (const field of ['started', 'starting', 'client_count', 'server_port', 'rcon_port', 'last_error']) {
            expect(schema.properties[field], 'InstanceStatus has no ' + field).toBeDefined();
        }
    });
});
```

- [ ] **Step 4: Allow JSON module imports**

Confirm `app/tsconfig.json` has `"resolveJsonModule": true`; if it does not, add it under `compilerOptions`. Vite resolves JSON imports natively, so no plugin is needed.

- [ ] **Step 5: Run the test and watch it pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/api/openapi.contract.spec.ts'
```

Expected: PASS. A failure on any `/api/v1/jobs*` row means plan 4 has not landed yet — stop and wait for it rather than deleting the row.

- [ ] **Step 6: Lint and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint'
git add app/src/api app/package.json app/eslint.config.mjs app/tsconfig.json
git commit -m "test(app): pin the http client to a committed openapi snapshot"
```

---

## Task 6: `appStore` over HTTP

**Files:**
- Modify: `app/src/store/appStore.ts`
- Create: `app/src/store/appStore.spec.ts`

**Interfaces:**
- Consumes: `getSettings`, `putSettings`, `pathExists` from `@/api/client` (Task 4); `AppSettings` from `@/models/settings` (Task 4).
- Produces (the store's public surface, which views depend on):
  - unchanged getters: `getSettings`, `getWorkspacePath`, `getRecreateLevel`, `getEnableAutostart`, `getRestapiPort`, `getFactorioArchivePath`, `getClientCount`, `getMapExchangeString`, `getSeed`
  - unchanged actions: `loadSettings(): Promise<AppSettings | null>`, `fileExists(path: string): Promise<boolean>`, `updateWorkspacePath`, `updateFactorioArchivePath`, `updateRecreateLevel`, `updateEnableAutostart`, `updateRestapiPort`, `updateClientCount`, `updateMapExchangeString`, `updateSeed`
  - **removed:** `maximizeWindow`, `openInBrowser`, `updateEnableRestApi`, getter `getEnableRestapi`

- [ ] **Step 1: Write the failing tests**

`app/src/store/appStore.spec.ts`:

```ts
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useAppStore} from './appStore';
import * as client from '@/api/client';
import {AppSettings} from '@/models/settings';

vi.mock('@/api/client');

function settingsFixture(): AppSettings {
    return {
        gui: {enable_autostart: false, enable_restapi: true},
        restapi: {port: 7492, web_root: null},
        factorio: {
            client_count: 2,
            factorio_archive_path: '/tmp/factorio.tar.xz',
            map_exchange_string: '',
            rcon_pass: 'foobar',
            rcon_port: 4321,
            recreate: false,
            restapi_port: 1234,
            seed: '',
            workspace_path: '/tmp/workspace'
        }
    };
}

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

describe('appStore', () => {
    it('loads settings from the api and exposes them through the getters', async () => {
        vi.mocked(client.getSettings).mockResolvedValue(settingsFixture());
        const store = useAppStore();
        await store.loadSettings();
        expect(store.getWorkspacePath).toBe('/tmp/workspace');
        expect(store.getClientCount).toBe(2);
    });

    it('persists a changed workspace path with PUT and keeps the servers answer', async () => {
        vi.mocked(client.getSettings).mockResolvedValue(settingsFixture());
        const saved = settingsFixture();
        saved.factorio.workspace_path = '/srv/workspace';
        vi.mocked(client.putSettings).mockResolvedValue(saved);

        const store = useAppStore();
        await store.loadSettings();
        await store.updateWorkspacePath('/srv/workspace');

        expect(client.putSettings).toHaveBeenCalledTimes(1);
        // The server is authoritative: it may normalise what it was sent.
        expect(store.getWorkspacePath).toBe('/srv/workspace');
    });

    it('does not call the api when there are no settings loaded yet', async () => {
        const store = useAppStore();
        await store.updateSeed('12345');
        expect(client.putSettings).not.toHaveBeenCalled();
    });

    it('answers fileExists from the servers filesystem, not the browsers', async () => {
        vi.mocked(client.pathExists).mockResolvedValue({exists: true});
        const store = useAppStore();
        await expect(store.fileExists('/tmp/workspace')).resolves.toBe(true);
        expect(client.pathExists).toHaveBeenCalledWith('/tmp/workspace');
    });

    it('reports a failed load instead of leaving the app on stale state', async () => {
        vi.mocked(client.getSettings).mockRejectedValue(new Error('boom'));
        const store = useAppStore();
        await expect(store.loadSettings()).rejects.toThrow('boom');
        expect(store.settings).toBeNull();
    });
});
```

- [ ] **Step 2: Run the tests and watch them fail**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/store/appStore.spec.ts'
```

Expected: FAIL — `store.fileExists` still calls `invoke`, which is undefined under vitest.

- [ ] **Step 3: Rewrite the store**

Replace the whole of `app/src/store/appStore.ts`:

```ts
import {defineStore} from 'pinia';
import {AppSettings} from '@/models/settings';
import {getSettings, pathExists, putSettings} from '@/api/client';

export const useAppStore = defineStore('app', {
    state: () => ({
        settings: null as AppSettings | null
    }),
    getters: {
        getSettings(): AppSettings | null {
            return this.settings;
        },
        getWorkspacePath(): string | null {
            return this.settings ? this.settings.factorio.workspace_path : null;
        },
        getRecreateLevel(): boolean | undefined {
            return this.settings ? this.settings.factorio.recreate : undefined;
        },
        getEnableAutostart(): boolean | null {
            return this.settings ? this.settings.gui.enable_autostart : null;
        },
        getRestapiPort(): number | null {
            return this.settings ? this.settings.restapi.port : null;
        },
        getFactorioArchivePath(): string | null {
            return this.settings ? this.settings.factorio.factorio_archive_path : null;
        },
        getClientCount(): number | null {
            return this.settings ? this.settings.factorio.client_count : null;
        },
        getMapExchangeString(): string | null {
            return this.settings ? this.settings.factorio.map_exchange_string : null;
        },
        getSeed(): string | null {
            return this.settings ? this.settings.factorio.seed : null;
        }
    },
    actions: {
        /**
         * Checks a path on the *server's* filesystem. A browser cannot see it,
         * which is also why the native file pickers had to go.
         */
        async fileExists(path: string): Promise<boolean> {
            const response = await pathExists(path);
            return response.exists;
        },
        async loadSettings(): Promise<AppSettings | null> {
            this.settings = await getSettings();
            return this.settings;
        },
        /**
         * PUT returns the persisted settings; adopting the answer rather than
         * keeping the local object means a server-side normalisation is
         * visible immediately instead of at the next reload.
         */
        async _updateSettings() {
            if (this.settings === null) {
                return;
            }
            this.settings = await putSettings(this.settings);
        },
        async updateWorkspacePath(workspacePath: string) {
            if (this.settings !== null) {
                this.settings.factorio.workspace_path = workspacePath;
                await this._updateSettings();
            }
        },
        async updateFactorioArchivePath(factorioArchivePath: string) {
            if (this.settings !== null) {
                this.settings.factorio.factorio_archive_path = factorioArchivePath;
                await this._updateSettings();
            }
        },
        async updateRecreateLevel(recreateLevel: boolean) {
            if (this.settings !== null) {
                this.settings.factorio.recreate = recreateLevel;
                await this._updateSettings();
            }
        },
        async updateEnableAutostart(enableAutostart: boolean) {
            if (this.settings !== null) {
                this.settings.gui.enable_autostart = enableAutostart;
                await this._updateSettings();
            }
        },
        /**
         * Persisted only. The bind port is chosen when `factorio-bot serve`
         * starts; changing it here takes effect on the next server start.
         */
        async updateRestapiPort(restapiPort: number) {
            if (this.settings !== null) {
                this.settings.restapi.port = restapiPort;
                await this._updateSettings();
            }
        },
        async updateClientCount(clientCount: number) {
            if (this.settings !== null) {
                this.settings.factorio.client_count = clientCount;
                await this._updateSettings();
            }
        },
        async updateMapExchangeString(mapExchangeString: string) {
            if (this.settings !== null) {
                this.settings.factorio.map_exchange_string = mapExchangeString;
                await this._updateSettings();
            }
        },
        async updateSeed(seed: string) {
            if (this.settings !== null) {
                this.settings.factorio.seed = seed;
                await this._updateSettings();
            }
        }
    }
});
```

- [ ] **Step 4: Run the tests and watch them pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/store/appStore.spec.ts'
```

Expected: 5 passed. `pnpm run lint` will still fail at this point because `SettingsPage.vue` and `App.vue` reference the removed actions — Task 11 fixes that. Do not chase it here.

- [ ] **Step 5: Commit**

```bash
git add app/src/store/appStore.ts app/src/store/appStore.spec.ts
git commit -m "refactor(app): move appStore onto the settings and fs http routes"
```

---

## Task 7: `instanceStore` over HTTP

**Files:**
- Modify: `app/src/store/instanceStore.ts`
- Create: `app/src/store/instanceStore.spec.ts`

**Interfaces:**
- Consumes: `getInstance`, `startInstance`, `stopInstance` from `@/api/client` (Task 4); `InstanceStatus` from `@/api/types` (Task 4); `useAppStore` from `@/store/appStore` (Task 6).
- Produces:
  - state: `starting: boolean`, `stopping: boolean`, `started: boolean`, `failed: boolean`, `lastError: string | null`, `clientCount: number`
  - getters: `isStarting`, `isStopping`, `isFailed`, `isStarted`, `getLastError`
  - actions: `checkInstanceState(): Promise<boolean>`, `startInstances(): Promise<void>`, `stopInstances(): Promise<void>`

**Design note for the implementer:** `startInstances` does **not** wait for Factorio to be up. `POST /api/v1/instance/start` answers `202` because a first-run archive extraction takes 8-10 minutes. The store sets `starting = true` and returns; `App.vue` (Task 11) polls `checkInstanceState()` every two seconds and that is what flips `starting → started` or surfaces `lastError`. Do not add a polling loop *inside* the store — a store that owns a timer is a store that leaks one in every test.

- [ ] **Step 1: Write the failing tests**

`app/src/store/instanceStore.spec.ts`:

```ts
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useInstanceStore} from './instanceStore';
import {useAppStore} from './appStore';
import * as client from '@/api/client';
import {ApiError} from '@/api/http';
import {InstanceStatus} from '@/api/types';

vi.mock('@/api/client');

function status(overrides: Partial<InstanceStatus> = {}): InstanceStatus {
    return {
        started: false,
        starting: false,
        client_count: 0,
        server_port: null,
        rcon_port: null,
        last_error: null,
        ...overrides
    };
}

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

describe('instanceStore', () => {
    it('mirrors the servers instance status', async () => {
        vi.mocked(client.getInstance).mockResolvedValue(status({started: true, client_count: 2}));
        const store = useInstanceStore();
        await expect(store.checkInstanceState()).resolves.toBe(true);
        expect(store.isStarted).toBe(true);
        expect(store.clientCount).toBe(2);
    });

    it('shows an in-flight start as starting, not as started', async () => {
        vi.mocked(client.getInstance).mockResolvedValue(status({starting: true}));
        const store = useInstanceStore();
        await store.checkInstanceState();
        expect(store.isStarting).toBe(true);
        expect(store.isStarted).toBe(false);
    });

    it('surfaces a background start failure through lastError and failed', async () => {
        // The failure happens after the 202, so this is the only channel the
        // browser has to learn about it.
        vi.mocked(client.getInstance).mockResolvedValue(status({last_error: 'archive missing'}));
        const store = useInstanceStore();
        await store.checkInstanceState();
        expect(store.isFailed).toBe(true);
        expect(store.getLastError).toBe('archive missing');
    });

    it('returns as soon as the start is accepted rather than waiting for factorio', async () => {
        vi.mocked(client.startInstance).mockResolvedValue({accepted: true});
        const app = useAppStore();
        app.settings = {
            gui: {enable_autostart: false, enable_restapi: true},
            restapi: {port: 7492, web_root: null},
            factorio: {factorio_archive_path: '/tmp/f.tar.xz'}
        } as never;

        const store = useInstanceStore();
        await store.startInstances();
        expect(store.isStarting).toBe(true);
        expect(store.isStarted).toBe(false);
    });

    it('refuses to start without a configured factorio archive path', async () => {
        const app = useAppStore();
        app.settings = {
            gui: {enable_autostart: false, enable_restapi: true},
            restapi: {port: 7492, web_root: null},
            factorio: {factorio_archive_path: ''}
        } as never;
        const store = useInstanceStore();
        await expect(store.startInstances()).rejects.toThrow(/archive path/);
        expect(client.startInstance).not.toHaveBeenCalled();
    });

    it('reports a 409 from the server with the servers own words', async () => {
        vi.mocked(client.startInstance).mockRejectedValue(
            new ApiError(409, 'instance already started', 5, {})
        );
        const app = useAppStore();
        app.settings = {
            gui: {enable_autostart: false, enable_restapi: true},
            restapi: {port: 7492, web_root: null},
            factorio: {factorio_archive_path: '/tmp/f.tar.xz'}
        } as never;
        const store = useInstanceStore();
        await expect(store.startInstances()).rejects.toThrow('instance already started');
        expect(store.isStarting).toBe(false);
        expect(store.isFailed).toBe(true);
    });

    it('clears started once the stop succeeds', async () => {
        vi.mocked(client.stopInstance).mockResolvedValue(undefined);
        const store = useInstanceStore();
        store.started = true;
        await store.stopInstances();
        expect(store.isStarted).toBe(false);
        expect(store.isStopping).toBe(false);
    });
});
```

- [ ] **Step 2: Run the tests and watch them fail**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/store/instanceStore.spec.ts'
```

Expected: FAIL — `invoke` is not defined.

- [ ] **Step 3: Rewrite the store**

Replace the whole of `app/src/store/instanceStore.ts`:

```ts
import {defineStore} from 'pinia';
import {getInstance, startInstance, stopInstance} from '@/api/client';
import {useAppStore} from '@/store/appStore';

export const useInstanceStore = defineStore('instance', {
    state: () => ({
        starting: false,
        stopping: false,
        started: false,
        failed: false,
        lastError: null as string | null,
        clientCount: 0
    }),
    getters: {
        isStarting(): boolean {
            return this.starting;
        },
        isStopping(): boolean {
            return this.stopping;
        },
        isFailed(): boolean {
            return this.failed;
        },
        isStarted(): boolean {
            return this.started;
        },
        getLastError(): string | null {
            return this.lastError;
        }
    },
    actions: {
        /**
         * One poll of GET /api/v1/instance. App.vue runs this on a 2 s timer:
         * starting Factorio is a background job on the server, so this is the
         * only way the browser learns that it finished — or failed.
         */
        async checkInstanceState(): Promise<boolean> {
            const status = await getInstance();
            this.started = status.started;
            this.starting = status.starting;
            this.clientCount = status.client_count;
            this.lastError = status.last_error;
            this.failed = status.last_error !== null;
            return this.started;
        },
        /**
         * Fires the start and returns. The server answers 202 because a
         * first-run archive extraction takes 8-10 minutes; `starting` is
         * cleared by the poll in App.vue, not here.
         */
        async startInstances(): Promise<void> {
            if (this.started) {
                throw new Error('already started');
            }
            const appStore = useAppStore();
            if (!appStore.settings?.factorio.factorio_archive_path) {
                throw new Error('please set the factorio archive path under settings first');
            }
            this.failed = false;
            this.lastError = null;
            this.starting = true;
            try {
                await startInstance();
            } catch (err) {
                this.starting = false;
                this.failed = true;
                this.lastError = err instanceof Error ? err.message : String(err);
                throw err;
            }
        },
        async stopInstances(): Promise<void> {
            if (!this.started) {
                throw new Error('not started');
            }
            this.failed = false;
            this.stopping = true;
            try {
                await stopInstance();
                this.started = false;
            } catch (err) {
                this.failed = true;
                this.lastError = err instanceof Error ? err.message : String(err);
                throw err;
            } finally {
                this.stopping = false;
            }
        }
    }
});
```

- [ ] **Step 4: Run the tests and watch them pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/store/instanceStore.spec.ts'
```

Expected: 7 passed.

- [ ] **Step 5: Commit**

```bash
git add app/src/store/instanceStore.ts app/src/store/instanceStore.spec.ts
git commit -m "refactor(app): move instanceStore onto the instance http routes"
```

---

## Task 8: `rconStore` over HTTP, and delete `restapiStore`

**Files:**
- Modify: `app/src/store/rconStore.ts`
- Create: `app/src/store/rconStore.spec.ts`
- Delete: `app/src/store/restapiStore.ts`

**Interfaces:**
- Consumes: `sendRcon` from `@/api/client` (Task 4).
- Produces:
  - state: `executing: boolean`, `success: boolean`, `error: boolean`, `lastError: string | null`
  - getters: `isExecuting`
  - action: `execute(command: string): Promise<void>` — **now rejects on failure**; previously it swallowed the error *and* set `success = true`, so `RconPage.vue`'s toast never fired.
- Removed: the entire `restapiStore` module and its `useRestApiStore` export. Every importer (`appStore.ts` — already gone in Task 6 — `App.vue` and `SettingsPage.vue`) is fixed in Task 11.

- [ ] **Step 1: Write the failing tests**

`app/src/store/rconStore.spec.ts`:

```ts
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useRconStore} from './rconStore';
import * as client from '@/api/client';
import {ApiError} from '@/api/http';

vi.mock('@/api/client');

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

describe('rconStore', () => {
    it('posts the command and settles', async () => {
        vi.mocked(client.sendRcon).mockResolvedValue(undefined);
        const store = useRconStore();
        await store.execute('/server-save');
        expect(client.sendRcon).toHaveBeenCalledWith('/server-save');
        expect(store.isExecuting).toBe(false);
        expect(store.success).toBe(true);
        expect(store.error).toBe(false);
    });

    it('rejects an empty command without touching the api', async () => {
        const store = useRconStore();
        await expect(store.execute('')).rejects.toThrow(/no command/);
        expect(client.sendRcon).not.toHaveBeenCalled();
    });

    it('rethrows a server error instead of reporting success', async () => {
        // The old store caught the error and then set success = true, so the
        // page's error toast was unreachable.
        vi.mocked(client.sendRcon).mockRejectedValue(new ApiError(400, 'not started', 2, {}));
        const store = useRconStore();
        await expect(store.execute('/server-save')).rejects.toThrow('not started');
        expect(store.error).toBe(true);
        expect(store.success).toBe(false);
        expect(store.isExecuting).toBe(false);
    });
});
```

- [ ] **Step 2: Run the tests and watch them fail**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/store/rconStore.spec.ts'
```

Expected: FAIL — `invoke` is not defined.

- [ ] **Step 3: Rewrite the store**

Replace the whole of `app/src/store/rconStore.ts`:

```ts
import {defineStore} from 'pinia';
import {sendRcon} from '@/api/client';

export const useRconStore = defineStore('rcon', {
    state: () => ({
        executing: false,
        success: false,
        error: false,
        lastError: null as string | null
    }),
    getters: {
        isExecuting(): boolean {
            return this.executing;
        }
    },
    actions: {
        async execute(command: string): Promise<void> {
            if (!command) {
                throw new Error('no command to execute?');
            }
            this.error = false;
            this.success = false;
            this.lastError = null;
            this.executing = true;
            try {
                await sendRcon(command);
                this.success = true;
            } catch (err) {
                this.error = true;
                this.lastError = err instanceof Error ? err.message : String(err);
                // Rethrow: RconPage.vue's catch is what shows the toast, and
                // the old store swallowed this while also setting success.
                throw err;
            } finally {
                this.executing = false;
            }
        }
    }
});
```

- [ ] **Step 4: Delete the restapi store**

```bash
git rm app/src/store/restapiStore.ts
```

- [ ] **Step 5: Run the tests and watch them pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/store/rconStore.spec.ts'
```

Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add app/src/store/rconStore.ts app/src/store/rconStore.spec.ts
git commit -m "refactor(app): move rconStore onto POST /api/v1/rcon and drop restapiStore"
```

---

## Task 9: The SSE job-event consumer

> **Contract note from plan 4 Task 7, which shipped after this plan was written.**
>
> **Recorded dissent, and why the consumer-side rule still stands.** Task 7's implementer argued this should be a wire-level `interrupted` event rather than a documented consumer obligation: a 145-line gap gets an explicit `lagged`, while losing the entire remainder of a run gets silence, so the more severe failure is reported less. That is a fair objection and it is on the record.
>
> The reason it is not a blocker: **re-reading `GET /api/v1/jobs/{id}` after the stream ends is correct for every truncation cause**, not just shutdown — a dropped connection, a proxy timeout, and a killed server are indistinguishable from the client's side and all resolve the same way. An `interrupted` event would cover exactly the one case where the server survives long enough to send it, which is the case the client least needs help with. If a wire marker is ever added it should be treated as an optimisation, not as the thing that makes the contract sound.
>
> **A stream cut short by server shutdown ends without a `finished` event.** The consumer must treat EOF-without-`finished` as **unknown**, not as success — otherwise a shutdown mid-script silently reports every running job as having completed. Resolve the real outcome with one `GET /api/v1/jobs/{id}` after the stream ends, which this task already does for the reconnect case; the same call covers this one.
>
> **The backlog replays all stdout, then all stderr** — not interleaved in the order they were produced. `Job` keeps two separate buffers, so a late subscriber sees the two streams concatenated rather than in real time order. Live events after attach *are* in order. If the output pane interleaves them, say so in the UI or accept that a replayed run reads differently from a watched one.


**Files:**
- Create: `app/src/api/jobEvents.ts`
- Create: `app/src/api/jobEvents.spec.ts`

**Interfaces:**
- Consumes: `jobEventsUrl`, `getJob` from `@/api/client` (Task 4); `Job`, `JobStatus`, `OutputStream` from `@/api/types` (Task 4). Wire format from plan 4 Task 7: `event: output` with `{"stream":"stdout","text":"…"}`, `event: finished` with `{"status":"succeeded"}`, `event: lagged` with `{"skipped":12}`.
- Produces:
  - `export interface EventSourceLike { addEventListener(type: string, listener: (event: MessageEvent) => void): void; close(): void }`
  - `export interface JobEventHandlers { onOutput(stream: OutputStream, text: string): void; onLagged(skipped: number): void; onFinished(status: JobStatus): void; onError(error: Error): void }`
  - `export function subscribeJobEvents(jobId: string, handlers: JobEventHandlers, createSource?: (url: string) => EventSourceLike): () => void` — the returned function unsubscribes.

**The four cases this must get right:**
1. **Job finishes** — `finished` arrives; call `onFinished(status)` exactly once and close.
2. **Stream ends** — the server closes after `finished`. Browsers fire `error` on *any* close, including a clean one, so the `finished` handler must have already latched or the error path would double-report.
3. **`lagged`** — a slow subscriber missed messages. Report the gap; do **not** close.
4. **Server goes away mid-stream** — `EventSource` reconnects automatically, and the server replays a job's buffered output to every new subscriber, so an unhandled reconnect duplicates the entire output. Close first, then ask `GET /api/v1/jobs/{id}` once: finished → `onFinished`, still running → `onError`, request fails too → `onError`.

- [ ] **Step 1: Write the failing tests**

`app/src/api/jobEvents.spec.ts`:

```ts
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {subscribeJobEvents, EventSourceLike} from './jobEvents';
import * as client from './client';
import {Job} from './types';

vi.mock('./client', () => ({
    jobEventsUrl: (id: string) => '/api/v1/jobs/' + id + '/events',
    getJob: vi.fn()
}));

class FakeEventSource implements EventSourceLike {
    listeners = new Map<string, Array<(event: MessageEvent) => void>>();
    closed = false;

    addEventListener(type: string, listener: (event: MessageEvent) => void) {
        const existing = this.listeners.get(type) ?? [];
        existing.push(listener);
        this.listeners.set(type, existing);
    }

    close() {
        this.closed = true;
    }

    emit(type: string, data?: unknown) {
        for (const listener of this.listeners.get(type) ?? []) {
            listener({data: data === undefined ? '' : JSON.stringify(data)} as MessageEvent);
        }
    }
}

function handlers() {
    return {
        onOutput: vi.fn(),
        onLagged: vi.fn(),
        onFinished: vi.fn(),
        onError: vi.fn()
    };
}

function job(overrides: Partial<Job> = {}): Job {
    return {
        id: '7',
        script: '/a.lua',
        status: 'running',
        started_at_ms: 0,
        finished_at_ms: null,
        stdout: '',
        stderr: '',
        error: null,
        ...overrides
    };
}

let source: FakeEventSource;

beforeEach(() => {
    vi.resetAllMocks();
    source = new FakeEventSource();
});

describe('subscribeJobEvents', () => {
    it('reports each output line with its stream', () => {
        const h = handlers();
        subscribeJobEvents('7', h, () => source);
        source.emit('output', {stream: 'stdout', text: 'hello'});
        source.emit('output', {stream: 'stderr', text: 'oops'});
        expect(h.onOutput).toHaveBeenNthCalledWith(1, 'stdout', 'hello');
        expect(h.onOutput).toHaveBeenNthCalledWith(2, 'stderr', 'oops');
    });

    it('closes the connection once the job finishes', () => {
        const h = handlers();
        subscribeJobEvents('7', h, () => source);
        source.emit('finished', {status: 'succeeded'});
        expect(h.onFinished).toHaveBeenCalledWith('succeeded');
        expect(source.closed).toBe(true);
    });

    it('does not double-report when the stream ends right after finishing', () => {
        // A browser EventSource fires `error` on a clean end-of-stream too.
        const h = handlers();
        subscribeJobEvents('7', h, () => source);
        source.emit('finished', {status: 'failed'});
        source.emit('error');
        expect(h.onFinished).toHaveBeenCalledTimes(1);
        expect(h.onError).not.toHaveBeenCalled();
        expect(client.getJob).not.toHaveBeenCalled();
    });

    it('reports a lagged gap without tearing the stream down', () => {
        const h = handlers();
        subscribeJobEvents('7', h, () => source);
        source.emit('lagged', {skipped: 12});
        expect(h.onLagged).toHaveBeenCalledWith(12);
        expect(source.closed).toBe(false);
        source.emit('output', {stream: 'stdout', text: 'still here'});
        expect(h.onOutput).toHaveBeenCalledWith('stdout', 'still here');
    });

    it('closes on a mid-stream error so the browser cannot silently reconnect and replay', async () => {
        // The server replays a job's buffered output to every new subscriber,
        // so an EventSource auto-reconnect would duplicate the whole run.
        const h = handlers();
        vi.mocked(client.getJob).mockResolvedValue(job({status: 'running'}));
        subscribeJobEvents('7', h, () => source);
        source.emit('error');
        expect(source.closed).toBe(true);
        await vi.waitFor(() => expect(h.onError).toHaveBeenCalled());
        expect(h.onError.mock.calls[0][0].message).toMatch(/lost connection/);
    });

    it('recovers the outcome when the job finished while the connection was down', async () => {
        const h = handlers();
        vi.mocked(client.getJob).mockResolvedValue(job({status: 'succeeded'}));
        subscribeJobEvents('7', h, () => source);
        source.emit('error');
        await vi.waitFor(() => expect(h.onFinished).toHaveBeenCalledWith('succeeded'));
        expect(h.onError).not.toHaveBeenCalled();
    });

    it('reports an error when the server is gone entirely', async () => {
        const h = handlers();
        vi.mocked(client.getJob).mockRejectedValue(new TypeError('Failed to fetch'));
        subscribeJobEvents('7', h, () => source);
        source.emit('error');
        await vi.waitFor(() => expect(h.onError).toHaveBeenCalled());
        expect(h.onError.mock.calls[0][0].message).toMatch(/server/);
    });

    it('stops delivering events after the caller unsubscribes', () => {
        const h = handlers();
        const unsubscribe = subscribeJobEvents('7', h, () => source);
        unsubscribe();
        expect(source.closed).toBe(true);
        source.emit('output', {stream: 'stdout', text: 'too late'});
        source.emit('finished', {status: 'succeeded'});
        expect(h.onOutput).not.toHaveBeenCalled();
        expect(h.onFinished).not.toHaveBeenCalled();
    });

    it('survives a malformed data payload', () => {
        const h = handlers();
        subscribeJobEvents('7', h, () => source);
        for (const listener of source.listeners.get('output') ?? []) {
            listener({data: 'not json'} as MessageEvent);
        }
        expect(h.onOutput).not.toHaveBeenCalled();
        expect(h.onError).not.toHaveBeenCalled();
    });
});
```

- [ ] **Step 2: Run the tests and watch them fail**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/api/jobEvents.spec.ts'
```

Expected: FAIL — cannot resolve `./jobEvents`.

- [ ] **Step 3: Write the implementation**

`app/src/api/jobEvents.ts`:

```ts
import {getJob, jobEventsUrl} from './client';
import {JobStatus, OutputStream} from './types';

/** The slice of EventSource this module uses, so tests can supply a fake. */
export interface EventSourceLike {
    addEventListener(type: string, listener: (event: MessageEvent) => void): void;
    close(): void;
}

export interface JobEventHandlers {
    onOutput(stream: OutputStream, text: string): void;
    /** The server dropped `skipped` messages for this slow subscriber. */
    onLagged(skipped: number): void;
    onFinished(status: JobStatus): void;
    onError(error: Error): void;
}

function parse(event: MessageEvent): Record<string, unknown> | null {
    if (typeof event.data !== 'string' || event.data.length === 0) {
        return null;
    }
    try {
        const parsed = JSON.parse(event.data);
        return parsed !== null && typeof parsed === 'object'
            ? parsed as Record<string, unknown>
            : null;
    } catch {
        return null;
    }
}

/**
 * Subscribes to a job's output stream. Returns an unsubscribe function that is
 * safe to call at any point, including after the job has already finished.
 *
 * The stream is single-shot on purpose. EventSource reconnects by itself after
 * any error, and the server replays a job's buffered output to every new
 * subscriber — so an unattended reconnect would append the whole run a second
 * time. Instead this closes on the first error and asks the job endpoint once
 * what actually happened.
 */
export function subscribeJobEvents(
    jobId: string,
    handlers: JobEventHandlers,
    createSource: (url: string) => EventSourceLike = (url) => new EventSource(url)
): () => void {
    let settled = false;
    const source = createSource(jobEventsUrl(jobId));

    const close = () => {
        try {
            source.close();
        } catch {
            // already closed; nothing to do
        }
    };

    source.addEventListener('output', (event) => {
        if (settled) {
            return;
        }
        const data = parse(event);
        if (data === null) {
            return;
        }
        const stream: OutputStream = data.stream === 'stderr' ? 'stderr' : 'stdout';
        handlers.onOutput(stream, typeof data.text === 'string' ? data.text : '');
    });

    source.addEventListener('lagged', (event) => {
        if (settled) {
            return;
        }
        const data = parse(event);
        const skipped = data !== null && typeof data.skipped === 'number' ? data.skipped : 0;
        handlers.onLagged(skipped);
    });

    source.addEventListener('finished', (event) => {
        if (settled) {
            return;
        }
        settled = true;
        close();
        const data = parse(event);
        const status: JobStatus = data !== null && data.status === 'failed' ? 'failed' : 'succeeded';
        handlers.onFinished(status);
    });

    source.addEventListener('error', () => {
        // Fires on a clean end-of-stream too, which is why `settled` is
        // checked first: after `finished` there is nothing left to report.
        if (settled) {
            return;
        }
        settled = true;
        close();
        getJob(jobId).then((job) => {
            if (job.status === 'running') {
                handlers.onError(new Error('lost connection to the job output stream'));
            } else {
                handlers.onFinished(job.status);
            }
        }).catch(() => {
            handlers.onError(new Error('lost connection to the server'));
        });
    });

    return () => {
        settled = true;
        close();
    };
}
```

- [ ] **Step 4: Run the tests and watch them pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/api/jobEvents.spec.ts'
```

Expected: 9 passed.

- [ ] **Step 5: Prove the reconnect guard is load-bearing (mutation)**

Delete the `close()` call from the `error` listener. `closes on a mid-stream error so the browser cannot silently reconnect and replay` must fail on `expect(source.closed).toBe(true)`. Record that it failed, then restore.

- [ ] **Step 6: Lint and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint'
git add app/src/api/jobEvents.ts app/src/api/jobEvents.spec.ts
git commit -m "feat(app): consume job output over server-sent events"
```

---

## Task 10: `scriptStore` over HTTP and SSE, and the live output pane

**Files:**
- Modify: `app/src/store/scriptStore.ts`
- Create: `app/src/store/scriptStore.spec.ts`
- Modify: `app/src/pages/ScriptPage.vue`

**Interfaces:**
- Consumes: `listScripts`, `readScript`, `writeScript`, `executeScript` from `@/api/client` (Task 4); `subscribeJobEvents` from `@/api/jobEvents` (Task 9); `ApiError` from `@/api/http` (Task 3); `languageFromPath` from `@/utils`.
- Produces:
  - state adds `jobId: string | null`
  - getters unchanged: `isExecuting`, `getCode`, `getLanguage`, `getStdout`, `getStderr`, `getLoadingScriptsInDirectory`, `getActiveScriptPath`
  - actions unchanged in name: `loadScriptsInDirectory(path): Promise<ScriptTreeNode[]>`, `loadScriptFile(path): Promise<string>`, `setCode(code): Promise<void>`, `executeCode(): Promise<void>`, `executeScript(): Promise<void>`
  - action added: `attachToJob(jobId: string): void` — subscribes to an already-running job (used for the 409 path)
  - action added: `stopWatching(): void` — unsubscribes; called from `ScriptPage.vue`'s `onUnmounted`

- [ ] **Step 1: Write the failing tests**

`app/src/store/scriptStore.spec.ts`:

```ts
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {useScriptStore} from './scriptStore';
import * as client from '@/api/client';
import * as jobEvents from '@/api/jobEvents';
import {ApiError} from '@/api/http';
import {JobEventHandlers} from '@/api/jobEvents';

vi.mock('@/api/client');
vi.mock('@/api/jobEvents');

let captured: JobEventHandlers | null = null;
const unsubscribe = vi.fn();

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
    captured = null;
    vi.mocked(jobEvents.subscribeJobEvents).mockImplementation((_id, handlers) => {
        captured = handlers;
        return unsubscribe;
    });
});

describe('scriptStore', () => {
    it('lists a directory through GET /api/v1/scripts', async () => {
        vi.mocked(client.listScripts).mockResolvedValue([
            {key: '/a.lua', label: 'a.lua', leaf: true, children: []}
        ]);
        const store = useScriptStore();
        const nodes = await store.loadScriptsInDirectory('/');
        expect(nodes[0].key).toBe('/a.lua');
        expect(store.getLoadingScriptsInDirectory).toBe(false);
    });

    it('clears the loading flag even when the listing fails', async () => {
        vi.mocked(client.listScripts).mockRejectedValue(new ApiError(400, 'not a directory', 1, {}));
        const store = useScriptStore();
        await expect(store.loadScriptsInDirectory('/nope')).rejects.toThrow('not a directory');
        expect(store.getLoadingScriptsInDirectory).toBe(false);
    });

    it('unwraps the code field and derives the editor language', async () => {
        vi.mocked(client.readScript).mockResolvedValue({code: 'print(1)'});
        const store = useScriptStore();
        await store.loadScriptFile('/sub/a.lua');
        expect(store.getCode).toBe('print(1)');
        expect(store.getLanguage).toBe('lua');
        expect(store.getActiveScriptPath).toBe('/sub/a.lua');
    });

    it('saves the editor buffer to the active path', async () => {
        vi.mocked(client.readScript).mockResolvedValue({code: 'print(1)'});
        vi.mocked(client.writeScript).mockResolvedValue(undefined);
        const store = useScriptStore();
        await store.loadScriptFile('/a.lua');
        await store.setCode('print(2)');
        expect(client.writeScript).toHaveBeenCalledWith('/a.lua', 'print(2)');
    });

    it('starts a script run and streams its output into stdout and stderr', async () => {
        vi.mocked(client.readScript).mockResolvedValue({code: 'print(1)'});
        vi.mocked(client.executeScript).mockResolvedValue({job_id: '7'});
        const store = useScriptStore();
        await store.loadScriptFile('/a.lua');
        await store.executeScript();

        expect(client.executeScript).toHaveBeenCalledWith({path: '/a.lua'});
        expect(store.isExecuting).toBe(true);

        captured!.onOutput('stdout', 'hello');
        captured!.onOutput('stderr', 'careful');
        expect(store.getStdout).toBe('hello\n');
        expect(store.getStderr).toBe('careful\n');

        captured!.onFinished('succeeded');
        expect(store.isExecuting).toBe(false);
        expect(store.success).toBe(true);
    });

    it('sends inline code with its language when running the buffer', async () => {
        vi.mocked(client.executeScript).mockResolvedValue({job_id: '8'});
        const store = useScriptStore();
        store.code = 'print(1)';
        store.language = 'lua';
        await store.executeCode();
        expect(client.executeScript).toHaveBeenCalledWith({code: 'print(1)', language: 'lua'});
    });

    it('makes a lagged gap visible in the output instead of losing it silently', async () => {
        vi.mocked(client.executeScript).mockResolvedValue({job_id: '9'});
        const store = useScriptStore();
        store.code = 'print(1)';
        await store.executeCode();
        captured!.onLagged(12);
        expect(store.getStdout).toContain('12');
    });

    it('marks the run failed when the stream dies', async () => {
        vi.mocked(client.executeScript).mockResolvedValue({job_id: '9'});
        const store = useScriptStore();
        store.code = 'print(1)';
        await store.executeCode();
        captured!.onError(new Error('lost connection to the server'));
        expect(store.isExecuting).toBe(false);
        expect(store.error).toBe(true);
        expect(store.getStderr).toContain('lost connection');
    });

    it('attaches to the already-running job when the server answers 409', async () => {
        // One script at a time: rather than a dead-end error, follow the id
        // the server hands back and show that run's output.
        vi.mocked(client.executeScript).mockRejectedValue(
            new ApiError(409, 'a script is already running', 409, {running_job_id: '3'})
        );
        const store = useScriptStore();
        store.code = 'print(1)';
        await expect(store.executeCode()).rejects.toThrow('already running');
        expect(jobEvents.subscribeJobEvents).toHaveBeenCalledWith('3', expect.anything());
        expect(store.jobId).toBe('3');
        expect(store.isExecuting).toBe(true);
    });

    it('unsubscribes when told to stop watching', async () => {
        vi.mocked(client.executeScript).mockResolvedValue({job_id: '7'});
        const store = useScriptStore();
        store.code = 'print(1)';
        await store.executeCode();
        store.stopWatching();
        expect(unsubscribe).toHaveBeenCalled();
    });
});
```

- [ ] **Step 2: Run the tests and watch them fail**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/store/scriptStore.spec.ts'
```

Expected: FAIL — `invoke` is not defined.

- [ ] **Step 3: Rewrite the store**

Replace the whole of `app/src/store/scriptStore.ts`:

```ts
import {defineStore} from 'pinia';
import {ScriptTreeNode} from '@/models/types';
import {languageFromPath} from '@/utils';
import {executeScript as postExecute, listScripts, readScript, writeScript} from '@/api/client';
import {subscribeJobEvents} from '@/api/jobEvents';
import {ApiError} from '@/api/http';
import {JobStatus, OutputStream} from '@/api/types';

/**
 * The unsubscribe callback of the live stream. Kept outside the Pinia state
 * on purpose: it is a closure, not serialisable data, and putting a function
 * in `state` makes it reactive for no reason.
 */
let unsubscribe: (() => void) | null = null;

function runningJobIdOf(err: unknown): string | null {
    if (!(err instanceof ApiError) || err.status !== 409) {
        return null;
    }
    const body = err.body;
    if (body === null || typeof body !== 'object') {
        return null;
    }
    const id = (body as Record<string, unknown>).running_job_id;
    return typeof id === 'string' ? id : null;
}

export const useScriptStore = defineStore('script', {
    state: () => ({
        code: '',
        language: 'lua',
        executing: false,
        success: false,
        error: false,
        stdout: '',
        stderr: '',
        jobId: null as string | null,

        activeScriptPath: '',
        loadingScriptsInDirectory: false
    }),
    getters: {
        isExecuting(): boolean {
            return this.executing;
        },
        getCode(): string {
            return this.code;
        },
        getLanguage(): string {
            return this.language;
        },
        getStdout(): string {
            return this.stdout;
        },
        getStderr(): string {
            return this.stderr;
        },
        getLoadingScriptsInDirectory(): boolean {
            return this.loadingScriptsInDirectory;
        },
        getActiveScriptPath(): string {
            return this.activeScriptPath;
        }
    },
    actions: {
        async loadScriptsInDirectory(path: string): Promise<ScriptTreeNode[]> {
            this.loadingScriptsInDirectory = true;
            try {
                return await listScripts(path);
            } finally {
                this.loadingScriptsInDirectory = false;
            }
        },
        async loadScriptFile(path: string): Promise<string> {
            this.activeScriptPath = path;
            const content = await readScript(path);
            this.code = content.code;
            this.language = languageFromPath(path);
            return content.code;
        },
        async setCode(code: string): Promise<void> {
            this.code = code;
            if (!this.activeScriptPath) {
                return;
            }
            await writeScript(this.activeScriptPath, this.code);
        },
        /** Subscribes to a job's output stream and mirrors it into state. */
        attachToJob(jobId: string): void {
            this.stopWatching();
            this.jobId = jobId;
            this.executing = true;
            unsubscribe = subscribeJobEvents(jobId, {
                onOutput: (stream: OutputStream, text: string) => {
                    if (stream === 'stderr') {
                        this.stderr += text + '\n';
                    } else {
                        this.stdout += text + '\n';
                    }
                },
                onLagged: (skipped: number) => {
                    // A visible gap beats silently missing lines.
                    this.stdout += '... ' + skipped + ' lines skipped (output produced faster than this page could read it) ...\n';
                },
                onFinished: (status: JobStatus) => {
                    this.executing = false;
                    this.success = status === 'succeeded';
                    this.error = status === 'failed';
                    unsubscribe = null;
                },
                onError: (err: Error) => {
                    this.executing = false;
                    this.success = false;
                    this.error = true;
                    this.stderr += err.message + '\n';
                    unsubscribe = null;
                }
            });
        },
        stopWatching(): void {
            if (unsubscribe !== null) {
                unsubscribe();
                unsubscribe = null;
            }
        },
        async executeCode(): Promise<void> {
            if (!this.code) {
                throw new Error('no code to execute?');
            }
            await this._execute({code: this.code, language: this.language});
        },
        async executeScript(): Promise<void> {
            if (!this.activeScriptPath) {
                throw new Error('no script to execute?');
            }
            await this._execute({path: this.activeScriptPath});
        },
        async _execute(body: {path?: string, code?: string, language?: string}): Promise<void> {
            this.stopWatching();
            this.stdout = '';
            this.stderr = '';
            this.error = false;
            this.success = false;
            this.executing = true;
            this.jobId = null;
            try {
                const accepted = await postExecute(body);
                this.attachToJob(accepted.job_id);
            } catch (err) {
                this.executing = false;
                this.error = true;
                // One script runs at a time. When the slot is taken the server
                // names the occupant; follow it so the user sees that run's
                // output rather than an error with no way forward.
                const runningJobId = runningJobIdOf(err);
                if (runningJobId !== null) {
                    this.attachToJob(runningJobId);
                }
                throw err;
            }
        }
    }
});
```

- [ ] **Step 4: Run the tests and watch them pass**

```
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/store/scriptStore.spec.ts'
```

Expected: 10 passed.

- [ ] **Step 5: Wire the page to stop watching on unmount**

In `app/src/pages/ScriptPage.vue`, change the `<script setup>` imports line

```ts
import {computed} from 'vue';
```

to

```ts
import {computed, onUnmounted} from 'vue';
```

and add, just above the closing `</script>`:

```ts
// The SSE connection outlives the component otherwise: navigating away from
// the page would leave an open stream appending into a store nothing renders.
onUnmounted(() => {
  scriptStore.stopWatching()
})
```

Leave the rest of the file — including its PrimeFlex class names — alone.

- [ ] **Step 6: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/'
git add app/src/store/scriptStore.ts app/src/store/scriptStore.spec.ts app/src/pages/ScriptPage.vue
git commit -m "feat(app): run scripts over http and stream their output live"
```

---

## Task 11: Views — remove the last Tauri imports and the restapi UI

**Files:**
- Modify: `app/src/pages/SettingsPage.vue`
- Modify: `app/src/App.vue`

**Interfaces:**
- Consumes: `useAppStore` (Task 6, no `maximizeWindow` / `openInBrowser` / `updateEnableRestApi`), `useInstanceStore` (Task 7, `checkInstanceState`), `useToast` from PrimeVue.
- Produces: no exports; after this task `grep -rn "@tauri-apps" app/src/` returns only `app/src/plugins/configure-ynetwork.js`, which Task 12 deletes.

- [ ] **Step 1: Strip the Tauri dialog out of SettingsPage**

In `app/src/pages/SettingsPage.vue`:

Delete line 4 (`import {open} from '@tauri-apps/plugin-dialog'`) and line 9 (`import {useRestApiStore} from '@/store/restapiStore';`).

Delete `const restApiStore = useRestApiStore();` (line 12), the `enableRestapi` computed (lines 45-52), the `isPortAvaiable` computed (line 136), the `selectWorkspacePath` function (lines 86-98), the `selectFactorioArchivePath` function (lines 100-112), and the `openInBrowser` function (lines 153-159).

- [ ] **Step 2: Fix the template**

In the same file's `<template>`, replace the "Factorio Archive" card with:

```html
      <div class="card p-fluid">
        <h5>Factorio Archive - Download from
          <a href="https://factorio.com/download" target="_blank" rel="noopener">https://factorio.com/download</a>
        </h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <InputText v-model="factorioArchivePath" :class="isFactorioArchivePathValid ? '' : 'p-invalid'"/>
            <small v-if="!isFactorioArchivePathValid" class="p-error">
              no such file on the server
            </small>
          </div>
        </div>
      </div>
```

Replace the whole "Enable REST API" card with:

```html
      <div class="card p-fluid">
        <h5>HTTP API</h5>
        <p>
          This page is served by the same server that exposes the API.
          <a href="/swagger-ui/" target="_blank" rel="noopener">Swagger UI</a>
          &middot;
          <a href="/openapi.json" target="_blank" rel="noopener">openapi.json</a>
        </p>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <label for="restapi_port">Port (applies on the next server start)</label>
            <InputText id="restapi_port" v-model="restapiPort" type="number" min="1" max="65535"/>
          </div>
        </div>
      </div>
```

Replace the "Workspace Folder" card with:

```html
      <div class="card p-fluid">
        <h5>Workspace Folder</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <InputText v-model="workspacePath" :class="isWorkspacePathValid ? '' : 'p-invalid'"/>
            <small v-if="!isWorkspacePathValid" class="p-error">
              no such directory on the server
            </small>
          </div>
        </div>
      </div>
```

Then delete the now-unused `Button` import (line 7) if no `<Button>` remains in the template.

Note for the implementer: these are *server-side* paths. A browser file picker returns a `File`, never a path, so there is nothing to substitute — a text input validated by `GET /api/v1/fs/exists` (which the two `isValid` refs already do) is the honest replacement. The PrimeFlex class names above are deliberately preserved; the layout is the next plan's problem.

- [ ] **Step 3: Rewrite the App.vue bootstrap**

In `app/src/App.vue`, delete the `import {useRestApiStore} from '@/store/restapiStore';` line, change

```ts
import {computed, onBeforeUpdate, onMounted, ref} from 'vue';
```

to

```ts
import {computed, onBeforeUpdate, onMounted, onUnmounted, ref} from 'vue';
```

and replace the whole `onMounted(async () => { ... })` block with:

```ts
// Starting Factorio is a background job on the server (POST
// /api/v1/instance/start answers 202), so the browser learns the outcome by
// polling. Two seconds against a loopback server is free.
const INSTANCE_POLL_MS = 2000
let instancePollTimer: number | null = null

onMounted(async () => {
  const toast = useToast()
  const appStore = useAppStore()
  const instanceStore = useInstanceStore()
  try {
    await appStore.loadSettings()
    await instanceStore.checkInstanceState()
  } catch (err) {
    // Without this the app renders a blank shell when the server is down and
    // never says why.
    toast.add({
      severity: 'error',
      summary: 'Cannot reach the factorio-bot server',
      detail: err instanceof Error ? err.message : String(err),
      life: 10000
    })
  }
  instancePollTimer = window.setInterval(() => {
    instanceStore.checkInstanceState().catch(() => {
      // A transient poll failure is not worth a toast every two seconds; the
      // next successful poll refreshes the state.
    })
  }, INSTANCE_POLL_MS)
})

onUnmounted(() => {
  if (instancePollTimer !== null) {
    window.clearInterval(instancePollTimer)
    instancePollTimer = null
  }
})
```

**Autostart is deliberately not reimplemented here.** The old code fired it on `onMounted`, which in a browser means "every time anyone opens a tab" — it would race N tabs into N start requests. Per the spec it belongs in server startup (`app/src-tauri/src/cli/serve.rs`), which no plan has implemented yet. Leave `settings.gui.enable_autostart` persisted and inert, and record the follow-up.

- [ ] **Step 4: Verify no Tauri imports remain in `src/` outside the dead plugin**

```bash
grep -rn "@tauri-apps" app/src/
```

Expected: exactly one file, `app/src/plugins/configure-ynetwork.js`.

- [ ] **Step 5: Lint and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm vitest run src/'
git add app/src/pages/SettingsPage.vue app/src/App.vue
git commit -m "refactor(app): drop the native file pickers and the restapi control ui"
```

---

## Task 12: Delete Tauri from the JavaScript side

**Files:**
- Delete: `app/src/plugins/configure-ynetwork.js`
- Modify: `app/src/main.ts`
- Modify: `app/package.json`
- Modify: `app/vite.config.mts`
- Delete: `app/scripts/build-auto-updater-json.js`, `app/scripts/merge-auto-updater-json.js`, `app/scripts/update-checksum.js`

**Interfaces:**
- Consumes: nothing.
- Produces: `app/vite.config.mts` exports a config whose `server.proxy` forwards `/api`, `/openapi.json` and `/swagger-ui` to `http://127.0.0.1:7492`; `pnpm start` runs Vite alone.

- [ ] **Step 1: Delete the dead transport plugin and its import**

```bash
git rm app/src/plugins/configure-ynetwork.js
```

In `app/src/main.ts`, delete the line:

```ts
import './plugins/configure-ynetwork';
```

- [ ] **Step 2: Remove the Tauri packages and scripts**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm remove @tauri-apps/api @tauri-apps/plugin-dialog @tauri-apps/plugin-fs @tauri-apps/plugin-http @tauri-apps/cli ynetwork cross-env concurrently'
```

Then replace the `"scripts"` block of `app/package.json` with:

```json
  "scripts": {
    "start": "vite --port 8080",
    "serve": "vite --port 8080",
    "build:web": "vite build",
    "preview": "vite preview --port 8080",
    "test": "vitest",
    "test:coverage": "vitest run --coverage",
    "e2e": "node e2e/smoke.mjs",
    "openapi:snapshot": "curl -sSf http://127.0.0.1:7492/openapi.json -o src/api/openapi.snapshot.json && node -e \"const f='src/api/openapi.snapshot.json';const j=JSON.parse(require('fs').readFileSync(f,'utf8'));require('fs').writeFileSync(f,JSON.stringify(j,null,2)+'\\n')\"",
    "lint": "tsc --noEmit --skipLibCheck && vue-tsc --noEmit --skipLibCheck && eslint src/",
    "test:ci": "pnpm run lint && pnpm run test:coverage && pnpm run build:web && pnpm run cargo:test && pnpm run cargo:clippy",
    "cargo:clippy": "cd .. && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated",
    "cargo:test": "cd .. && cargo test",
    "cargo:tree": "cd .. && cargo tree --charset ascii > tree.txt",
    "cargo:docs": "cd .. && cargo doc --all --no-deps",
    "cargo:userguide": "cd ../docs/userguide && mdbook serve -o",
    "cargo:devguide": "cd ../docs/devguide && mdbook serve -o",
    "changelog:update": "cd .. && git cliff --output CHANGELOG.md --config Cargo.toml",
    "precommit:check": "pnpm run lint && pnpm run test:coverage && pnpm run build:web && pnpm run cargo:clippy && pnpm run cargo:test && cd .. && cargo build --all-features && cargo build --no-default-features"
  },
```

(`lint1` and `dummy` go too: `lint1` duplicated half of `lint`, and `dummy` existed only as `tauri-action`'s `tauriScript`.)

- [ ] **Step 3: Add the dev proxy**

In `app/vite.config.mts`, add a `server` block between `plugins` and `test`:

```ts
    server: {
        port: 8080,
        // In production the axum server serves the SPA and the API from one
        // origin, so the client uses relative URLs. The Vite dev server has to
        // reproduce that or every request would 404 against Vite itself.
        proxy: {
            '/api': {
                target: 'http://127.0.0.1:7492',
                changeOrigin: true
            },
            '/openapi.json': {
                target: 'http://127.0.0.1:7492',
                changeOrigin: true
            },
            '/swagger-ui': {
                target: 'http://127.0.0.1:7492',
                changeOrigin: true
            }
        }
    },
```

Note for the implementer: `/api/v1/jobs/{id}/events` is an SSE stream and goes through this proxy. Vite's proxy streams responses without buffering, so nothing extra is needed — but if output ever arrives only when a script finishes, this proxy is the first suspect, and the workaround is to point `VITE_API_BASE` straight at `http://127.0.0.1:7492` instead.

- [ ] **Step 4: Delete the auto-updater helper scripts**

```bash
git rm app/scripts/build-auto-updater-json.js app/scripts/merge-auto-updater-json.js app/scripts/update-checksum.js
```

Keep `app/scripts/update-package-version.js`: `publish.yml` still uses it to stamp the version into `package.json`.

- [ ] **Step 5: Verify nothing references Tauri from JavaScript any more**

```bash
grep -rn "tauri\|ynetwork\|VITE_HTTP_HANDLER" app/src app/package.json app/vite.config.mts app/index.html
```

Expected: no output.

- [ ] **Step 6: Build, lint, test, commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm install && pnpm run lint && pnpm vitest run src/ && pnpm run build:web'
git add app/package.json app/pnpm-lock.yaml app/vite.config.mts app/src/main.ts
git commit -m "chore(app): remove the tauri packages, the dead ynetwork plugin and the updater scripts"
```

---

## Task 13: Move TypeScript type generation to `crates/server`

`typescriptify()` regenerates `app/src/models/types.ts` from `crates/core/src/types.rs`. It runs under `#[cfg(feature = "gui")]` in `app/src-tauri/build.rs`, so deleting the `gui` feature would silently stop it. Move it first.

**Files:**
- Create: `crates/server/build.rs`
- Modify: `crates/server/Cargo.toml`
- Modify: `app/src-tauri/build.rs`

**Interfaces:**
- Consumes: `factorio_bot_core::types::*`, `factorio_bot_core::settings::{FactorioSettings, RestApiSettings}`, and `typescript_definitions::TypeScriptifyTrait` (a normal dependency of `crates/core`, so it survives).
- Produces: `crates/server/build.rs` rewrites everything after the `// --- AUTOGENERATED` marker line in `app/src/models/types.ts` (currently line 117).

**Collision warning:** another agent has uncommitted edits in `app/src-tauri/build.rs`. **Re-read the working-tree file right now** and copy *that* version of `typescriptify()`, not the one quoted below, if they differ. The version below is the working tree as of writing this plan.

- [ ] **Step 1: Read the current source of truth**

```bash
sed -n '43,120p' app/src-tauri/build.rs
```

- [ ] **Step 2: Create `crates/server/build.rs`**

```rust
//! Regenerates the frontend's `models/types.ts` from the Rust types the API
//! serialises. Moved here from `app/src-tauri/build.rs`, where it lived behind
//! the now-deleted `gui` feature. It belongs to the crate that owns the wire
//! format, so a change to a serialised type cannot land without the
//! TypeScript following it.

const TYPESCRIPT_SETTINGS_PATH: &str = "../../app/src/models/types.ts";

fn main() {
    println!("cargo:rerun-if-changed=../core/src/types.rs");
    println!("cargo:rerun-if-changed=../core/src/settings.rs");
    typescriptify();
}

fn typescriptify() {
    use factorio_bot_core::settings::RestApiSettings;
    use factorio_bot_core::settings::*;
    use factorio_bot_core::types::*;
    use std::fs;
    use typescript_definitions::TypeScriptifyTrait;

    let existing = fs::read_to_string(TYPESCRIPT_SETTINGS_PATH).expect("types.ts does not exist?");
    let lines: Vec<&str> = existing.split('\n').collect();
    let autogenerated_marker = lines
        .iter()
        .position(|line| line.contains("AUTOGENERATED"))
        .expect("AUTOGENERATED marker not found");

    let mut output = String::from(&lines[0..=autogenerated_marker].join("\n")) + "\n";
    output += "export type PlayerId = number;\n";
    output += &InventoryItemWithQuality::type_script_ify();
    output += &FactorioFluidBoxPrototype::type_script_ify();
    output += &FactorioFluidBoxConnection::type_script_ify();
    output += &FactorioBlueprintInfo::type_script_ify();
    output += &PlayerChangedDistanceEvent::type_script_ify();
    output += &PlayerChangedPositionEvent::type_script_ify();
    output += &PlayerChangedMainInventoryEvent::type_script_ify();
    output += &PlayerLeftEvent::type_script_ify();
    output += &RequestEntity::type_script_ify();
    output += &FactorioTile::type_script_ify();
    output += &FactorioTechnology::type_script_ify();
    output += &FactorioForce::type_script_ify();
    output += &InventoryResponse::type_script_ify();
    output += &FactorioRecipe::type_script_ify();
    output += &PlaceEntityResult::type_script_ify();
    output += &PlaceEntitiesResult::type_script_ify();
    output += &FactorioIngredient::type_script_ify();
    output += &FactorioProduct::type_script_ify();
    output += &FactorioPlayer::type_script_ify();
    output += &ChunkPosition::type_script_ify();
    output += &Position::type_script_ify();
    output += &Rect::type_script_ify();
    output += &FactorioChunk::type_script_ify();
    output += &ChunkObject::type_script_ify();
    output += &ChunkResource::type_script_ify();
    output += &FactorioGraphic::type_script_ify();
    output += &FactorioEntity::type_script_ify();
    output += &FactorioEntityPrototype::type_script_ify();
    output += &FactorioItemPrototype::type_script_ify();
    output += &FactorioResult::type_script_ify();
    output += &ScriptTreeNode::type_script_ify();
    output += &FactorioSettings::type_script_ify();
    output += &RestApiSettings::type_script_ify();

    output = output.replace("DateTime<Utc>", "String");
    output = output.replace("DateTime<    Utc>", "String");
    output = output.replace("DateTime    <Utc>", "String");
    output = output.replace("NaiveDate", "String");
    output = output.replace("R64", "number");
    output = output.replace(": Value", ": object");
    output = output.replace("};", "};\n");
    while output.contains("  ") {
        output = output.replace("  ", " ");
    }
    for _ in 0..5 {
        output = output.replace("\n\n", "\n");
        output = output.replace("\r\n\r\n", "\r\n");
    }
    fs::write(TYPESCRIPT_SETTINGS_PATH, output).expect("failed to write typescript types");
}
```

Note the two deliberate changes from the original: the `#[cfg(feature = "restapi")]` gates around `RestApiSettings` are gone (this crate *is* the REST API — the feature does not exist here), and `&lines[0..autogenerated_marker + 1]` is written as the equivalent `&lines[0..=autogenerated_marker]` because clippy's pedantic lint `range_plus_one` fires on the former.

- [ ] **Step 3: Give `crates/server` the build dependency**

Append to `crates/server/Cargo.toml`:

```toml
[build-dependencies]
factorio-bot-core = { path = "../core" }
typescript-definitions = { version = "0.1", package = "typescript-definitions-ufo-patch", features = ["export-typescript"] }
```

- [ ] **Step 4: Take `typescriptify` out of the Tauri build script**

Re-read `app/src-tauri/build.rs` (it may have moved again), then delete the entire `#[cfg(feature = "gui")] fn typescriptify() { ... }` function and the `typescriptify();` call inside the `#[cfg(feature = "gui")]` block in `main()`, and remove the `typescript-definitions` and `factorio-bot-core` entries from `[build-dependencies]` in `app/src-tauri/Cargo.toml`. Leave `luaify()`, the Windows resource block, the `mobile` cfg declaration and the `rerun-if-changed` lines untouched — Task 14 handles the rest.

- [ ] **Step 5: Prove the generation still runs**

```bash
git diff --stat app/src/models/types.ts   # expect: no changes yet
touch crates/core/src/types.rs
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build -p factorio-bot-server'
git diff --stat app/src/models/types.ts
```

Expected: `types.ts` is byte-identical (no diff) — the move is a refactor, not a regeneration change. If it differs, inspect the diff before committing; a non-empty diff means the two versions of the generator disagree.

- [ ] **Step 6: Lint and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all -- --check && cargo clippy --workspace --all-features --all-targets -- --deny warnings'
git add crates/server/build.rs crates/server/Cargo.toml app/src-tauri/build.rs app/src-tauri/Cargo.toml
git commit -m "build(server): own the typescript type generation so it survives the gui removal"
```

---

## Task 14: Delete Tauri from the Rust side

**Files:**
- Delete: `app/src-tauri/src/gui/` (9 files), `app/src-tauri/src/repl/gui.rs`, `app/src-tauri/tauri.conf.json`, `app/src-tauri/capabilities/`, `app/src-tauri/gen/`, `app/src-tauri/icons/`
- Modify: `app/src-tauri/src/lib.rs`, `app/src-tauri/src/repl/mod.rs`, `app/src-tauri/src/settings.rs`, `app/src-tauri/src/paths.rs`, `app/src-tauri/src/cli/serve.rs`, `app/src-tauri/build.rs`, `app/src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: nothing from earlier tasks except that Task 13 has already moved `typescriptify()` out.
- Produces: `app_lib::run()` with no `gui` branch; default features `["cli", "repl", "lua", "restapi"]`.

**Not renaming the crate directory.** `app/src-tauri` keeps its name and path. The spec calls for moving it to `crates/cli`, but a directory move rewrites every path that three other agents currently hold uncommitted work against (`build.rs` here, plus `crates/executor` and `crates/scripting_lua` neighbours in the same workspace). Record the rename as a follow-up; it is a mechanical `git mv` plus one `Cargo.toml` member line once the tree is quiet.

- [ ] **Step 1: Delete the GUI**

```bash
git rm -r app/src-tauri/src/gui
git rm app/src-tauri/src/repl/gui.rs
git rm app/src-tauri/tauri.conf.json
git rm -r app/src-tauri/capabilities app/src-tauri/gen app/src-tauri/icons
```

- [ ] **Step 2: Fix `lib.rs`**

Delete these two lines:

```rust
#[cfg(feature = "gui")]
mod gui;
```

Delete the attribute above `pub fn run()`:

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
```

Replace the whole tail of `run()` — from `#[cfg(feature = "gui")]` through the closing brace of the `#[cfg(not(feature = "gui"))]` block — with:

```rust
  #[cfg(feature = "repl")]
  {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async { repl::start(context.clone()) })
      .expect("repl failed");
  }
  #[cfg(all(not(feature = "cli"), not(feature = "repl")))]
  {
    panic!("select at least one feature of cli or repl");
  }
```

then correct the `block_on` line to await properly:

```rust
    rt.block_on(async { repl::start(context.clone()).await })
      .expect("repl failed");
```

Also fix the two `#[cfg]` gates inside the `#[cfg(feature = "cli")]` block that mention `gui`:

```rust
    #[cfg(not(feature = "repl"))]
    {
      if app.is_none() {
        return;
      }
      app
        .expect("checked before")
        .print_help()
        .expect("failed to print_help");
      return;
    }
    #[cfg(feature = "repl")]
    {
      if let Some(mut app) = app {
        // No subcommand was run, show help and let the REPL take over
        app.print_help().expect("failed to print_help");
      }
      // If app is None, a subcommand ran - continue to the REPL
    }
```

- [ ] **Step 3: Fix `repl/mod.rs`**

Delete these lines:

```rust
#[cfg(all(debug_assertions, feature = "gui"))]
mod gui;
```

and inside `subcommands()`:

```rust
    #[cfg(all(debug_assertions, feature = "gui"))]
    gui::build(),
```

- [ ] **Step 4: Fix the two re-export shims**

`app/src-tauri/src/settings.rs` becomes:

```rust
//! Re-exported from `factorio_bot_core::app_settings` so the existing
//! `crate::settings::` call sites keep compiling.
pub use factorio_bot_core::app_settings::{load_app_settings, SharedAppSettings};
```

`app/src-tauri/src/paths.rs` becomes:

```rust
//! Re-exported from `factorio_bot_core::paths` so the existing
//! `crate::paths::` call sites keep compiling.
pub use factorio_bot_core::paths::{data_local_dir, workspace_dir};
```

(`AppSettings` and `settings_file` were named only by the GUI's settings commands; `crates/server` reaches for `factorio_bot_core::paths::settings_file()` directly.)

- [ ] **Step 5: Fix the `serve` command's exit comment**

In `app/src-tauri/src/cli/serve.rs`, `run()` ends with `std::process::exit(0)` guarded by a comment saying "plan 4 deletes the GUI ... at which point this should become proper control flow". That is now. Replace the `std::process::exit(0);` and its comment block with:

```rust
  // `run()` falls through into the REPL after any subcommand, which is right
  // for setup commands like `start` that intentionally hand off to an
  // interactive session. `serve` is different: by the time
  // `start_with_shutdown` returns, the Factorio instance is stopped and the
  // user asked (Ctrl-C / SIGTERM) for the process to end. Falling through
  // would start a REPL with no TTY behind it. Exit explicitly.
  std::process::exit(0);
```

- [ ] **Step 6: Strip `build.rs`**

`app/src-tauri/build.rs` becomes:

```rust
fn main() {
  luaify();
  #[cfg(windows)]
  {
    // set .exe file properties
    let mut res = winres::WindowsResource::new();
    res.set("ProductName", "Factorio-Bot");
    res.set("FileDescription", "Factorio-Bot");
    res.set("Version", env!("CARGO_PKG_VERSION"));
    res.set("LegalCopyright", "Copyright (C) 2022");
    res
      .compile()
      .expect("Failed to run the Windows resource compiler (rc.exe)");
  }
  println!("cargo:rerun-if-changed=../../crates/scripting_lua/src/");
}

fn luaify() {
  #[cfg(feature = "lua")]
  {
    use factorio_bot_scripting_lua::lua_docs::write_lua_docs;
    let path = std::path::Path::new(&format!(
      "{}/../../docs/lua/src/",
      env!("CARGO_MANIFEST_DIR")
    ))
    .to_path_buf();
    write_lua_docs(path).expect("Failed to write lua docs");
  }
}
```

`res.set_icon("icons/icon.ico")` is gone with `icons/`, the `mobile` cfg declaration is gone with the `tauri::mobile_entry_point` attribute that needed it, and the `types.rs` rerun-if-changed moved to `crates/server/build.rs` with the generator.

- [ ] **Step 7: Strip `Cargo.toml`**

In `app/src-tauri/Cargo.toml`:

Delete the dependencies `tauri`, `open`, `port_scanner`, `serde_json`, `tauri-plugin-fs`, `tauri-plugin-http`, `tauri-plugin-dialog`, `tauri-plugin-log`, `log`, and the entire `[target.'cfg(not(any(target_os = "android", target_os = "ios")))'.dependencies]` section holding `tauri-plugin-updater`.

Delete the `tauri-build` entry from `[build-dependencies]` (the `typescript-definitions` and `factorio-bot-core` entries went in Task 13), leaving:

```toml
[build-dependencies]
serde = { version = "1.0", features = [ "derive" ] }
```

Replace the `[features]` block with:

```toml
[features]
default = ["restapi", "repl", "cli", "lua"]
cli = ["dep:clap", "dep:clap_complete"]
restapi = ["dep:factorio-bot-server"]
lua = ["dep:factorio-bot-scripting-lua"]
repl = ["reedline-repl-rs/async"]
tokio-console = ["dep:console-subscriber"]
```

Also delete the `[lib] crate-type = ["staticlib", "cdylib", "rlib"]` line's `"staticlib"` and `"cdylib"` entries — they existed for Tauri's mobile targets:

```toml
[lib]
name = "app_lib"
crate-type = ["rlib"]
```

- [ ] **Step 8: Build every feature combination**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --all-features && cargo build --no-default-features --features cli,lua && cargo build --no-default-features --features repl'
```

Expected: all three succeed. Any "unused import" or "never used" warning names something else that only the GUI reached; delete it rather than allow-ing it.

- [ ] **Step 9: Confirm Tauri is gone from the dependency graph**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo tree --workspace --all-features -i tauri'
```

Expected: `error: package ID specification 'tauri' did not match any packages`.

- [ ] **Step 10: Test, lint, commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all && cargo fmt --all -- --check && cargo clippy --workspace --all-features --all-targets -- --deny warnings && cargo test --workspace'
git add -A app/src-tauri
git commit -m "feat!: remove the tauri desktop shell; the app is browser-only"
```

---

## Task 15: Nix devShell, CI and justfile

**Files:**
- Modify: `flake.nix`
- Modify: `.github/workflows/test.yml`
- Modify: `.github/workflows/publish.yml`
- Modify: `justfile`

**Interfaces:**
- Consumes: the Tauri-free Cargo workspace (Task 14) and the Tauri-free `package.json` (Task 12).
- Produces: `CHROMIUM_BIN` set in the devShell, consumed by Task 16's `app/e2e/smoke.mjs`.

- [ ] **Step 1: Slim the devShell and add Chromium**

Replace the `libs` binding and `mkShell` in `flake.nix` with:

```nix
          libs = with pkgs; [
            lua5_4 # mlua links against system lua 5.4
            openssl
            xz # liblzma, loaded at runtime by the compiled binaries
          ];
        in {
          # Only native/system libraries live here; the language toolchains are
          # pinned in mise.toml (rust, node, pnpm).
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [ pkg-config ]
              ++ lib.optionals stdenv.hostPlatform.isLinux [ patchelf file chromium ];

            buildInputs = libs;

            # build scripts and the built binaries load lua at runtime, and
            # nothing puts the nix store paths into their rpath
            LD_LIBRARY_PATH = lib.makeLibraryPath libs;

            # app/e2e/smoke.mjs drives this through playwright-core, which
            # never downloads a browser of its own.
            CHROMIUM_BIN =
              if stdenv.hostPlatform.isLinux then "${pkgs.chromium}/bin/chromium" else "";
          };
        });
```

The webkitgtk/gtk3/gtksourceview3/libsoup/glib/cairo/pango/gdk-pixbuf/atk/librsvg/fuse list existed only so `tauri` and its AppImage bundler could link and run.

- [ ] **Step 2: Verify the shell still builds the workspace**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --all-features && echo "$CHROMIUM_BIN"'
```

Expected: build succeeds and `CHROMIUM_BIN` prints a `/nix/store/...` path on Linux.

- [ ] **Step 3: Fix `.github/workflows/test.yml`**

Delete the "🛠️ Install tauri-cli" step (lines 57-59). Replace the apt line in the "Install webkit2gtk (for tauri)" step and rename the step:

```yaml
      - name: 🛠️ Install lua and build tools (Ubuntu)
        if: matrix.platform == 'ubuntu-latest'
        run: |
          sudo apt-get update
          sudo apt-get install -y liblua5.4-dev pkg-config
```

Replace the "🏗️ Build rust code" step with:

```yaml
      - name: 🏗️ Build rust code
        env:
          # Disable incremental compilation to prevent LNK1123/CVT1100 errors on Windows
          CARGO_INCREMENTAL: 0
        run: |
          cargo build --release --all-features
```

Move the "🏗️ Build javascript code" step so it runs *before* the Rust build is not required — it already does — and leave it as is.

- [ ] **Step 4: Fix `.github/workflows/publish.yml`**

Delete the "🛠️ Install tauri-cli" step, the "🛠️ Install webkit2gtk (for tauri)" step, the "🏗️ Build tauri updater json" step, and the `tauri-apps/tauri-action@v0` step. Replace the `cd app && pnpm run tauri:build` step with:

```yaml
      - name: 🏗️ Build the web ui
        run: |
          cd app && pnpm run build:web
      - name: 🏗️ Build the server binary
        run: |
          cargo build --release --all-features
```

Note for the implementer: the desktop auto-updater has no server equivalent. Release artefacts become the plain `factorio-bot` binary plus the built `app/dist`; updating is "replace the binary". If a release job step exists that uploads `.AppImage`/`.dmg`/`.msi`, replace its glob with `target/release/factorio-bot*`.

- [ ] **Step 5: Fix the justfile**

Replace the `start` recipe and add a `serve` recipe:

```make
# Vite dev server on :8080, proxying /api to a `just serve` on :7492
start:
    cd app; pnpm run start

# the real thing: axum serving the built SPA and the API on :7492
serve *ARGS:
    cargo run --release --no-default-features --features cli,lua,restapi -- serve --web-root app/dist {{ARGS}}
```

- [ ] **Step 6: Verify CI's commands locally**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm install --frozen-lockfile && pnpm run build:web && pnpm run lint && pnpm run test:coverage'
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --release --all-features && cargo test --workspace'
```

Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add flake.nix .github/workflows/test.yml .github/workflows/publish.yml justfile
git commit -m "ci: drop the tauri toolchain from the devshell, workflows and justfile"
```

---

## Task 16: The coverage floor for the transport swap

The frontend has exactly one test today (`src/dummy.spec.ts`, `expect(1 + 1).toBe(2)`). This task states, and enforces, the minimum that makes swapping every backend call safe — **not** a general testing initiative for the app. Views, routing and PrimeVue components stay untested here.

**The floor, as a claim:** every function in `app/src/api/**` and every action in `app/src/store/**` is exercised at least once, on its happy path *and* on its failure path, with `fetch` and `EventSource` stubbed. Those two directories are the entire blast radius of the transport swap; everything else in `app/src/` was already untested before this plan and is no worse after it.

**Files:**
- Modify: `app/vite.config.mts`
- Delete: `app/src/dummy.spec.ts`

**Interfaces:**
- Consumes: the spec files from Tasks 2, 3, 4, 5, 6, 7, 8, 9.
- Produces: `pnpm run test:coverage` fails when `src/api/**` or `src/store/**` drops below the thresholds.

- [ ] **Step 1: Check the current numbers**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run --coverage --coverage.reporter=text'
```

Write down the lines/functions/branches percentages for `src/api` and `src/store`.

- [ ] **Step 2: Scope coverage to the transport and set thresholds**

In `app/vite.config.mts`, replace the `test` block with:

```ts
    test: {
        environment: 'node',
        coverage: {
            reporter: ['html-spa', 'cobertura', 'text'],
            // The transport swap's blast radius, and nothing else. Views and
            // routing were untested before this change and are out of scope
            // here; widening `include` without writing the tests first would
            // just move the thresholds down to meaninglessness.
            include: ['src/api/**/*.ts', 'src/store/**/*.ts'],
            exclude: ['src/api/openapi.snapshot.json', '**/*.spec.ts'],
            thresholds: {
                lines: 90,
                functions: 90,
                statements: 90,
                branches: 80
            }
        }
    },
```

If Step 1's numbers came in below a threshold, do **not** lower the threshold: add the missing case as a test. The gaps to expect are the `catch` arms — `_updateSettings` with no settings loaded, `stopInstances` when the stop fails, `loadScriptFile` on a 404 — and each is three lines of test.

- [ ] **Step 3: Delete the placeholder test**

```bash
git rm app/src/dummy.spec.ts
```

- [ ] **Step 4: Run and confirm the gate holds**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run test:coverage'
```

Expected: PASS with no threshold error.

- [ ] **Step 5: Prove the gate is load-bearing (mutation)**

Comment out the `it('rejects an empty command without touching the api')` case in `src/store/rconStore.spec.ts` and re-run. Expect a branch-coverage threshold failure. Restore it.

- [ ] **Step 6: Commit**

```bash
git add app/vite.config.mts
git commit -m "test(app): gate the transport layer at 90% lines and 80% branches"
```

---

## Task 17: Prove it works in a browser with no Tauri present

Compiling is not evidence. This runs the real server, loads the real SPA in a real headless Chromium, and screenshots three pages.

**Files:**
- Create: `app/e2e/smoke.mjs`
- Create: `app/e2e/.gitignore`
- Modify: `app/package.json`

**Interfaces:**
- Consumes: `CHROMIUM_BIN` from the devShell (Task 15); the `serve` subcommand; the built `app/dist` (Task 12's `build:web`); `POST /api/v1/instance/start` (Task 1).
- Produces: `app/e2e/screenshots/{dashboard,settings,script}.png`; exit code 0 on success, 1 with a printed reason otherwise.

- [ ] **Step 1: Add the driver**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm add -D playwright-core'
```

`playwright-core` rather than `playwright`: it never downloads a browser, and the devShell already provides one.

- [ ] **Step 2: Keep screenshots out of git**

`app/e2e/.gitignore`:

```
screenshots/
```

- [ ] **Step 3: Write the smoke check**

`app/e2e/smoke.mjs`:

```js
// Headless browser check: the SPA must work against a real `factorio-bot
// serve` with no Tauri runtime anywhere. Run with `pnpm run e2e` inside the
// nix devShell (it supplies CHROMIUM_BIN).
//
// Prerequisites, both produced by earlier steps of this task:
//   pnpm run build:web
//   cargo build --release --all-features
import {chromium} from 'playwright-core';
import {spawn} from 'node:child_process';
import {mkdirSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '..', '..');
const binary = path.join(repoRoot, 'target', 'release', 'factorio-bot');
const webRoot = path.join(repoRoot, 'app', 'dist');
const shots = path.join(here, 'screenshots');
const base = 'http://127.0.0.1:7492';

mkdirSync(shots, {recursive: true});

function fail(message) {
    console.error('FAIL: ' + message);
    process.exitCode = 1;
}

async function waitForHealth(timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        try {
            const response = await fetch(base + '/api/v1/health');
            if (response.ok) {
                return true;
            }
        } catch {
            // not listening yet
        }
        await new Promise(resolve => setTimeout(resolve, 200));
    }
    return false;
}

const server = spawn(binary, ['serve', '--bind', '127.0.0.1:7492', '--web-root', webRoot], {
    cwd: repoRoot,
    stdio: ['ignore', 'inherit', 'inherit']
});

let browser;
try {
    if (!await waitForHealth(30000)) {
        fail('the server never answered /api/v1/health');
        throw new Error('server did not start');
    }

    browser = await chromium.launch({
        executablePath: process.env.CHROMIUM_BIN || undefined,
        args: ['--no-sandbox']
    });
    const page = await browser.newPage({viewport: {width: 1440, height: 900}});

    const consoleErrors = [];
    const failedApiRequests = [];
    page.on('console', message => {
        if (message.type() === 'error') {
            consoleErrors.push(message.text());
        }
    });
    page.on('response', response => {
        if (response.url().includes('/api/v1/') && response.status() >= 400) {
            failedApiRequests.push(response.status() + ' ' + response.url());
        }
    });

    // 1. The shell loads and there is no Tauri runtime in the page.
    await page.goto(base + '/#/', {waitUntil: 'networkidle'});
    const tauriPresent = await page.evaluate(
        () => typeof window.__TAURI__ !== 'undefined' ||
              typeof window.__TAURI_INTERNALS__ !== 'undefined'
    );
    if (tauriPresent) {
        fail('a Tauri runtime object is present in the page');
    }
    await page.screenshot({path: path.join(shots, 'dashboard.png'), fullPage: true});

    // 2. Settings round-tripped through GET /api/v1/settings: the workspace
    //    path input is populated from the server, which a stubbed or failed
    //    fetch could not do.
    await page.goto(base + '/#/settings', {waitUntil: 'networkidle'});
    await page.waitForSelector('input', {timeout: 10000});
    const inputValues = await page.$$eval('input', nodes => nodes.map(n => n.value));
    if (!inputValues.some(value => typeof value === 'string' && value.length > 0)) {
        fail('no settings value reached the settings page: ' + JSON.stringify(inputValues));
    }
    await page.screenshot({path: path.join(shots, 'settings.png'), fullPage: true});

    // 3. The script tree rendered, i.e. GET /api/v1/scripts answered.
    await page.goto(base + '/#/script', {waitUntil: 'networkidle'});
    await page.waitForSelector('.p-tree', {timeout: 10000}).catch(() => {
        fail('the script tree never rendered');
    });
    await page.screenshot({path: path.join(shots, 'script.png'), fullPage: true});

    if (consoleErrors.length > 0) {
        fail('console errors: ' + JSON.stringify(consoleErrors, null, 2));
    }
    if (failedApiRequests.length > 0) {
        fail('failing api requests: ' + JSON.stringify(failedApiRequests, null, 2));
    }

    if (process.exitCode !== 1) {
        console.log('OK: browser smoke check passed; screenshots in ' + shots);
    }
} catch (err) {
    fail(err instanceof Error ? err.message : String(err));
} finally {
    if (browser) {
        await browser.close();
    }
    server.kill('SIGTERM');
}
```

- [ ] **Step 4: Build both halves and run it**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run build:web'
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --release --all-features'
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run e2e'
```

Expected: `OK: browser smoke check passed`, and three PNGs in `app/e2e/screenshots/`.

If the settings check fails with an empty `workspace_path`, the server has no settings file yet: run `cargo run --release --all-features -- serve` once by hand, save the settings page, stop it, and re-run. If the script-tree check fails with a 400 "missing scripts directory", `serve`'s bootstrap did not create `workspace/scripts` — that is a `cli/serve.rs` bug, not a frontend one; fix it there.

- [ ] **Step 5: Look at the screenshots**

Open all three PNGs. Confirm the sidebar, the topbar and page content render, and that the pages show *data* (a workspace path, a script tree), not empty shells. They will be laid out as stacked blocks rather than a grid — that is the known PrimeFlex gap and is the next plan's problem, not a failure of this one.

- [ ] **Step 6: Commit**

```bash
git add app/e2e app/package.json app/pnpm-lock.yaml
git commit -m "test(app): add a headless browser smoke check with no tauri present"
```

---

## Follow-up work this plan deliberately does not do

Record these; do not start them here.

1. **Server-side autostart.** `settings.gui.enable_autostart` is persisted and inert. The trigger belonged in `App.vue`'s `onMounted` and could not survive the move to a browser (N tabs, N start requests). It belongs in `cli/serve.rs` at startup, calling the same code path as `POST /api/v1/instance/start`.
2. **Renaming `app/src-tauri` to `crates/cli`.** Mechanical, but it collides with three agents' uncommitted work right now.
3. **The PrimeFlex layout gap.** `p-grid`, `p-col-12`, `p-formgrid`, `p-field-radiobutton` resolve to no rules; those pages stack. PrimeFlex is archived upstream. The next plan is a shadcn-vue/Tailwind redesign.
4. **Create / delete script UI.** `POST` and `DELETE /api/v1/scripts/file` exist and `client.ts` wraps them; no component calls them.
5. **The RCON reply is still discarded.** `POST /api/v1/rcon` answers `204`; surfacing the server's response text is a server change plus a UI change.
6. **`GET /api/v1/jobs` has no UI.** A job history panel would be a natural home for it.
7. **Domain types reach the browser through two independent generators.** `typescript_definitions` emits `app/src/models/types.ts` from the Rust structs (now via `crates/server/build.rs`), and `utoipa` emits the same structs into `/openapi.json`. Neither knows about the other, so a `#[serde(rename)]` visible to one and not the other would drift silently. Task 5's contract test catches drift on the handful of shapes the client depends on; unifying the two generators (emit TypeScript *from* the OpenAPI document, or drop `typescript-definitions` in favour of it) is a separate decision.
8. **The seven placeholder routes.** Listed in "Scope: the app is four pages" above. The redesign plan decides what replaces them.
