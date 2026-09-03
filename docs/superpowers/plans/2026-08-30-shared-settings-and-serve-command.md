# Shared Settings and `serve` Command — Implementation Plan (2 of 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `factorio-bot serve --bind ADDR` start the axum server as a first-class, long-lived process that owns settings from a shared crate and stops Factorio cleanly on shutdown.

**Architecture:** `AppSettings`, `GuiSettings`, `RestApiSettings` and the `paths` helpers move out of the binary crate into `crates/core`, which breaks the dependency knot that currently prevents `crates/server` from reading application settings. The server's `AppState` then holds the real `SharedAppSettings` instead of its own private settings copy, a new `serve` subcommand starts it, and `axum::serve` gets a graceful-shutdown future that takes and stops the running `FactorioInstance`.

**Tech Stack:** Rust 2024, axum 0.8, tokio, clap 4, miette.

**Spec:** `docs/superpowers/specs/2026-08-29-webserver-replaces-tauri-gui-design.md`

**Plans:** 1 of 4 (axum server replacing Rocket) is complete. This is 2 of 4. Plan 3 is the management API, job registry and SSE. Plan 4 is the frontend transport swap and deleting Tauri.

## Global Constraints

- Workspace root `/home/arturh/projects/private/factorio-bot`. Rust edition **2024** (corrected 2026-09-03; this line said 2021, and every crate's `Cargo.toml` says `edition = "2024"`). Any `rustfmt` invocation therefore **needs `--edition 2024`** — the flag is not optional: bare `rustfmt` defaults to Rust 2015, dies on every `async fn` in the file, and chained with `&&` silently skips whatever came next. Branch: `master`, committing directly — no feature branches.
- **Every cargo command runs inside the Nix devShell with mise on PATH:**
  `nix develop --command bash -c 'eval "$(mise env -s bash)"; <command>'`
- **`cargo fmt -p <crate>` only, never `cargo fmt --all`** — the workspace has pre-existing drift in crates another agent owns, and `--all` sweeps it in.
  **CORRECTED 2026-09-03: `cargo fmt -p <crate>` is banned as well.** CLAUDE.md bans every rewriting `cargo fmt` form, `-p` included — it rewrites a *whole crate*, so it clobbers another agent's uncommitted files in that crate exactly as `--all` already did once. Format only the files you edited: `rustfmt --edition 2024 <file>`. **`--edition 2024` is not optional** — bare `rustfmt` defaults to Rust 2015, dies on every `async fn` in the file, and chained with `&&` silently skips whatever came next. Every `cargo fmt -p …` in the steps below is subject to this.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated` must pass.
- **`git commit -- <explicit paths>`, never `git add -A` or a bare `git commit`.** Another agent works in this same checkout and the git index is shared; committing the whole index sweeps up their staged work.
- `cargo test --workspace` rewrites `crates/scripting_lua/tests/task_graph-1.{dot,md}` and three `.png` snapshots. They are stale in the repo and regenerate on every run — revert with `git checkout -- crates/scripting_lua/tests/` and never commit them.
- **The data directory name must not change.** `paths::data_local_dir()` currently builds it from `env!("CARGO_PKG_NAME")`, which evaluates to `factorio-bot` in the binary crate. Moving the code to `crates/core` would silently change it to `factorio-bot-core` and orphan every existing user's settings file and workspace.
- No `.unwrap()` on user-supplied input in crate source — the workspace sets `panic = "abort"` for release.
- `crates/server` must not depend on the binary crate `factorio-bot`; the binary already depends on the server.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/src/paths.rs` | New. Data-local dir, settings file, workspace dir. Moved verbatim from `app/src-tauri/src/paths.rs` with the crate-name hazard fixed. |
| `crates/core/src/app_settings.rs` | New. `AppSettings`, `GuiSettings`, `SharedAppSettings`, load/save. Moved from `app/src-tauri/src/settings.rs`. |
| `crates/core/src/settings.rs` | Modified. Gains `RestApiSettings`, moved from `crates/server/src/settings.rs`, so both the server and the settings model can see it without a cycle. |
| `crates/server/src/settings.rs` | Deleted; re-exported from core at its old path so existing `use` sites keep working. |
| `crates/server/src/state.rs` | Modified. `AppState.settings` becomes `SharedAppSettings`. |
| `crates/server/src/webserver.rs` | Modified. `start()` takes `SharedAppSettings`, binds a configurable address, and gets graceful shutdown. |
| `app/src-tauri/src/cli/serve.rs` | New. The `serve` subcommand. |
| `app/src-tauri/src/settings.rs` | Reduced to re-exports from core, so the ~30 existing `crate::settings::` references keep compiling. |
| `app/src-tauri/src/paths.rs` | Reduced to re-exports from core. |

---

### Task 1: Move paths into core without renaming the data directory

**Files:**
- Create: `crates/core/src/paths.rs`
- Modify: `crates/core/src/lib.rs`, `app/src-tauri/src/paths.rs`
- Test: `crates/core/src/paths.rs` (inline `#[cfg(test)]` module)

**Interfaces:**
- Produces: `factorio_bot_core::paths::{APP_SETTINGS_FILENAME, data_local_dir, settings_file, workspace_dir}` — all with the same signatures they have today in the binary crate.

The whole point of this task is the constant. `app/src-tauri/src/paths.rs:10` uses `env!("CARGO_PKG_NAME")`, which expands at compile time to the *containing crate's* name. Moving the file as-is changes the directory from `factorio-bot` to `factorio-bot-core`, and every existing install silently loses its settings and workspace. The moved code hardcodes the name instead, and a test pins it.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/paths.rs` containing only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The data directory is named after the binary, not the crate this code
    /// lives in. Moving this module must not rename it: doing so orphans every
    /// existing user's AppSettings.toml and workspace.
    #[test]
    fn data_local_dir_is_named_after_the_binary() {
        let dir = data_local_dir();
        let name = dir
            .file_name()
            .expect("data dir has a final component")
            .to_str()
            .expect("data dir name is utf-8");
        let expected = if cfg!(debug_assertions) {
            "factorio-bot-dev"
        } else {
            "factorio-bot"
        };
        assert_eq!(name, expected);
    }

    #[test]
    fn settings_file_lives_in_the_data_dir() {
        assert_eq!(
            settings_file().parent().expect("has a parent"),
            data_local_dir()
        );
        assert_eq!(
            settings_file()
                .file_name()
                .expect("has a name")
                .to_str()
                .expect("utf-8"),
            APP_SETTINGS_FILENAME
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core paths'`
Expected: FAIL — the module is not declared and the functions do not exist.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/paths.rs`, above the test module:

```rust
use crate::constants::WORKSPACE_FOLDERNAME;
use std::path::PathBuf;

pub const APP_SETTINGS_FILENAME: &str = "AppSettings.toml";

/// Name of the shipped binary. Hardcoded rather than taken from
/// `env!("CARGO_PKG_NAME")`, which would resolve to this crate's name and
/// rename the user-visible data directory.
const APP_NAME: &str = "factorio-bot";

pub fn data_local_dir() -> PathBuf {
    dirs_next::data_local_dir()
        .expect("no local data directory available")
        .join(format!(
            "{}{}",
            APP_NAME,
            if cfg!(debug_assertions) { "-dev" } else { "" }
        ))
}

pub fn settings_file() -> PathBuf {
    data_local_dir().join(APP_SETTINGS_FILENAME)
}

pub fn workspace_dir() -> PathBuf {
    data_local_dir().join(WORKSPACE_FOLDERNAME)
}
```

Add `pub mod paths;` to `crates/core/src/lib.rs`, alongside the other `pub mod` declarations.

`dirs_next` must be a dependency of `crates/core`. Check `crates/core/Cargo.toml`; if it is absent, add `dirs-next = "2.0"` (the binary crate already uses that exact version).

- [ ] **Step 4: Run the test to verify it passes**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core paths'`
Expected: PASS — two tests.

- [ ] **Step 5: Point the binary crate at core**

Replace the entire contents of `app/src-tauri/src/paths.rs` with:

```rust
//! Re-exported from `factorio_bot_core::paths` so the ~10 existing
//! `crate::paths::` call sites keep compiling.
pub use factorio_bot_core::paths::{
    data_local_dir, settings_file, workspace_dir, APP_SETTINGS_FILENAME,
};
```

- [ ] **Step 6: Verify the workspace still builds**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --all-features'`
Expected: success.

If a call site complains that `APP_SETTINGS_FILENAME` is unused, delete it from the re-export list rather than adding an `#[allow]`.

- [ ] **Step 7: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-core'
git commit -- crates/core/src/paths.rs crates/core/src/lib.rs crates/core/Cargo.toml app/src-tauri/src/paths.rs -m "refactor(core): move paths into core, pinning the data directory name"
```

---

### Task 2: Move RestApiSettings into core

**Files:**
- Modify: `crates/core/src/settings.rs`, `crates/server/src/settings.rs`, `app/src-tauri/build.rs`
- Test: `crates/core/src/settings.rs` (inline `#[cfg(test)]` module)

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `factorio_bot_core::settings::RestApiSettings { pub port: i64, pub web_root: Option<String> }`, with `Default` giving `port: 7492, web_root: None`. `factorio_bot_server::settings::RestApiSettings` remains valid as a re-export.

This is what unties the knot. `AppSettings` (moving to core in Task 3) has a `RestApiSettings` field, and `RestApiSettings` currently lives in `crates/server`. Core cannot depend on the server — the server already depends on core — so the type moves down into core and the server re-exports it.

- [ ] **Step 1: Write the failing test**

Append to `crates/core/src/settings.rs`:

```rust
#[cfg(test)]
mod restapi_settings_tests {
    use super::RestApiSettings;

    #[test]
    fn defaults_match_the_shipped_configuration() {
        let settings = RestApiSettings::default();
        assert_eq!(settings.port, 7492);
        assert_eq!(settings.web_root, None);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core restapi_settings'`
Expected: FAIL — `RestApiSettings` is not defined in this crate.

- [ ] **Step 3: Move the type**

Append to `crates/core/src/settings.rs`, above the test module (this is the current body of `crates/server/src/settings.rs`, unchanged):

```rust
#[derive(
    Debug, Clone, typescript_definitions::TypeScriptify, serde::Serialize, serde::Deserialize,
)]
pub struct RestApiSettings {
    pub port: i64,
    /// Directory containing the built SPA. Relative paths resolve against the
    /// current working directory. When the directory does not exist, the server
    /// still starts and serves the API only.
    #[serde(default)]
    pub web_root: Option<String>,
}

impl Default for RestApiSettings {
    fn default() -> Self {
        RestApiSettings {
            port: 7492,
            web_root: None,
        }
    }
}
```

Replace the entire contents of `crates/server/src/settings.rs` with:

```rust
//! Re-exported from core, which owns this type so that `AppSettings` can embed
//! it without core depending on this crate.
pub use factorio_bot_core::settings::RestApiSettings;
```

- [ ] **Step 4: Update the TypeScript generator**

`app/src-tauri/build.rs:44-45` imports `RestApiSettings` for `typescriptify()`. Change that import to `factorio_bot_core::settings::RestApiSettings`. Leave the `#[cfg(feature = "restapi")]` gate as it is.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core restapi_settings && cargo build --all-features'`
Expected: PASS, then a successful build.

`app/src/models/types.ts` may be regenerated identically — the type's shape has not changed, only its location. If it does change, commit it.

- [ ] **Step 6: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-core -p factorio-bot-server'
git commit -- crates/core/src/settings.rs crates/server/src/settings.rs app/src-tauri/build.rs app/src/models/types.ts -m "refactor(core): move RestApiSettings into core"
```

---

### Task 3: Move AppSettings into core

**Files:**
- Create: `crates/core/src/app_settings.rs`
- Modify: `crates/core/src/lib.rs`, `app/src-tauri/src/settings.rs`
- Test: `crates/core/src/app_settings.rs` (inline `#[cfg(test)]` module)

**Interfaces:**
- Consumes: `factorio_bot_core::paths` (Task 1), `factorio_bot_core::settings::RestApiSettings` (Task 2).
- Produces: `factorio_bot_core::app_settings::{AppSettings, GuiSettings, SharedAppSettings, load_app_settings}`. `AppSettings::into_shared(self) -> SharedAppSettings`, `AppSettings::load(PathBuf) -> Result<AppSettings>`, and whatever save/update functions `app/src-tauri/src/settings.rs` currently defines, with identical signatures.

Read `app/src-tauri/src/settings.rs` in full before starting — this task moves all of it. Two changes are required during the move:

1. `use crate::paths;` becomes `use crate::paths;` still — but now that is core's own `paths` module from Task 1, not the binary's.
2. The `#[cfg(feature = "restapi")]` gate on the `restapi` field must be **removed**. Core has no `restapi` feature, and gating a settings field on a feature that does not exist there would silently drop it from the serialized form. The field becomes unconditional, which also fixes a latent inconsistency: the settings file's shape currently depends on which features the binary was built with.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/app_settings.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_serialisable_and_round_trip() {
        let settings = AppSettings::default();
        let json = serde_json::to_value(&settings).expect("serialises");
        let restored: AppSettings = serde_json::from_value(json).expect("deserialises");
        assert_eq!(restored.restapi.port, settings.restapi.port);
        assert_eq!(
            restored.gui.enable_autostart,
            settings.gui.enable_autostart
        );
    }

    /// The restapi section is no longer feature-gated, so it is present in the
    /// serialised form regardless of how the binary was built.
    #[test]
    fn restapi_section_is_always_present() {
        let json = serde_json::to_value(AppSettings::default()).expect("serialises");
        assert!(
            json.get("restapi").is_some(),
            "expected a restapi section, got: {json}"
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core app_settings'`
Expected: FAIL — the module is not declared.

- [ ] **Step 3: Move the implementation**

Copy the non-test contents of `app/src-tauri/src/settings.rs` into `crates/core/src/app_settings.rs`, above the test module, applying the two changes named above. The struct definitions become:

```rust
#[allow(clippy::module_name_repetitions)]
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct GuiSettings {
    pub enable_autostart: bool,
    pub enable_restapi: bool,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub factorio: FactorioSettings,
    pub restapi: RestApiSettings,
    pub gui: GuiSettings,
}

#[allow(clippy::module_name_repetitions)]
pub type SharedAppSettings = Arc<RwLock<AppSettings>>;
```

Note `RestApiSettings` has no `Default` derive of its own kind that `AppSettings::default()` can use — it has a hand-written `impl Default` from Task 2, which satisfies the derive on `AppSettings`.

Imports change from the binary crate's paths to core-internal ones: `use crate::settings::{FactorioSettings, RestApiSettings};`, `use crate::paths;`, and `use crate::miette::{IntoDiagnostic, Result};` becomes `use miette::{IntoDiagnostic, Result};` if core imports miette directly — check how neighbouring core modules do it and match them.

Add `pub mod app_settings;` to `crates/core/src/lib.rs`.

- [ ] **Step 4: Point the binary crate at core**

Replace the entire contents of `app/src-tauri/src/settings.rs` with:

```rust
//! Re-exported from `factorio_bot_core::app_settings` so the existing
//! `crate::settings::` call sites keep compiling.
pub use factorio_bot_core::app_settings::{
    load_app_settings, AppSettings, GuiSettings, SharedAppSettings,
};
```

Add any other item the binary references from this module — `grep -rn "crate::settings::" app/src-tauri/src` lists them all. Every name in that grep must appear in the re-export.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-core app_settings && cargo build --all-features'`
Expected: PASS, then a successful build.

- [ ] **Step 6: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-core'
git commit -- crates/core/src/app_settings.rs crates/core/src/lib.rs app/src-tauri/src/settings.rs -m "refactor(core): move AppSettings into core and ungate the restapi section"
```

---

### Task 4: Give the server the real settings and a configurable bind address

**Files:**
- Modify: `crates/server/src/state.rs`, `crates/server/src/webserver.rs`, `app/src-tauri/src/gui/command/restapi.rs:30-33`, `app/src-tauri/src/repl/restapi_control.rs:24-27`
- Test: `crates/server/tests/bind.rs`

**Interfaces:**
- Consumes: `factorio_bot_core::app_settings::SharedAppSettings` (Task 3).
- Produces: `AppState { pub instance: SharedFactorioInstance, pub settings: SharedAppSettings }`; `webserver::start(settings: SharedAppSettings, instance_state: SharedFactorioInstance, bind: SocketAddr) -> miette::Result<()>`; `webserver::build_router(state: AppState) -> axum::Router` (unchanged signature).

Plan 1 froze `start()`'s signature so the cutover needed no call-site edits. That job is done, and this task deliberately breaks it: the server needs the whole `AppSettings`, not a private copy of the restapi section, and the bind address must be a parameter rather than a hardcoded `127.0.0.1`.

- [ ] **Step 1: Write the failing test**

Create `crates/server/tests/bind.rs`:

```rust
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::webserver::start;
use std::net::SocketAddr;
use std::time::Duration;

/// start() must bind the address it is given, not a hardcoded one.
#[tokio::test]
async fn start_binds_the_requested_address() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    // port 0 asks the OS for a free port, so this test cannot collide with a
    // developer's running server
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");

    let server = tokio::spawn(async move { start(settings, instance, bind).await });

    // the server runs until aborted; if it returned early it failed to bind
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!server.is_finished(), "server exited instead of serving");
    server.abort();
}

#[tokio::test]
async fn start_reports_an_unbindable_address() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    // port 1 is privileged; binding it as an unprivileged user fails
    let bind: SocketAddr = "127.0.0.1:1".parse().expect("valid addr");

    let result = start(settings, instance, bind).await;
    assert!(result.is_err(), "expected a bind error");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test bind'`
Expected: FAIL to compile — `start` takes two arguments and a different settings type.

- [ ] **Step 3: Change the state**

In `crates/server/src/state.rs`:

```rust
use factorio_bot_core::app_settings::SharedAppSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;

#[derive(Clone)]
pub struct AppState {
    pub instance: SharedFactorioInstance,
    pub settings: SharedAppSettings,
}
```

- [ ] **Step 4: Change start() and the web-root lookup**

In `crates/server/src/webserver.rs`, `start` becomes:

```rust
pub async fn start(
    settings: SharedAppSettings,
    instance_state: SharedFactorioInstance,
    bind: SocketAddr,
) -> Result<()> {
    let state = AppState {
        instance: instance_state,
        settings,
    };
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(bind).await.into_diagnostic()?;
    tracing::info!("listening on http://{bind}");
    axum::serve(listener, app).await.into_diagnostic()?;
    Ok(())
}
```

`build_router` reads the web root; the field is now one level deeper. Change that read from `settings.web_root.clone()` to `settings.restapi.web_root.clone()`. The `u16::try_from(settings.port)` conversion added at the end of plan 1 moves out of `start()` entirely — the caller now supplies a `SocketAddr`, so there is no cast left to get wrong. Delete it.

- [ ] **Step 5: Update the two existing call sites**

`app/src-tauri/src/gui/command/restapi.rs:30-33` and `app/src-tauri/src/repl/restapi_control.rs:24-27` both call `webserver::start(app_settings.restapi.clone(), instance_state)`. Both become:

```rust
let bind = std::net::SocketAddr::from((
    [127, 0, 0, 1],
    u16::try_from(app_settings.read().await.restapi.port).unwrap_or(7492),
));
webserver::start(app_settings.clone(), instance_state, bind)
```

Match each site's surrounding `async` and borrow structure; read the file rather than pasting blindly. These two call sites are deleted entirely in plan 4 — the goal here is only to keep them compiling.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server && cargo build --all-features'`
Expected: PASS — the two bind tests plus the 24 from plan 1.

- [ ] **Step 7: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-server'
git commit -- crates/server/src/state.rs crates/server/src/webserver.rs crates/server/tests/bind.rs app/src-tauri/src/gui/command/restapi.rs app/src-tauri/src/repl/restapi_control.rs -m "feat(server): take shared app settings and an explicit bind address"
```

---

### Task 5: The `serve` subcommand with graceful shutdown

**Files:**
- Create: `app/src-tauri/src/cli/serve.rs`
- Modify: `app/src-tauri/src/cli/mod.rs:21-33`, `crates/server/src/webserver.rs`
- Test: `crates/server/tests/shutdown.rs`

**Interfaces:**
- Consumes: `webserver::start(SharedAppSettings, SharedFactorioInstance, SocketAddr)` (Task 4).
- Produces: `webserver::start_with_shutdown(settings, instance_state, bind, shutdown: impl Future<Output = ()> + Send + 'static) -> Result<()>`, with `start` delegating to it using a never-completing future. The `serve` subcommand: `factorio-bot serve [--bind ADDR]`, default `127.0.0.1:<settings.restapi.port>`.

`FactorioInstance` has no `Drop` impl and `stop()` is only ever called explicitly. On the desktop, closing the window let the OS reap Factorio's children; a server that is `SIGTERM`ed would orphan them.

- [ ] **Step 1: Write the failing test**

Create `crates/server/tests/shutdown.rs`:

```rust
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::webserver::start_with_shutdown;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::oneshot;

#[tokio::test]
async fn server_returns_when_the_shutdown_future_resolves() {
    let settings = AppSettings::default().into_shared();
    let instance = FactorioInstance::new_shared();
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");
    let (tx, rx) = oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        start_with_shutdown(settings, instance, bind, async {
            let _ = rx.await;
        })
        .await
    });

    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!server.is_finished(), "server exited before shutdown");

    tx.send(()).expect("receiver alive");

    let result = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server shut down within 5s")
        .expect("task did not panic");
    assert!(result.is_ok(), "shutdown should be a clean exit: {result:?}");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test shutdown'`
Expected: FAIL to compile — `start_with_shutdown` does not exist.

- [ ] **Step 3: Add graceful shutdown to the server**

In `crates/server/src/webserver.rs`:

```rust
pub async fn start_with_shutdown(
    settings: SharedAppSettings,
    instance_state: SharedFactorioInstance,
    bind: SocketAddr,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let state = AppState {
        instance: instance_state.clone(),
        settings,
    };
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(bind).await.into_diagnostic()?;
    tracing::info!("listening on http://{bind}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .into_diagnostic()?;

    // FactorioInstance has no Drop impl, so a server that is signalled would
    // otherwise leave the Factorio server and every client process orphaned.
    if let Some(instance) = instance_state.write().await.take() {
        tracing::info!("stopping factorio instance");
        instance.stop().await?;
    }
    Ok(())
}

pub async fn start(
    settings: SharedAppSettings,
    instance_state: SharedFactorioInstance,
    bind: SocketAddr,
) -> Result<()> {
    start_with_shutdown(settings, instance_state, bind, std::future::pending()).await
}
```

Check `FactorioInstance::stop`'s real signature in `crates/core/src/process/process_control.rs:470` before writing that call — it consumes `self` and is `async`, and its return type determines whether `?` applies.

- [ ] **Step 4: Run the test to verify it passes**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server`
Expected: PASS — shutdown, bind, and the 24 earlier tests.

- [ ] **Step 5: Write the subcommand**

Create `app/src-tauri/src/cli/serve.rs`, following the shape of `app/src-tauri/src/cli/rcon.rs`:

```rust
use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use clap::{value_parser, Arg, ArgMatches, Command};
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use std::net::SocketAddr;

impl Subcommand for ThisCommand {
    fn name(&self) -> &'static str {
        "serve"
    }
    fn build_command(&self) -> Command {
        Command::new("serve")
            .arg(
                Arg::new("bind")
                    .short('b')
                    .long("bind")
                    .value_name("ADDR")
                    .required(false)
                    .value_parser(value_parser!(String))
                    .help("address to bind, defaults to 127.0.0.1 on the configured port"),
            )
            .about("serve the web UI and HTTP API")
    }

    fn build_callback(&self) -> SubcommandCallback {
        |args, context| Box::pin(run(args, context))
    }
}

async fn run(matches: &ArgMatches, context: &mut Context) -> Result<()> {
    let bind: SocketAddr = match matches.get_one::<String>("bind") {
        Some(raw) => raw.parse().into_diagnostic()?,
        None => {
            let port = context.app_settings.read().await.restapi.port;
            SocketAddr::from(([127, 0, 0, 1], u16::try_from(port).into_diagnostic()?))
        }
    };

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutdown signal received");
    };

    factorio_bot_server::webserver::start_with_shutdown(
        context.app_settings.clone(),
        context.instance_state.clone(),
        bind,
        shutdown,
    )
    .await
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
    Box::new(ThisCommand {})
}
```

Register it in `app/src-tauri/src/cli/mod.rs`: add `#[cfg(feature = "restapi")] mod serve;` to the module list at the top, and `#[cfg(feature = "restapi")] serve::build(),` to the `subcommands()` vector.

- [ ] **Step 6: Verify the subcommand end to end**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --no-default-features --features cli,repl,restapi,lua'`

Then, in one shell:
`nix develop --command bash -c 'eval "$(mise env -s bash)"; ./target/debug/factorio-bot serve --bind 127.0.0.1:7492'`

And in another:
```bash
curl -s localhost:7492/api/v1/health
curl -s -o /dev/null -w '%{http_code}\n' localhost:7492/openapi.json
curl -s localhost:7492/api/v1/game/all-players
```
Expected: `ok`; `200`; and `{"message":"not started","code":2}`. Then press Ctrl-C in the first shell and confirm the process exits promptly rather than hanging.

- [ ] **Step 7: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-server'
git commit -- crates/server/src/webserver.rs crates/server/tests/shutdown.rs app/src-tauri/src/cli/serve.rs app/src-tauri/src/cli/mod.rs -m "feat(cli): add a serve subcommand with graceful shutdown"
```

---

### Task 6: JSON error shape for extractor rejections, and the parked plan-1 test

**Files:**
- Modify: `crates/server/src/error.rs`, `crates/server/src/game/query.rs`, `crates/server/src/game/control.rs`
- Test: `crates/server/tests/game_control.rs`, `crates/server/tests/game_query.rs`

**Interfaces:**
- Consumes: `ErrorResponse` and `ApiResult<T>` from plan 1.
- Produces: no new public API. Malformed query strings and bodies return the same JSON error shape as everything else.

Two loose ends from plan 1, both recorded in its final review.

The first: axum's own `Query`/`Json` rejections return `text/plain`, so `GET /api/v1/game/find-entities?radius=notanumber` answers in a different format from every other error the API produces.

The second: plan 1's fix for a newly-reachable process abort — validating `player_id` before the rcon call — went in without a regression test, on a report that claimed one was impossible. It is not: `FactorioRcon::new_empty()` (`crates/core/src/factorio/rcon.rs:61`) and `FactorioWorld::new()` (`crates/core/src/factorio/world.rs:226`) are plain public constructors needing no socket, and `panic = "abort"` applies only to the release profile, so a regression fails the test rather than killing the harness.

- [ ] **Step 1: Write the failing tests**

Append to `crates/server/tests/game_query.rs`:

```rust
#[tokio::test]
async fn malformed_query_parameters_return_json() {
    let response = build_router(test_state())
        .oneshot(
            Request::builder()
                .uri("/api/v1/game/find-entities?position=0,0&radius=notanumber")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("has a content type")
        .to_str()
        .expect("utf-8");
    assert!(
        content_type.starts_with("application/json"),
        "expected JSON, got {content_type}"
    );
}
```

Append to `crates/server/tests/game_control.rs`, adding `use factorio_bot_core::factorio::rcon::FactorioRcon;`, `use factorio_bot_core::factorio::world::FactorioWorld;` and `use factorio_bot_core::process::process_control::FactorioInstance;`:

```rust
/// A player id that is not in the world must be rejected before the rcon call.
/// Reaching rcon with an unknown id hits an unguarded unwrap in
/// player_path's error-recovery branch, which aborts the process in release.
#[tokio::test]
async fn mutating_routes_reject_an_unknown_player_before_calling_rcon() {
    let state = AppState {
        instance: Arc::new(RwLock::new(Some(FactorioInstance {
            world: Some(Arc::new(FactorioWorld::new())),
            rcon: Arc::new(FactorioRcon::new_empty()),
            ..FactorioInstance::default()
        }))),
        settings: AppSettings::default().into_shared(),
    };

    for (uri, body) in [
        ("/api/v1/game/move-player", r#"{"player_id":42,"goal":"0,0"}"#),
        (
            "/api/v1/game/place-entity",
            r#"{"player_id":42,"item":"stone-furnace","position":"0,0","direction":0}"#,
        ),
    ] {
        let response = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("player"), "{uri} returned: {text}");
    }
}
```

`FactorioInstance`'s fields and whether it implements `Default` must be checked against `crates/core/src/process/process_control.rs:20-33` before writing this — construct it with whatever its real shape requires. If it has no `Default`, build the struct literal with every field, using `None`/empty values for the process handles. If constructing it proves genuinely impossible without spawning processes, report that rather than deleting the test.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: the query test fails on content type (axum returns `text/plain`); the control test fails to compile or fails its assertion.

- [ ] **Step 3: Make extractor rejections JSON**

In `crates/server/src/error.rs`, add conversions so the extractors produce `ErrorResponse`:

```rust
impl From<axum::extract::rejection::QueryRejection> for ErrorResponse {
    fn from(rejection: axum::extract::rejection::QueryRejection) -> Self {
        ErrorResponse::bad_request(rejection.body_text())
    }
}

impl From<axum::extract::rejection::JsonRejection> for ErrorResponse {
    fn from(rejection: axum::extract::rejection::JsonRejection) -> Self {
        ErrorResponse::bad_request(rejection.body_text())
    }
}
```

Then change the handlers' extractors so those conversions are actually used. axum applies a rejection type from the extractor itself, so the handler signatures change from `Query(params): Query<T>` to a wrapper. The least invasive form is `axum_extra::extract::WithRejection<Query<T>, ErrorResponse>`; if adding `axum-extra` is unwelcome, define a local `ApiQuery<T>` newtype implementing `FromRequestParts` that delegates to `Query<T>` and maps the rejection through `ErrorResponse::from`. Pick one, apply it to every handler in `query.rs` and `control.rs`, and say which you chose in your report.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS — all tests including the two new ones.

- [ ] **Step 5: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-server'
git commit -- crates/server/src/error.rs crates/server/src/game/query.rs crates/server/src/game/control.rs crates/server/tests/game_query.rs crates/server/tests/game_control.rs -m "fix(server): return JSON for extractor rejections and cover player pre-validation"
```

---

## Self-Review

**Spec coverage.** This plan covers the spec's "Process" section (the `serve` subcommand and its bind address), its "Shutdown" section (graceful shutdown that stops the instance), and the state-sharing prerequisite the spec assumed but did not scope — the spec says the server "holds the same `Context`", which is impossible while `AppSettings` lives in the binary crate. Task 6 clears two items the plan-1 final review left open. Deferred to plan 3: the 19 management endpoints, the job registry and SSE, script-execution serialization, `spawn_blocking` for `run_lua` and `extract_archive`, and removing the `gag` stdout redirect. Deferred to plan 4: the frontend and the deletion of Tauri.

**Deviation from plan 1, flagged.** Plan 1 froze `webserver::start`'s signature so the Rocket cutover needed no call-site edits. Task 4 deliberately breaks that freeze now the cutover is done. The two call sites it updates are deleted in plan 4.

**Placeholder scan.** No TBD/TODO entries. Three places tell the implementer to read the real code before writing — `FactorioInstance::stop`'s signature, `FactorioInstance`'s fields, and the `crate::settings::` re-export list — because each depends on a detail that must match exactly rather than be guessed. Task 6 Step 3 offers two named implementations of the same requirement and asks the implementer to report which they chose; that is a genuine judgment call, not a gap.

**Type consistency.** `SharedAppSettings`, `AppState { instance, settings }`, `start(settings, instance_state, bind)` and `start_with_shutdown(..., shutdown)` are used identically wherever they appear. `AppState.settings` changes type in Task 4 from `Arc<RwLock<RestApiSettings>>` to `SharedAppSettings`, which is why the web-root read moves to `settings.restapi.web_root` in the same task.
