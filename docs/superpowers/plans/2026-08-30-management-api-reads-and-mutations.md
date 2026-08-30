# Management API — Reads and Mutations (Plan 3 of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose over HTTP every management operation that completes quickly — settings, instance status and stop, RCON, script browsing and editing, path checks — so the Vue app can stop depending on Tauri IPC for anything but the long-running operations.

**Architecture:** The surviving `#[tauri::command]` handlers move into `crates/server` as axum routes under `/api/v1`, reading the same `SharedAppSettings` and `SharedFactorioInstance` the CLI already holds. The script helpers they depend on move down into `crates/core` first, as the settings did in plan 2. Long-running operations — starting instances and executing scripts — are deliberately excluded; they need the job registry and SSE, which is plan 4.

**Tech Stack:** Rust 2021, axum 0.8, tokio, utoipa, miette.

**Spec:** `docs/superpowers/specs/2026-08-29-webserver-replaces-tauri-gui-design.md`

**Plans:** 1 (axum replaces Rocket) and 2 (shared settings, `serve`) are complete. This is 3 of 5. Plan 4 is the job registry, SSE and script execution. Plan 5 is the frontend transport swap and deleting Tauri. A separate UI redesign follows.

## Global Constraints

- Workspace root `/home/arturh/projects/private/factorio-bot`. Rust edition 2021. Branch: `master`, committing directly — no feature branches.
- **Every cargo command runs inside the Nix devShell with mise on PATH:**
  `nix develop --command bash -c 'eval "$(mise env -s bash)"; <command>'`
- **`cargo fmt -p <crate>` only, never `cargo fmt --all`** — another agent owns crates with pre-existing drift in this shared checkout.
- **Commit with `git commit -- <explicit paths>`, never `git add -A` or a bare `git commit`.** The git index is shared with another agent.
- Lint gate: `cargo clippy -p factorio-bot-core -p factorio-bot-server -p factorio-bot --all-features --all-targets -- --deny warnings --deny deprecated`. Not `--workspace`: `crates/planner` belongs to another agent and its state fluctuates.
- Do not touch anything under `crates/planner/`.
- **No `.unwrap()`, `.expect()`, or byte-slicing on user-supplied input in crate source.** The workspace sets `panic = "abort"` for release, so any of those aborts the whole process. This plan ports code that violates this in several places; fixing them is part of the work, not incidental.
- Route paths kebab-case under `/api/v1`; JSON body and query field names snake_case. Mutations are `POST`/`PUT`, reads are `GET`.
- Every new handler carries a `#[utoipa::path]` attribute and is registered with `.routes(routes!(...))` so it appears in `/openapi.json`.
- The 32 existing `crates/server` tests and the core tests must keep passing.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/src/scripts.rs` | New. `scripts_dir(workspace: &Path) -> Result<PathBuf>` and `resolve_script_path(root, requested) -> Result<PathBuf>`. Moved from the binary's `scripting.rs`, with the panics removed and traversal properly checked. |
| `crates/server/src/manage/mod.rs` | New. Router for the management routes. |
| `crates/server/src/manage/settings.rs` | New. `GET`/`PUT /api/v1/settings`. |
| `crates/server/src/manage/instance.rs` | New. `GET /api/v1/instance`, `POST /api/v1/instance/stop`. |
| `crates/server/src/manage/rcon.rs` | New. `POST /api/v1/rcon`. |
| `crates/server/src/manage/scripts.rs` | New. `GET /api/v1/scripts`, `GET`/`PUT /api/v1/scripts/file`. |
| `crates/server/src/manage/fs.rs` | New. `GET /api/v1/fs/exists`. |
| `app/src-tauri/src/cli/serve.rs` | Modified. Gains `--web-root`. |
| `crates/server/src/webserver.rs` | Modified. Mounts the management router; shutdown gains a timeout. |

The management routes are split by resource rather than gathered in one file, matching how `game/{query,control}.rs` is already organised. Each file stays small enough to hold in view.

---

### Task 1: Move the script-path helpers into core and make them safe

**Files:**
- Create: `crates/core/src/scripts.rs`
- Modify: `crates/core/src/lib.rs`, `app/src-tauri/src/scripting.rs`
- Test: `crates/core/src/scripts.rs` (inline `#[cfg(test)]` module)

**Interfaces:**
- Produces: `factorio_bot_core::scripts::{scripts_dir, resolve_script_path}`.
  - `pub fn scripts_dir(workspace_path: &Path) -> Result<PathBuf>` — miette `Result`.
  - `pub fn resolve_script_path(root: &Path, requested: &str) -> Result<PathBuf>` — resolves a user-supplied path against the scripts root, refusing anything that escapes it.

`app/src-tauri/src/scripting.rs:90-128`'s `prepare_workspace_scripts` is the source. It has three problems this task fixes:

1. Two `.expect("Failed to canonicalize …")` calls that abort the process on an I/O error.
2. Callers do `&path[1..]` to strip a leading `/` (`gui/command/script.rs:104,157,191`). That is **byte** slicing: an empty string panics, and so does any path whose first character is multi-byte.
3. The traversal guard is `path.contains("..")`, a substring test. It rejects the legitimate filename `my..script.lua` and, more importantly, is the kind of check that gets fooled by encoding. With the server reachable over a network this becomes a real boundary, so the resolved path is canonicalized and checked against the root instead.

- [ ] **Step 1: Write the failing tests**

Create `crates/core/src/scripts.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("scripts");
        fs::create_dir_all(root.join("sub")).expect("mkdir");
        fs::write(root.join("a.lua"), "-- a").expect("write");
        fs::write(root.join("sub").join("b.lua"), "-- b").expect("write");
        let canonical = fs::canonicalize(&root).expect("canonicalize");
        (dir, canonical)
    }

    #[test]
    fn resolves_a_path_with_a_leading_slash() {
        let (_dir, root) = root();
        let resolved = resolve_script_path(&root, "/a.lua").expect("resolves");
        assert_eq!(resolved, root.join("a.lua"));
    }

    #[test]
    fn resolves_a_nested_path() {
        let (_dir, root) = root();
        let resolved = resolve_script_path(&root, "/sub/b.lua").expect("resolves");
        assert_eq!(resolved, root.join("sub").join("b.lua"));
    }

    #[test]
    fn resolves_a_path_without_a_leading_slash() {
        let (_dir, root) = root();
        let resolved = resolve_script_path(&root, "a.lua").expect("resolves");
        assert_eq!(resolved, root.join("a.lua"));
    }

    /// The old caller did `&path[1..]`, which panics on an empty string.
    #[test]
    fn rejects_an_empty_path_without_panicking() {
        let (_dir, root) = root();
        assert!(resolve_script_path(&root, "").is_err());
    }

    /// The old caller did `&path[1..]`, which panics when the first character
    /// is multi-byte.
    #[test]
    fn rejects_a_multibyte_path_without_panicking() {
        let (_dir, root) = root();
        let result = resolve_script_path(&root, "ä.lua");
        assert!(result.is_err(), "expected a miss, got {result:?}");
    }

    #[test]
    fn refuses_to_escape_the_root() {
        let (_dir, root) = root();
        for attempt in ["../outside.lua", "/../outside.lua", "sub/../../outside.lua"] {
            assert!(
                resolve_script_path(&root, attempt).is_err(),
                "{attempt} should not resolve"
            );
        }
    }

    /// A substring check on ".." rejects this legitimate name; a canonicalising
    /// check accepts it.
    #[test]
    fn accepts_a_filename_containing_two_dots() {
        let (_dir, root) = root();
        fs::write(root.join("my..script.lua"), "-- ok").expect("write");
        let resolved = resolve_script_path(&root, "/my..script.lua").expect("resolves");
        assert_eq!(resolved, root.join("my..script.lua"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core scripts'`
Expected: FAIL — the module is not declared and the functions do not exist.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/scripts.rs`:

```rust
use miette::{miette, IntoDiagnostic, Result};
use std::path::{Path, PathBuf};

/// Locates the directory holding user Lua scripts.
///
/// Development checkouts keep them at the repository root; an installed copy
/// keeps them under the workspace. The development paths are resolved against
/// the current working directory, which is why a server started from an
/// arbitrary directory falls back to the workspace copy.
pub fn scripts_dir(workspace_path: &Path) -> Result<PathBuf> {
    for candidate in [PathBuf::from("./scripts"), PathBuf::from("../../scripts")] {
        if candidate.exists() {
            return std::fs::canonicalize(candidate).into_diagnostic();
        }
    }

    let workspace_scripts = workspace_path.join("scripts");
    if workspace_scripts.exists() {
        return std::fs::canonicalize(workspace_scripts).into_diagnostic();
    }

    #[cfg(not(debug_assertions))]
    {
        std::fs::create_dir_all(&workspace_scripts).into_diagnostic()?;
        crate::process::instance_setup::PLANS_CONTENT
            .extract(workspace_scripts.clone())
            .map_err(|err| miette!("failed to extract bundled scripts: {err:?}"))?;
        return std::fs::canonicalize(workspace_scripts).into_diagnostic();
    }

    #[cfg(debug_assertions)]
    Err(miette!(
        "missing scripts/ directory: {}",
        workspace_scripts.display()
    ))
}

/// Resolves a client-supplied script path against the scripts root.
///
/// The path may or may not carry a leading `/`. The result is canonicalised and
/// verified to live under `root`, so `..` segments cannot escape it — this is a
/// network-reachable boundary, not a local convenience.
pub fn resolve_script_path(root: &Path, requested: &str) -> Result<PathBuf> {
    let relative = requested.trim_start_matches('/');
    if relative.is_empty() {
        return Err(miette!("empty path"));
    }

    let joined = root.join(relative);
    // canonicalize resolves `..` and symlinks, and fails if the target is absent
    let canonical = std::fs::canonicalize(&joined)
        .into_diagnostic()
        .map_err(|_| miette!("path not found: {requested}"))?;

    if !canonical.starts_with(root) {
        return Err(miette!("path escapes the scripts directory: {requested}"));
    }
    Ok(canonical)
}
```

Add `pub mod scripts;` to `crates/core/src/lib.rs`. Add `tempfile = "3"` to `crates/core`'s `[dev-dependencies]` if plan 2 did not already add it.

Note `resolve_script_path` requires the target to exist, because `canonicalize` does. That matches the existing handlers, which all check `exists()` and error otherwise — including `save_script`, which cannot create new files today. Preserving that is deliberate; changing it is a separate decision.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core scripts'`
Expected: PASS — seven tests.

- [ ] **Step 5: Point the binary at core**

In `app/src-tauri/src/scripting.rs`, delete `prepare_workspace_scripts` and replace its uses with `factorio_bot_core::scripts::scripts_dir`. Its callers are in `app/src-tauri/src/gui/command/script.rs` and expect `Result<PathBuf, String>`, so map the error: `.map_err(|e| format!("{e}"))`. Leave the rest of those handlers alone — they are deleted in plan 5.

- [ ] **Step 6: Verify the workspace builds**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --all-features'`
Expected: success.

- [ ] **Step 7: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-core'
git commit -- crates/core/src/scripts.rs crates/core/src/lib.rs crates/core/Cargo.toml app/src-tauri/src/scripting.rs -m "refactor(core): move script path resolution into core and make it panic-free"
```

---

### Task 2: Settings endpoints

**Files:**
- Create: `crates/server/src/manage/mod.rs`, `crates/server/src/manage/settings.rs`
- Modify: `crates/server/src/lib.rs`, `crates/server/src/webserver.rs`
- Test: `crates/server/tests/manage_settings.rs`

**Interfaces:**
- Consumes: `AppState { instance, settings }`, `ApiResult<T>`, `ErrorResponse`, `ApiJson<T>` (from `crates/server/src/extract.rs`).
- Produces: `crate::manage::router() -> OpenApiRouter<AppState>`, merged in `build_router`; `GET /api/v1/settings` returning `AppSettings`; `PUT /api/v1/settings` accepting `AppSettings`.

`PUT` replaces the whole settings object and persists it, mirroring today's `update_settings` (`app/src-tauri/src/gui/command/settings.rs:23-31`), which assigns and then calls `AppSettings::save`. The separate `save_settings` command has no caller and is not ported.

- [ ] **Step 1: Write the failing tests**

Create `crates/server/tests/manage_settings.rs`:

```rust
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        instance: FactorioInstance::new_shared(),
        settings: AppSettings::default().into_shared(),
    }
}

#[tokio::test]
async fn get_settings_returns_the_current_settings() {
    let response = build_router(test_state(), None)
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let settings: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(settings["restapi"]["port"], 7492);
    assert!(settings.get("factorio").is_some());
}

#[tokio::test]
async fn put_settings_updates_the_shared_state() {
    let state = test_state();
    let mut updated = AppSettings::default();
    updated.factorio.client_count = 4;
    let body = serde_json::to_string(&updated).unwrap();

    let response = build_router(state.clone(), None)
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "got {}",
        response.status()
    );
    assert_eq!(state.settings.read().await.factorio.client_count, 4);
}

#[tokio::test]
async fn put_settings_rejects_a_malformed_body_with_json() {
    let response = build_router(test_state(), None)
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"factorio\": 12}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("has a content type")
        .to_str()
        .unwrap();
    assert!(content_type.starts_with("application/json"), "{content_type}");
}
```

**Note the `build_router(state, None)` signature** — plan 2's fix wave changed it to take the web root as a second argument. Check its current shape in `crates/server/src/webserver.rs` and match it.

`AppSettings` must implement `utoipa::ToSchema` for the `#[utoipa::path]` annotations. It does not yet; add the derive alongside its existing ones in `crates/core/src/app_settings.rs`, as plan 1 did for the game types, and to `GuiSettings`, `FactorioSettings` and `RestApiSettings` if the compiler asks.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test manage_settings'`
Expected: FAIL — 404, the routes do not exist.

- [ ] **Step 3: Write the handlers**

Create `crates/server/src/manage/settings.rs`:

```rust
use crate::error::{ApiResult, ErrorResponse};
use crate::extract::ApiJson;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::paths::settings_file;

/// Returns the current application settings
#[utoipa::path(
    get,
    path = "/api/v1/settings",
    tag = "Admin",
    responses(
        (status = 200, body = AppSettings),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_settings(State(state): State<AppState>) -> ApiResult<AppSettings> {
    Ok(Json(state.settings.read().await.clone()))
}

/// Replaces the application settings and persists them
#[utoipa::path(
    put,
    path = "/api/v1/settings",
    tag = "Admin",
    request_body = AppSettings,
    responses(
        (status = 200, body = AppSettings),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn put_settings(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<AppSettings>,
) -> ApiResult<AppSettings> {
    let mut settings = state.settings.write().await;
    *settings = body;
    AppSettings::save(settings_file(), &settings).map_err(ErrorResponse::from)?;
    Ok(Json(settings.clone()))
}
```

Check `AppSettings::save`'s real signature in `crates/core/src/app_settings.rs` before writing that call — argument order and whether it takes a reference both matter.

Create `crates/server/src/manage/mod.rs`:

```rust
pub mod settings;

use crate::state::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(settings::get_settings))
        .routes(routes!(settings::put_settings))
}
```

`routes!` derives the path from each handler's `#[utoipa::path]` attribute, which carries the absolute `/api/v1/...` path — so merge this router at the root in `build_router`, exactly as `game::router()` is merged, never nested.

Add `pub mod manage;` to `crates/server/src/lib.rs` and `.merge(crate::manage::router())` to `build_router`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS — three new tests plus the existing 32.

- [ ] **Step 5: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-server -p factorio-bot-core'
git commit -- crates/server/src/manage crates/server/src/lib.rs crates/server/src/webserver.rs crates/server/tests/manage_settings.rs crates/core/src/app_settings.rs -m "feat(server): expose settings over HTTP"
```

---

### Task 3: Instance status and stop

**Files:**
- Create: `crates/server/src/manage/instance.rs`
- Modify: `crates/server/src/manage/mod.rs`
- Test: `crates/server/tests/manage_instance.rs`

**Interfaces:**
- Produces: `GET /api/v1/instance` returning `InstanceStatus { started: bool, client_count: u8, server_port: Option<u16>, rcon_port: Option<u16> }`; `POST /api/v1/instance/stop` returning `204`.

Starting instances is **not** in this task. It takes minutes on a first run and needs the job registry, which is plan 4.

The port of `stop_instances` (`app/src-tauri/src/gui/command/instances.rs:70-84`) must fix a live panic: it does `instance_state.take().unwrap().stop().unwrap()`. The second `.unwrap()` aborts the process if stopping fails, which under `panic = "abort"` takes the server with it.

- [ ] **Step 1: Write the failing tests**

Create `crates/server/tests/manage_instance.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::process::process_control::{FactorioInstance, SharedFactorioInstance};
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn state_with(instance: SharedFactorioInstance) -> AppState {
    AppState {
        instance,
        settings: AppSettings::default().into_shared(),
    }
}

#[tokio::test]
async fn instance_status_reports_stopped_when_nothing_runs() {
    let response = build_router(state_with(FactorioInstance::new_shared()), None)
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
}

#[tokio::test]
async fn stopping_when_nothing_runs_is_an_error_not_a_panic() {
    let response = build_router(state_with(FactorioInstance::new_shared()), None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/instance/stop")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn stopping_takes_the_instance_out_of_shared_state() {
    // an instance with no child processes: stop() on it is a no-op that succeeds
    let instance = Arc::new(RwLock::new(Some(FactorioInstance {
        world: Some(Arc::new(FactorioWorld::new())),
        rcon: Arc::new(FactorioRcon::new_empty()),
        server_process: None,
        client_processes: vec![],
        ..empty_instance_fields()
    })));

    let response = build_router(state_with(instance.clone()), None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/instance/stop")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success(), "got {}", response.status());
    assert!(instance.read().await.is_none(), "instance was not taken");
}
```

`FactorioInstance` has no `Default`, so `empty_instance_fields()` above is a placeholder for the real struct literal. `crates/server/tests/shutdown.rs` already builds one — copy its `empty_factorio_instance()` helper rather than inventing a second version, and drop the placeholder.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test manage_instance'`
Expected: FAIL — 404 on both routes.

- [ ] **Step 3: Write the handlers**

Create `crates/server/src/manage/instance.rs`:

```rust
use crate::error::{ApiResult, ErrorResponse};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstanceStatus {
    pub started: bool,
    pub client_count: u8,
    pub server_port: Option<u16>,
    pub rcon_port: Option<u16>,
}

/// Reports whether a Factorio instance is running
#[utoipa::path(
    get,
    path = "/api/v1/instance",
    tag = "Admin",
    responses(
        (status = 200, body = InstanceStatus),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_instance(State(state): State<AppState>) -> ApiResult<InstanceStatus> {
    let instance = state.instance.read().await;
    let status = match instance.as_ref() {
        Some(instance) => InstanceStatus {
            started: true,
            client_count: instance.client_count,
            server_port: Some(instance.server_port),
            rcon_port: Some(instance.rcon_port),
        },
        None => InstanceStatus {
            started: false,
            client_count: 0,
            server_port: None,
            rcon_port: None,
        },
    };
    Ok(Json(status))
}

/// Stops the running Factorio instance
#[utoipa::path(
    post,
    path = "/api/v1/instance/stop",
    tag = "Admin",
    responses(
        (status = 204),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn stop_instance(State(state): State<AppState>) -> Result<StatusCode, ErrorResponse> {
    let taken = state.instance.write().await.take();
    match taken {
        // stop() consumes self and is synchronous; propagate its error rather
        // than unwrapping, which would abort the process under panic = "abort"
        Some(instance) => {
            instance.stop().map_err(ErrorResponse::from)?;
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err(ErrorResponse::not_started()),
    }
}
```

Check `FactorioInstance`'s real field names and types (`crates/core/src/process/process_control.rs:20-33`) before writing `get_instance` — `client_count`, `server_port` and `rcon_port` must match exactly, including whether the ports are `u16` or something else.

Register both in `crates/server/src/manage/mod.rs` with `.routes(routes!(instance::get_instance))` and `.routes(routes!(instance::stop_instance))`, and add `pub mod instance;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-server'
git commit -- crates/server/src/manage crates/server/tests/manage_instance.rs -m "feat(server): expose instance status and stop over HTTP"
```

---

### Task 4: RCON endpoint

**Files:**
- Create: `crates/server/src/manage/rcon.rs`
- Modify: `crates/server/src/manage/mod.rs`
- Test: `crates/server/tests/manage_rcon.rs`

**Interfaces:**
- Produces: `POST /api/v1/rcon` accepting `{ "command": string }`, returning `204`.

The Tauri command (`app/src-tauri/src/gui/command/rcon.rs:8-18`) discards the RCON reply. This port keeps that behavior; returning the reply is a separate improvement noted in the spec, not this task's job.

- [ ] **Step 1: Write the failing tests**

Create `crates/server/tests/manage_rcon.rs`:

```rust
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        instance: FactorioInstance::new_shared(),
        settings: AppSettings::default().into_shared(),
    }
}

async fn post_rcon(body: &str) -> StatusCode {
    build_router(test_state(), None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/rcon")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn rcon_without_a_running_instance_reports_not_started() {
    assert_eq!(
        post_rcon(r#"{"command":"/help"}"#).await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn rcon_rejects_a_body_without_a_command() {
    assert_eq!(post_rcon("{}").await, StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test manage_rcon'`
Expected: FAIL — 404.

- [ ] **Step 3: Write the handler**

Create `crates/server/src/manage/rcon.rs`:

```rust
use crate::error::ErrorResponse;
use crate::extract::ApiJson;
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RconBody {
    pub command: String,
}

/// Sends a raw RCON command to the running Factorio server
#[utoipa::path(
    post,
    path = "/api/v1/rcon",
    tag = "Admin",
    request_body = RconBody,
    responses(
        (status = 204),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn send_rcon(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<RconBody>,
) -> Result<StatusCode, ErrorResponse> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    instance
        .rcon
        .send(&body.command)
        .await
        .map_err(ErrorResponse::from)?;
    Ok(StatusCode::NO_CONTENT)
}
```

Add `pub mod rcon;` and `.routes(routes!(rcon::send_rcon))` to `crates/server/src/manage/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-server'
git commit -- crates/server/src/manage crates/server/tests/manage_rcon.rs -m "feat(server): expose rcon over HTTP"
```

---

### Task 5: Script browsing and editing

**Files:**
- Create: `crates/server/src/manage/scripts.rs`
- Modify: `crates/server/src/manage/mod.rs`
- Test: `crates/server/tests/manage_scripts.rs`

**Interfaces:**
- Consumes: `factorio_bot_core::scripts::{scripts_dir, resolve_script_path}` (Task 1).
- Produces: `GET /api/v1/scripts?path=/` returning `Vec<ScriptTreeNode>`; `GET /api/v1/scripts/file?path=…` returning `{ code: string }`; `PUT /api/v1/scripts/file?path=…` accepting `{ code: string }`.

`ScriptTreeNode` is `{ key: String, label: String, leaf: bool, children: Vec<ScriptTreeNode> }` — the same shape as the existing `PrimeVueTreeNode` in `crates/core/src/types.rs`, which the frontend already consumes. **Reuse `PrimeVueTreeNode` rather than defining a second type**; renaming it is a separate change that would touch the generated `types.ts` and the Vue code.

The directory listing in `app/src-tauri/src/gui/command/script.rs:113-129` calls `.to_str().unwrap()` on each filename. A file whose name is not valid UTF-8 aborts the process. Use `to_string_lossy()`.

- [ ] **Step 1: Write the failing tests**

Create `crates/server/tests/manage_scripts.rs`:

```rust
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use tower::ServiceExt;

/// Points workspace_path at a temp dir holding a scripts/ directory, so these
/// tests never touch the developer's real scripts.
fn state_with_scripts(dir: &std::path::Path) -> AppState {
    std::fs::create_dir_all(dir.join("scripts").join("sub")).expect("mkdir");
    std::fs::write(dir.join("scripts").join("hello.lua"), "-- hello").expect("write");
    let mut settings = AppSettings::default();
    settings.factorio.workspace_path = dir.to_string_lossy().into_owned();
    AppState {
        instance: FactorioInstance::new_shared(),
        settings: settings.into_shared(),
    }
}

async fn get(state: AppState, uri: &str) -> (StatusCode, String) {
    let response = build_router(state, None)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn lists_the_scripts_directory() {
    let dir = tempfile::tempdir().unwrap();
    let (status, body) = get(state_with_scripts(dir.path()), "/api/v1/scripts?path=/").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("hello.lua"), "{body}");
    assert!(body.contains("\"leaf\":true"), "{body}");
}

#[tokio::test]
async fn reads_a_script() {
    let dir = tempfile::tempdir().unwrap();
    let (status, body) = get(
        state_with_scripts(dir.path()),
        "/api/v1/scripts/file?path=/hello.lua",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("-- hello"), "{body}");
}

/// The old handler byte-sliced the path with &path[1..], which panics here.
#[tokio::test]
async fn an_empty_path_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let (status, _body) = get(state_with_scripts(dir.path()), "/api/v1/scripts/file?path=").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn traversal_outside_the_scripts_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("secret.txt"), "top secret").expect("write");
    let (status, body) = get(
        state_with_scripts(dir.path()),
        "/api/v1/scripts/file?path=/../secret.txt",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!body.contains("top secret"), "leaked file contents: {body}");
}

#[tokio::test]
async fn writes_a_script() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/scripts/file?path=/hello.lua")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"code":"-- replaced"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success(), "got {}", response.status());
    let written = std::fs::read_to_string(dir.path().join("scripts").join("hello.lua")).unwrap();
    assert_eq!(written, "-- replaced");
}
```

**These tests depend on `scripts_dir` preferring the workspace.** It checks `./scripts` and `../../scripts` relative to the current working directory first, and the test process runs inside the repository, where `./scripts` exists — so it would return the repo's scripts and the tests would fail confusingly. Before writing the handlers, decide how to handle that and say which you chose in your report: either give `scripts_dir` an explicit override argument that the handler passes from settings, or have the handler resolve the workspace path itself and call `resolve_script_path` against `workspace/scripts` directly. **Do not** make the tests chdir — that is process-global and would break other tests running in parallel.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test manage_scripts'`
Expected: FAIL — 404 on all routes.

- [ ] **Step 3: Write the handlers**

Create `crates/server/src/manage/scripts.rs` with three handlers. The listing handler builds `PrimeVueTreeNode`s from `read_dir`, using `to_string_lossy()` for both `key` and `label`, sorting entries by name so the response is deterministic, and marking directories with `leaf: false`. The read handler returns `{ code }`. The write handler takes `{ code }` and writes it.

All three resolve their path through `resolve_script_path`, mapping its error to `ErrorResponse::bad_request`. None of them may use `.unwrap()` on anything derived from the request or the filesystem.

Register all three in `crates/server/src/manage/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS — five new tests plus everything existing.

- [ ] **Step 5: Prove the traversal guard**

Temporarily replace `resolve_script_path`'s canonicalise-and-check with the old `if requested.contains("..")` substring test, run `traversal_outside_the_scripts_directory_is_refused`, and confirm it still passes — the old check does block this particular attempt. Then try `path=/sub/../../secret.txt`, which the substring check also blocks. The point of the exercise is to confirm your test actually exercises the resolver rather than an incidental `exists()` failure: verify the request reaches `resolve_script_path` at all, by temporarily making it return `Ok(root.join("secret.txt"))` and watching the test fail with leaked contents. Restore afterwards and record what you saw.

- [ ] **Step 6: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-server'
git commit -- crates/server/src/manage crates/server/tests/manage_scripts.rs -m "feat(server): expose script browsing and editing over HTTP"
```

---

### Task 6: `--web-root`, a filesystem check, and a shutdown timeout

**Files:**
- Create: `crates/server/src/manage/fs.rs`
- Modify: `crates/server/src/manage/mod.rs`, `app/src-tauri/src/cli/serve.rs`, `crates/server/src/webserver.rs`
- Test: `crates/server/tests/manage_fs.rs`, `crates/server/tests/shutdown.rs`

**Interfaces:**
- Produces: `GET /api/v1/fs/exists?path=…` returning `{ exists: bool }`; `factorio-bot serve --web-root DIR`.

Three loose ends, gathered because each is small.

**`--web-root`** is in the spec's Process section — `factorio-bot serve --bind ADDR [--web-root DIR]` — but neither plan 2 nor its review scoped it, so it fell between plans. The flag overrides `settings.restapi.web_root`.

**`fs/exists`** ports `file_exists` (`app/src-tauri/src/gui/command/io.rs:7-11`). The settings page uses it to validate the workspace and archive paths as you type. Note this now checks paths on the *server*, which is the correct semantics for a remote UI but a change in meaning.

**The shutdown timeout** guards a hang that does not exist yet but will: once plan 4 adds SSE streams that never end on their own, `with_graceful_shutdown` will wait for them forever, and a second Ctrl-C cannot help because `tokio::signal` has replaced the default disposition for the process lifetime. Wrap the serve future so shutdown gives in-flight work a bounded grace period — 10 seconds — and then proceeds to stop the instance regardless.

- [ ] **Step 1: Write the failing tests**

Create `crates/server/tests/manage_fs.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        instance: FactorioInstance::new_shared(),
        settings: AppSettings::default().into_shared(),
    }
}

async fn exists(uri: &str) -> String {
    let response = build_router(test_state(), None)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn reports_an_existing_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().into_owned();
    let body = exists(&format!("/api/v1/fs/exists?path={path}")).await;
    assert!(body.contains("true"), "{body}");
}

#[tokio::test]
async fn reports_a_missing_path() {
    let body = exists("/api/v1/fs/exists?path=/definitely/not/here/at/all").await;
    assert!(body.contains("false"), "{body}");
}
```

Append to `crates/server/tests/shutdown.rs`:

```rust
/// A request that never completes must not hold shutdown open forever — plan 4
/// adds SSE streams that never end on their own.
#[tokio::test]
async fn shutdown_does_not_wait_forever_for_an_in_flight_request() {
    // This test asserts the timeout path exists and bounds shutdown. Build a
    // state, start the server on a free port, open a connection, then signal
    // shutdown and assert the future resolves well inside the grace period plus
    // a margin.
}
```

Replace that placeholder body with a real test. If driving a genuinely stuck request proves impractical, assert instead that `start_with_shutdown` returns within the grace period plus a margin when a client holds an idle keep-alive connection open, and say in your report which you did and why.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server`
Expected: FAIL — 404 on the fs routes.

- [ ] **Step 3: Write the fs handler**

Create `crates/server/src/manage/fs.rs`:

```rust
use crate::error::ApiResult;
use crate::extract::ApiQuery;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
pub struct ExistsParams {
    pub path: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExistsResponse {
    pub exists: bool,
}

/// Reports whether a path exists on the server
#[utoipa::path(
    get,
    path = "/api/v1/fs/exists",
    tag = "Admin",
    params(ExistsParams),
    responses(
        (status = 200, body = ExistsResponse),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn exists(ApiQuery(params): ApiQuery<ExistsParams>) -> ApiResult<ExistsResponse> {
    Ok(Json(ExistsResponse {
        exists: std::path::Path::new(&params.path).exists(),
    }))
}
```

Register it in `crates/server/src/manage/mod.rs`.

- [ ] **Step 4: Add `--web-root`**

In `app/src-tauri/src/cli/serve.rs`, add an `Arg` named `web-root` alongside `bind`, and when present override the settings value before starting — the server reads the web root from settings at startup, so the override must be applied to the `SharedAppSettings` (or passed through the call) rather than set after the router is built. Match whatever shape plan 2's fix wave left `start_with_shutdown` in.

- [ ] **Step 5: Add the shutdown timeout**

In `crates/server/src/webserver.rs`, bound the graceful-shutdown wait. `axum::serve(...).with_graceful_shutdown(shutdown)` returns a future; wrap the await in `tokio::time::timeout(Duration::from_secs(10), …)`. On timeout, log that in-flight requests were abandoned and continue to the instance-stop block — never skip it, which is the bug plan 2 already fixed once on the error path.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server`
Expected: PASS.

- [ ] **Step 7: End-to-end check**

Build and run:
```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --no-default-features --features cli,repl,restapi,lua'
./target/debug/factorio-bot serve --bind 127.0.0.1:7492 --web-root ./app/dist
```
Then in another shell:
```bash
curl -s localhost:7492/api/v1/settings | head -c 120; echo
curl -s localhost:7492/api/v1/instance; echo
curl -s "localhost:7492/api/v1/scripts?path=/"; echo
curl -s -o /dev/null -w '%{http_code}\n' localhost:7492/
```
Expected: settings JSON; `{"started":false,...}`; a JSON array of script nodes; and `200` for `/` if `app/dist` exists. Then Ctrl-C and confirm prompt exit.

- [ ] **Step 8: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-server -p factorio-bot'
git commit -- crates/server/src/manage crates/server/src/webserver.rs crates/server/tests/manage_fs.rs crates/server/tests/shutdown.rs app/src-tauri/src/cli/serve.rs -m "feat(server): add fs check, --web-root and a bounded shutdown"
```

---

### Task 7: Creating and deleting scripts

**Files:**
- Modify: `crates/server/src/manage/scripts.rs`, `crates/server/src/manage/mod.rs`
- Test: `crates/server/tests/manage_scripts.rs`

**Interfaces:**
- Consumes: `factorio_bot_core::scripts::resolve_script_path`, `scripts_root`/`scripts_root_path` (Task 5).
- Produces: `POST /api/v1/scripts/file?path=…` accepting `{ "code": string }`, creating a file that must **not** already exist; `DELETE /api/v1/scripts/file?path=…`, removing a file that must exist.

Today the web UI cannot add or remove a script — `PUT` only overwrites, faithfully preserving the Tauri behavior. On a desktop that was tolerable because the files were on the same machine; for a remote browser it means you need shell access on the server to add a script, which defeats the point. The owner asked for both verbs.

**The design point that makes this non-trivial.** `resolve_script_path` canonicalises, and `canonicalize` fails when the target does not exist. So it cannot resolve the path of a file you are about to create. Creating therefore resolves the **parent directory** instead, and validates the final component separately:

1. Split the requested path into a parent part and a final component.
2. Resolve the parent through `resolve_script_path` — this is what keeps the traversal guard in force — and require it to be a directory.
3. Reject a final component that is empty, `.`, `..`, or contains a path separator (`/` or `\\`). Without this, `path=/sub/../../evil.lua` would resolve a legitimate parent and then escape via the component.
4. Join, require the result does **not** exist, and write.

Deleting is simpler: resolve through `resolve_script_path` as the read and write handlers do, require `is_file()`, and remove it. Deleting a directory is not supported.

- [ ] **Step 1: Write the failing tests**

Append to `crates/server/tests/manage_scripts.rs`, following the helpers already there:

```rust
#[tokio::test]
async fn creates_a_new_script() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/scripts/file?path=/fresh.lua")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"code":"-- fresh"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success(), "got {}", response.status());
    let written =
        std::fs::read_to_string(dir.path().join("scripts").join("fresh.lua")).unwrap();
    assert_eq!(written, "-- fresh");
}

#[tokio::test]
async fn refuses_to_create_over_an_existing_script() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/scripts/file?path=/hello.lua")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"code":"-- clobber"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let kept = std::fs::read_to_string(dir.path().join("scripts").join("hello.lua")).unwrap();
    assert_eq!(kept, "-- hello", "existing script must not be overwritten");
}

/// The parent resolves legitimately; the escape is in the final component.
#[tokio::test]
async fn creating_cannot_escape_via_the_final_component() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    for attempt in [
        "/sub/../../escaped.lua",
        "/../escaped.lua",
        "/sub/..%2F..%2Fescaped.lua",
    ] {
        let response = build_router(state.clone(), None)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&format!("/api/v1/scripts/file?path={attempt}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"code":"-- escaped"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{attempt}");
    }
    assert!(
        !dir.path().join("escaped.lua").exists(),
        "a file escaped the scripts root"
    );
}

#[tokio::test]
async fn deletes_a_script() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/scripts/file?path=/hello.lua")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success(), "got {}", response.status());
    assert!(!dir.path().join("scripts").join("hello.lua").exists());
}

#[tokio::test]
async fn deleting_a_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_scripts(dir.path());

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/scripts/file?path=/sub")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(dir.path().join("scripts").join("sub").exists());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test manage_scripts'`
Expected: FAIL — 405 or 404 on both new methods.

- [ ] **Step 3: Implement the two handlers**

In `crates/server/src/manage/scripts.rs`, add `create_script` and `delete_script` following the shape of the existing `read_script`/`write_script`: `ApiQuery` for the path, `ApiJson` for the create body, `#[utoipa::path]` with tag `"Admin"`, and `ErrorResponse::bad_request` for every rejection.

Factor the parent-plus-component resolution into a small helper rather than inlining it, since it is the security-relevant part and wants to be readable on its own.

Register both in `crates/server/src/manage/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS — five new tests plus everything existing.

- [ ] **Step 5: Prove the component guard**

Delete the final-component validation (step 3 of the design point) and run the suite. `creating_cannot_escape_via_the_final_component` must fail, and the escaped file must appear outside the scripts root. Report the full failure set, restore, and confirm green. A guard is only proven by a red test when it is removed.

- [ ] **Step 6: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-server'
git commit -- crates/server/src/manage/scripts.rs crates/server/src/manage/mod.rs crates/server/tests/manage_scripts.rs -m "feat(server): create and delete scripts over HTTP"
```

---

## Self-Review

**Spec coverage.** This plan implements the spec's management endpoints for every operation that completes quickly: settings (GET/PUT), instance status and stop, RCON, script listing/read/write/create/delete, and `fs/exists`. Create and delete go beyond the ported Tauri surface, at the owner's request — a browser-only UI otherwise needs shell access on the server to add a script. It also closes `--web-root`, which the spec's Process section requires and which fell between plans 1 and 2, and adds the shutdown timeout the plan-2 review asked be scoped here. Deliberately deferred to plan 4: `start_instances`, `execute_script` and `execute_code`, the job registry, SSE, script-execution serialization, `spawn_blocking` for `run_lua` and `extract_archive`, and removing the `gag` stdout redirect — all of which hang together and are useless separately. Deferred to plan 5: the frontend and deleting Tauri. Not ported at all, per the spec: `maximize_window`, `open_in_browser`, `is_port_available`, `start_restapi`/`stop_restapi`/`is_restapi_started`, and `save_settings` (no caller).

**Placeholder scan.** No TBD entries. Three steps deliberately ask the implementer to decide and report rather than prescribing: how `scripts_dir` avoids picking up the repository's own `scripts/` during tests (Task 5 Step 1), whether the shutdown-timeout test drives a stuck request or an idle keep-alive (Task 6 Step 1), and the exact `--web-root` override mechanism (Task 6 Step 4). Each is a real judgment call that depends on code shape the plan cannot pin down in advance. Task 5 Step 3 describes the three handlers in prose rather than full code because they are near-identical to the originals being ported; the file:line of each original is given, and every non-mechanical part — the lossy filename conversion, the sort, the path resolution — is called out explicitly.

**Type consistency.** `AppState { instance, settings }`, `ApiResult<T>`, `ErrorResponse::{not_started, bad_request}`, `ApiQuery`/`ApiJson`, and `build_router(state, web_root)` are used identically throughout. `PrimeVueTreeNode` is reused rather than redefined. `InstanceStatus`, `RconBody`, `ExistsParams` and `ExistsResponse` are each defined once, in the task that introduces them.

**Known risk.** Task 2 adds `utoipa::ToSchema` to `AppSettings` and possibly to `FactorioSettings`, `GuiSettings` and `RestApiSettings`. Those types already carry `TypeScriptify`, `JsonSchema` and serde derives; a fourth derive on a type that `build.rs` also generates TypeScript from is the kind of thing that produces a surprising `types.ts` diff. Task 2 should check `git diff app/src/models/types.ts` after building and commit it if it changed.
