# axum Server Replacing Rocket — Implementation Plan (1 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Rocket-based `crates/restapi` with an axum crate that serves all 16 game endpoints, an OpenAPI spec with Swagger UI, and the built SPA — without touching the Tauri GUI, the CLI, or the frontend.

**Architecture:** A new `crates/server` crate exposes `pub async fn start(settings: RestApiSettings, instance_state: SharedFactorioInstance) -> miette::Result<()>` — byte-identical to the Rocket entry point — so `app/src-tauri`'s existing GUI and REPL call sites keep compiling untouched. Handlers move from Rocket's inline `?<param>` syntax to axum `Query<T>` structs with `#[serde(default)]`, from `BadRequest<Json<ErrorResponse>>` to an `IntoResponse` error type, and from `okapi` to `utoipa`. Rocket, `rocket_okapi` and `okapi` leave the workspace at the end.

**Tech Stack:** Rust 2021, axum 0.8, tower-http 0.6, utoipa 5, tokio, miette, serde.

**Spec:** `docs/superpowers/specs/2026-08-29-webserver-replaces-tauri-gui-design.md`

**Follow-on plans (not this one):** (2) management API, job registry and SSE; (3) frontend transport swap and deletion of Tauri.

## Global Constraints

- Workspace root: `/home/arturh/projects/private/factorio-bot`. Rust edition 2021.
- **All commands run inside the Nix devShell with mise tools on PATH.** Prefix every cargo invocation:
  `nix develop --command bash -c 'eval "$(mise env -s bash)"; <command>'`
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated` must pass at the end of every task.
- `cargo fmt --all` before every commit.
- Dependency versions are fixed: `axum = "0.8.9"`, `tower = "0.5.3"`, `tower-http = "0.6.11"`, `utoipa = "5.5.0"`, `utoipa-axum = "0.2.0"`, `utoipa-swagger-ui = "9.0.2"`.
- **`utoipa-swagger-ui` MUST be declared with `features = ["vendored"]`.** Without it, its build script downloads a zip at compile time, breaking CI, offline builds and the private registry mirror.
- **`tower-http` MUST be pinned to `"0.6"`, not `"0.7"`** — 0.6.11 is already in `Cargo.lock` via `reqwest`, and 0.7 would add a second build of the crate.
- Public entry point signature is frozen: `pub async fn start(settings: RestApiSettings, instance_state: SharedFactorioInstance) -> miette::Result<()>`. Call sites at `app/src-tauri/src/gui/command/restapi.rs:30-33` and `app/src-tauri/src/repl/restapi_control.rs:24-27` must not need edits.
- Query-parameter names stay **snake_case** on the wire (`entity_type`, `player_id`, `from_position`). Route paths become kebab-case under `/api/v1/game/`.
- **Every optional query field needs `#[serde(default)]`.** axum treats a missing key as a deserialization error, unlike Rocket which maps it to `None`.
- **No `.unwrap()` on user-supplied input.** `panic = "abort"` is set for release profile in the workspace `Cargo.toml`, so a panic in a handler aborts the whole process.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/server/Cargo.toml` | Crate manifest |
| `crates/server/src/lib.rs` | Module declarations, re-exports |
| `crates/server/src/error.rs` | `ErrorResponse`, `ApiResult<T>`, `IntoResponse`, `From` conversions |
| `crates/server/src/state.rs` | `AppState` |
| `crates/server/src/webserver.rs` | `start()`, router assembly, OpenAPI, static files |
| `crates/server/src/settings.rs` | `RestApiSettings` (moved verbatim from `crates/restapi/src/settings.rs`) |
| `crates/server/src/game/mod.rs` | Game route module tree |
| `crates/server/src/game/query.rs` | Read routes: entities, tiles, players, prototypes, inventory |
| `crates/server/src/game/control.rs` | Mutating routes: move, place, cheat, inventory, admin, research |
| `crates/server/src/spa.rs` | SPA static-file service |
| `crates/server/tests/` | Integration tests driving the router via `tower::ServiceExt::oneshot` |

Splitting game routes into `query.rs` (reads) and `control.rs` (mutations) mirrors the verb split introduced by this plan and keeps each file well under 400 lines. `crates/restapi/src/restapi.rs` is 858 lines today, most of it commented-out dead code.

---

### Task 1: Crate skeleton, error type, health route

**Files:**
- Create: `crates/server/Cargo.toml`, `crates/server/src/lib.rs`, `crates/server/src/error.rs`, `crates/server/src/state.rs`, `crates/server/src/settings.rs`, `crates/server/src/webserver.rs`
- Modify: `Cargo.toml:2-10` (workspace members)
- Test: `crates/server/tests/health.rs`

**Interfaces:**
- Produces: `factorio_bot_server::state::AppState { instance: SharedFactorioInstance, settings: Arc<RwLock<RestApiSettings>> }` (derives `Clone`); `factorio_bot_server::error::{ErrorResponse, ApiResult}`; `factorio_bot_server::webserver::{start, build_router}`; `factorio_bot_server::settings::RestApiSettings`.
- `build_router(state: AppState) -> axum::Router` is the seam every test uses — `start()` binds a listener around it and nothing else.

- [ ] **Step 1: Write the failing test**

Create `crates/server/tests/health.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_server::settings::RestApiSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        instance: Arc::new(RwLock::new(None)),
        settings: Arc::new(RwLock::new(RestApiSettings::default())),
    }
}

#[tokio::test]
async fn health_returns_ok() {
    let app = build_router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: FAIL — the package does not exist yet.

- [ ] **Step 3: Create the crate manifest**

Create `crates/server/Cargo.toml`:

```toml
[package]
name = "factorio-bot-server"
version = "0.2.4-dev"
authors = ["Artur Hallmann <arturh@arturh.de>"]
edition = "2021"

[package.metadata.release]
tag = false
push = false
publish = false

[dependencies]
factorio-bot-core = { path = "../core" }
axum = { version = "0.8.9", features = ["json", "tokio", "http1"] }
tower = "0.5.3"
tower-http = { version = "0.6.11", features = ["fs", "trace", "catch-panic"] }
utoipa = { version = "5.5.0", features = ["axum_extras", "preserve_order"] }
utoipa-axum = "0.2.0"
utoipa-swagger-ui = { version = "9.0.2", features = ["axum", "vendored"] }
tokio = { version = "1", features = ["full", "tracing"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
miette = { version = "7.4", features = ["fancy"] }
thiserror = "2.0"
num-traits = "0.2"
tracing = "0.1"
typescript-definitions = { version = "0.1", package = "typescript-definitions-ufo-patch", features = ["export-typescript"] }

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: Register the crate in the workspace**

In `Cargo.toml`, add `"crates/server",` to `[workspace] members` immediately after `"crates/restapi",`.

- [ ] **Step 5: Write the settings module**

Create `crates/server/src/settings.rs` — copied verbatim from `crates/restapi/src/settings.rs`, with one field added for the SPA root:

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

- [ ] **Step 6: Write the error module**

Create `crates/server/src/error.rs`:

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Error body returned by every failing endpoint.
#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct ErrorResponse {
    pub message: String,
    pub code: u32,
}

impl ErrorResponse {
    pub fn new(message: String, code: u32) -> Self {
        ErrorResponse { message, code }
    }

    /// No Factorio instance is running.
    pub fn not_started() -> Self {
        ErrorResponse::new("not started".into(), 2)
    }

    /// A query parameter could not be parsed.
    pub fn bad_request(message: impl Into<String>) -> Self {
        ErrorResponse::new(message.into(), 1)
    }
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

impl From<miette::Report> for ErrorResponse {
    fn from(report: miette::Report) -> Self {
        ErrorResponse::new(format!("{report}"), 3)
    }
}

pub type ApiResult<T> = Result<Json<T>, ErrorResponse>;
```

- [ ] **Step 7: Write the state module**

Create `crates/server/src/state.rs`:

```rust
use crate::settings::RestApiSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub instance: SharedFactorioInstance,
    pub settings: Arc<RwLock<RestApiSettings>>,
}
```

- [ ] **Step 8: Write the webserver module**

Create `crates/server/src/webserver.rs`:

```rust
use crate::settings::RestApiSettings;
use crate::state::AppState;
use axum::routing::get;
use axum::Router;
use miette::{IntoDiagnostic, Result};
use std::sync::Arc;
use tokio::sync::RwLock;

async fn health() -> &'static str {
    "ok"
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .with_state(state)
}

pub async fn start(
    settings: RestApiSettings,
    instance_state: SharedFactorioInstance,
) -> Result<()> {
    let port = settings.port as u16;
    let state = AppState {
        instance: instance_state,
        settings: Arc::new(RwLock::new(settings)),
    };
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .into_diagnostic()?;
    tracing::info!("restapi listening on http://127.0.0.1:{port}");
    axum::serve(listener, app).await.into_diagnostic()?;
    Ok(())
}
```

Add the missing import at the top: `use factorio_bot_core::process::process_control::SharedFactorioInstance;`

- [ ] **Step 9: Write the crate root**

Create `crates/server/src/lib.rs`:

```rust
pub mod error;
pub mod settings;
pub mod state;
pub mod webserver;
```

- [ ] **Step 10: Run the test to verify it passes**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS — `health_returns_ok`.

- [ ] **Step 11: Verify lints**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all && cargo clippy -p factorio-bot-server --all-targets -- --deny warnings'`
Expected: no output, exit 0.

- [ ] **Step 12: Commit**

```bash
git add Cargo.toml Cargo.lock crates/server
git commit -m "feat(server): add axum crate skeleton with error type and health route"
```

---

### Task 2: Port `find_entities`

**Files:**
- Create: `crates/server/src/game/mod.rs`, `crates/server/src/game/query.rs`
- Modify: `crates/server/src/lib.rs`, `crates/server/src/webserver.rs`
- Test: `crates/server/tests/game_query.rs`
- Reference (do not modify): `crates/restapi/src/restapi.rs:22-59`

**Interfaces:**
- Consumes: `AppState`, `ErrorResponse`, `ApiResult<T>`, `build_router` from Task 1.
- Produces: `crate::game::query::find_entities`; the `FindEntitiesParams` pattern every later route follows; `crate::game::router(state) -> Router` merged by `build_router`.

- [ ] **Step 1: Write the failing tests**

Create `crates/server/tests/game_query.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_server::settings::RestApiSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        instance: Arc::new(RwLock::new(None)),
        settings: Arc::new(RwLock::new(RestApiSettings::default())),
    }
}

async fn get(uri: &str) -> (StatusCode, String) {
    let response = build_router(test_state())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Rocket mapped absent optional params to None; axum needs #[serde(default)]
/// or this returns "missing field" instead of reaching the handler.
#[tokio::test]
async fn find_entities_without_any_params_reports_missing_area() {
    let (status, body) = get("/api/v1/game/find-entities").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("area or position"),
        "expected the handler's own error, got: {body}"
    );
}

#[tokio::test]
async fn find_entities_with_unparseable_area_does_not_panic() {
    let (status, body) = get("/api/v1/game/find-entities?area=garbage").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("area"), "got: {body}");
}

#[tokio::test]
async fn find_entities_without_running_instance_reports_not_started() {
    let (status, body) = get("/api/v1/game/find-entities?position=0,0&radius=10").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("not started"), "got: {body}");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test game_query'`
Expected: FAIL — all three return 404, since the route does not exist.

- [ ] **Step 3: Write the query module**

Create `crates/server/src/game/query.rs`:

```rust
use crate::error::{ApiResult, ErrorResponse};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::Json;
use factorio_bot_core::types::{AreaFilter, FactorioEntity};
use serde::Deserialize;
use utoipa::IntoParams;

#[derive(Debug, Deserialize, IntoParams)]
#[serde(default)]
pub struct FindEntitiesParams {
    pub area: Option<String>,
    pub position: Option<String>,
    pub radius: Option<f64>,
    pub name: Option<String>,
    pub entity_type: Option<String>,
}

impl Default for FindEntitiesParams {
    fn default() -> Self {
        FindEntitiesParams {
            area: None,
            position: None,
            radius: None,
            name: None,
            entity_type: None,
        }
    }
}

/// Finds entities in given area/radius
#[utoipa::path(
    get,
    path = "/api/v1/game/find-entities",
    tag = "Query",
    params(FindEntitiesParams),
    responses(
        (status = 200, body = Vec<FactorioEntity>),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn find_entities(
    State(state): State<AppState>,
    Query(params): Query<FindEntitiesParams>,
) -> ApiResult<Vec<FactorioEntity>> {
    let area_filter = match &params.area {
        Some(area) => AreaFilter::Rect(
            area.parse()
                .map_err(|_| ErrorResponse::bad_request(format!("invalid area: {area}")))?,
        ),
        None => match &params.position {
            Some(position) => AreaFilter::PositionRadius((
                position
                    .parse()
                    .map_err(|_| {
                        ErrorResponse::bad_request(format!("invalid position: {position}"))
                    })?,
                params.radius,
            )),
            None => {
                return Err(ErrorResponse::bad_request(
                    "area or position + optional radius needed",
                ))
            }
        },
    };

    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let entities = instance
        .rcon
        .find_entities_filtered(&area_filter, params.name.clone(), params.entity_type.clone())
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(entities))
}
```

- [ ] **Step 4: Write the game module router**

Create `crates/server/src/game/mod.rs`:

```rust
pub mod query;

use crate::state::AppState;
use axum::routing::get;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new().route("/find-entities", get(query::find_entities))
}
```

- [ ] **Step 5: Mount it**

In `crates/server/src/lib.rs`, add `pub mod game;` above `pub mod settings;`.

In `crates/server/src/webserver.rs`, change `build_router` to:

```rust
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .nest("/api/v1/game", crate::game::router())
        .with_state(state)
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS — four tests.

If `find_entities_without_any_params_reports_missing_area` fails with a body containing "missing field", the `#[serde(default)]` attribute is missing or misplaced. It must be on the struct, not on individual fields.

- [ ] **Step 7: Verify lints**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all && cargo clippy -p factorio-bot-server --all-targets -- --deny warnings'`
Expected: exit 0. Note clippy will flag the manual `Default` impl as `derivable_impls` only if every field is `None` and no doc comments intervene — if flagged, replace it with `#[derive(Default)]` on the struct and delete the manual impl.

- [ ] **Step 8: Commit**

```bash
git add crates/server
git commit -m "feat(server): port find-entities to axum with typed query params"
```

---

### Task 3: Port the remaining read routes

**Files:**
- Modify: `crates/server/src/game/query.rs`, `crates/server/src/game/mod.rs`
- Test: `crates/server/tests/game_query.rs`
- Reference (do not modify): `crates/restapi/src/restapi.rs` at `:101-137` (`find_tiles`), `:139-170` (`inventory_contents_at`), `:202-224` (`player_info`), `:405-422` (`all_players`), `:424-442` (`item_prototypes`), `:443-461` (`entity_prototypes`)

**Interfaces:**
- Consumes: everything from Task 2.
- Produces: `query::{find_tiles, inventory_contents_at, player_info, all_players, item_prototypes, entity_prototypes}`.

Route table for this task — every path is `GET` under `/api/v1/game`:

| Function | Path | Params struct | Response type |
|---|---|---|---|
| `find_tiles` | `/find-tiles` | `FindTilesParams { area: Option<String>, position: Option<String>, radius: Option<f64>, name: Option<String> }` | `Vec<FactorioTile>` |
| `inventory_contents_at` | `/inventory-contents-at` | `InventoryContentsAtParams { query: String }` | `Vec<Option<InventoryResponse>>` |
| `player_info` | `/player-info` | `PlayerInfoParams { player_id: PlayerId }` | `FactorioPlayer` |
| `all_players` | `/all-players` | none | `Vec<FactorioPlayer>` |
| `item_prototypes` | `/item-prototypes` | none | `HashMap<String, FactorioItemPrototype>` |
| `entity_prototypes` | `/entity-prototypes` | none | `HashMap<String, FactorioEntityPrototype>` |

- [ ] **Step 1: Write the failing tests**

Append to `crates/server/tests/game_query.rs`:

```rust
#[tokio::test]
async fn read_routes_report_not_started_without_instance() {
    for uri in [
        "/api/v1/game/find-tiles?position=0,0&radius=10",
        "/api/v1/game/player-info?player_id=1",
        "/api/v1/game/all-players",
        "/api/v1/game/item-prototypes",
        "/api/v1/game/entity-prototypes",
    ] {
        let (status, body) = get(uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert!(body.contains("not started"), "{uri} returned: {body}");
    }
}

#[tokio::test]
async fn inventory_contents_at_rejects_query_without_separator() {
    let (status, body) = get("/api/v1/game/inventory-contents-at?query=nonsense").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body.is_empty());
}

#[tokio::test]
async fn player_info_rejects_out_of_range_player_id() {
    let (status, _body) = get("/api/v1/game/player-info?player_id=300").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test game_query'`
Expected: FAIL — routes return 404.

- [ ] **Step 3: Implement the six handlers**

Each handler is a transcription of the referenced lines in `crates/restapi/src/restapi.rs`, with four mechanical changes:

1. Rocket's `instance_state: &State<SharedFactorioInstance>` becomes `State(state): State<AppState>` and `state.instance.read().await`.
2. The `if let Some(instance_state) = &*instance_state { … } else { Err(ErrorResponse::new("not started".into(), 2)) }` block becomes
   `let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;`.
3. Loose params become a `#[derive(Debug, Default, Deserialize, IntoParams)] #[serde(default)]` struct extracted with `Query<T>`, using the exact names in the table above.
4. **Every `.unwrap()` on parsed input or on an rcon result becomes `?`** — `.map_err(|_| ErrorResponse::bad_request(…))?` for parses, `.map_err(ErrorResponse::from)?` for `miette` results. `inventory_contents_at` additionally indexes `parts[1]` at `restapi.rs:98`; replace with
   ```rust
   let mut parts = query.split('@');
   let name = parts.next().unwrap_or_default().to_owned();
   let position = parts
       .next()
       .ok_or_else(|| ErrorResponse::bad_request("expected <name>@<position>"))?;
   ```

Add each to `game/mod.rs`:

```rust
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/find-entities", get(query::find_entities))
        .route("/find-tiles", get(query::find_tiles))
        .route("/inventory-contents-at", get(query::inventory_contents_at))
        .route("/player-info", get(query::player_info))
        .route("/all-players", get(query::all_players))
        .route("/item-prototypes", get(query::item_prototypes))
        .route("/entity-prototypes", get(query::entity_prototypes))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS.

- [ ] **Step 5: Verify no unwraps survive on user input**

Run: `grep -n "unwrap()" crates/server/src/game/query.rs`
Expected: no matches. If any remain, they are bugs — `panic = "abort"` makes them process-fatal.

- [ ] **Step 6: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all'
git add crates/server
git commit -m "feat(server): port remaining read endpoints to axum"
```

---

### Task 4: Port the mutating routes as POST

**Files:**
- Create: `crates/server/src/game/control.rs`
- Modify: `crates/server/src/game/mod.rs`
- Test: `crates/server/tests/game_control.rs`
- Reference (do not modify): `crates/restapi/src/restapi.rs` at `:172-200` (`move_player`), `:226-262` (`place_entity`), `:264-291` (`cheat_item`), `:292-307` (`cheat_technology`), `:308-321` (`cheat_all_technologies`), `:323-362` (`insert_to_inventory`), `:364-403` (`remove_from_inventory`), `:462-476` (`server_save`), `:477-489` (`add_research`)

**Interfaces:**
- Consumes: `AppState`, `ErrorResponse`, `ApiResult<T>`.
- Produces: `control::{move_player, place_entity, cheat_item, cheat_technology, cheat_all_technologies, insert_to_inventory, remove_from_inventory, server_save, add_research}`.

Route table — every path is `POST` under `/api/v1/game`, taking its parameters as a **JSON body** rather than a query string:

| Function | Path | Body struct fields | Response |
|---|---|---|---|
| `move_player` | `/move-player` | `player_id: PlayerId, goal: String, radius: Option<f64>` | `FactorioPlayer` |
| `place_entity` | `/place-entity` | `player_id: PlayerId, item: String, position: String, direction: u8` | `PlaceEntityResult` |
| `cheat_item` | `/cheat-item` | `name: String, count: u32, player_id: PlayerId` | `FactorioPlayer` |
| `cheat_technology` | `/cheat-technology` | `tech: String` | `OperationResult` |
| `cheat_all_technologies` | `/cheat-all-technologies` | none | `OperationResult` |
| `insert_to_inventory` | `/insert-to-inventory` | `player_id: PlayerId, entity_name: String, entity_position: String, inventory_type: u32, item_name: String, item_count: u32` | `FactorioPlayer` |
| `remove_from_inventory` | `/remove-from-inventory` | same six fields as above | `FactorioPlayer` |
| `server_save` | `/server-save` | none | `OperationResult` |
| `add_research` | `/add-research` | `tech: String` | `OperationResult` |

Note the old path for `remove_from_inventory` was `/removeToInventory` — a typo. The new path fixes it.

`OperationResult` moves into `control.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub success: bool,
}
```

- [ ] **Step 1: Write the failing tests**

Create `crates/server/tests/game_control.rs`:

```rust
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use factorio_bot_server::settings::RestApiSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        instance: Arc::new(RwLock::new(None)),
        settings: Arc::new(RwLock::new(RestApiSettings::default())),
    }
}

async fn post(uri: &str, body: &str) -> (StatusCode, String) {
    let response = build_router(test_state())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn mutating_routes_report_not_started_without_instance() {
    for (uri, body) in [
        ("/api/v1/game/move-player", r#"{"player_id":1,"goal":"0,0"}"#),
        ("/api/v1/game/cheat-all-technologies", "{}"),
        ("/api/v1/game/server-save", "{}"),
        ("/api/v1/game/add-research", r#"{"tech":"automation"}"#),
    ] {
        let (status, response_body) = post(uri, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert!(response_body.contains("not started"), "{uri}: {response_body}");
    }
}

/// direction > 7 hit `Direction::from_u8(...).unwrap()` in the Rocket version,
/// which aborts the process under `panic = "abort"`.
#[tokio::test]
async fn place_entity_rejects_out_of_range_direction() {
    let (status, body) = post(
        "/api/v1/game/place-entity",
        r#"{"player_id":1,"item":"stone-furnace","position":"0,0","direction":99}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("direction"), "got: {body}");
}

#[tokio::test]
async fn move_player_rejects_unparseable_goal() {
    let (status, body) = post(
        "/api/v1/game/move-player",
        r#"{"player_id":1,"goal":"not-a-position"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("goal") || body.contains("position"), "got: {body}");
}

#[tokio::test]
async fn get_is_not_allowed_on_mutating_routes() {
    let response = build_router(test_state())
        .oneshot(
            Request::builder()
                .uri("/api/v1/game/server-save")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test game_control'`
Expected: FAIL — routes return 404.

- [ ] **Step 3: Implement the nine handlers**

Same four mechanical transformations as Task 3, with two differences: parameters arrive as `Json(body): Json<XBody>` instead of `Query`, and the `Json` extractor must be the **last** argument (it consumes the request body). `direction` needs an explicit check replacing `Direction::from_u8(to_direction).unwrap()`:

```rust
let direction = Direction::from_u8(body.direction)
    .ok_or_else(|| ErrorResponse::bad_request(format!("invalid direction: {}", body.direction)))?;
```

Handler shape:

```rust
#[utoipa::path(
    post,
    path = "/api/v1/game/move-player",
    tag = "Control",
    request_body = MovePlayerBody,
    responses(
        (status = 200, body = FactorioPlayer),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn move_player(
    State(state): State<AppState>,
    Json(body): Json<MovePlayerBody>,
) -> ApiResult<FactorioPlayer> {
    // transcribed from crates/restapi/src/restapi.rs:174-200
}
```

Register all nine in `game/mod.rs` with `.route("/move-player", post(control::move_player))` and so on, adding `use axum::routing::post;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS.

- [ ] **Step 5: Verify no unwraps survive**

Run: `grep -n "unwrap()" crates/server/src/game/control.rs`
Expected: no matches.

- [ ] **Step 6: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all'
git add crates/server
git commit -m "feat(server): port mutating game endpoints as POST"
```

---

### Task 5: OpenAPI spec and Swagger UI

**Files:**
- Create: `crates/server/src/openapi.rs`
- Modify: `crates/server/src/lib.rs`, `crates/server/src/webserver.rs`, `crates/server/src/game/mod.rs`
- Test: `crates/server/tests/openapi.rs`

**Interfaces:**
- Consumes: every handler from Tasks 2-4.
- Produces: `crate::openapi::ApiDoc`; `/openapi.json`; Swagger UI at `/swagger-ui`.

**Note:** the router is rebuilt with `utoipa_axum::router::OpenApiRouter` so one route table feeds both the server and the spec — the structural equivalent of Rocket's `openapi_get_routes!`. `game::router()` changes return type from `Router<AppState>` to `OpenApiRouter<AppState>` and each `.route(path, get(handler))` becomes `.routes(routes!(handler))`, which derives the path from the `#[utoipa::path]` attribute.

- [ ] **Step 1: Write the failing test**

Create `crates/server/tests/openapi.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_server::settings::RestApiSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        instance: Arc::new(RwLock::new(None)),
        settings: Arc::new(RwLock::new(RestApiSettings::default())),
    }
}

#[tokio::test]
async fn openapi_json_lists_every_route() {
    let response = build_router(test_state())
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let paths = spec["paths"].as_object().expect("paths object");

    for path in [
        "/api/v1/game/find-entities",
        "/api/v1/game/find-tiles",
        "/api/v1/game/inventory-contents-at",
        "/api/v1/game/player-info",
        "/api/v1/game/all-players",
        "/api/v1/game/item-prototypes",
        "/api/v1/game/entity-prototypes",
        "/api/v1/game/plan-path",
        "/api/v1/game/move-player",
        "/api/v1/game/place-entity",
        "/api/v1/game/cheat-item",
        "/api/v1/game/cheat-technology",
        "/api/v1/game/cheat-all-technologies",
        "/api/v1/game/insert-to-inventory",
        "/api/v1/game/remove-from-inventory",
        "/api/v1/game/server-save",
        "/api/v1/game/add-research",
    ] {
        assert!(paths.contains_key(path), "spec is missing {path}");
    }
}

#[tokio::test]
async fn swagger_ui_is_served() {
    let response = build_router(test_state())
        .oneshot(
            Request::builder()
                .uri("/swagger-ui/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_success() || response.status().is_redirection(),
        "got {}",
        response.status()
    );
}
```

This test is also the completeness gate for Tasks 2-4: it fails if any route was skipped. `plan-path` is included, so **Task 5 requires `plan_path` to exist** — port it here if it is not already done, transcribing `crates/restapi/src/restapi.rs:61-99` as a `GET /api/v1/game/plan-path` with a `PlanPathParams` struct carrying all eight fields (`entity_name`, `entity_type`, `underground_entity_name`, `underground_entity_type`, `underground_max: u8`, `from_position`, `to_position`, `to_direction: u8`), all required, with `to_direction` validated through `Direction::from_u8`. Drop the `#[allow(clippy::too_many_arguments)]` from `restapi.rs:64` — collapsing the arguments into a struct makes it unnecessary, and an unused `allow` trips the workspace lint settings.

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test openapi'`
Expected: FAIL — `/openapi.json` returns 404.

- [ ] **Step 3: Write the openapi module**

Create `crates/server/src/openapi.rs`:

```rust
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "factorio-bot", description = "factorio-bot HTTP API"),
    tags(
        (name = "Query", description = "World queries"),
        (name = "Control", description = "Bot control"),
        (name = "Place", description = "Entity placement"),
        (name = "Cheat", description = "Cheats"),
        (name = "Inventory", description = "Inventory manipulation"),
        (name = "Admin", description = "Server administration"),
        (name = "Research", description = "Research"),
    )
)]
pub struct ApiDoc;
```

Add `pub mod openapi;` to `crates/server/src/lib.rs`.

- [ ] **Step 4: Convert the game router to `OpenApiRouter`**

In `crates/server/src/game/mod.rs`:

```rust
use crate::state::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(query::find_entities))
        .routes(routes!(query::find_tiles))
        .routes(routes!(query::inventory_contents_at))
        .routes(routes!(query::player_info))
        .routes(routes!(query::all_players))
        .routes(routes!(query::item_prototypes))
        .routes(routes!(query::entity_prototypes))
        .routes(routes!(query::plan_path))
        .routes(routes!(control::move_player))
        .routes(routes!(control::place_entity))
        .routes(routes!(control::cheat_item))
        .routes(routes!(control::cheat_technology))
        .routes(routes!(control::cheat_all_technologies))
        .routes(routes!(control::insert_to_inventory))
        .routes(routes!(control::remove_from_inventory))
        .routes(routes!(control::server_save))
        .routes(routes!(control::add_research))
}
```

`routes!` reads the full path from each `#[utoipa::path(path = "/api/v1/game/…")]` attribute, so the router is **not** nested — merge it at the root instead.

- [ ] **Step 5: Assemble the router in webserver.rs**

```rust
pub fn build_router(state: AppState) -> Router {
    let (router, api) = OpenApiRouter::with_openapi(crate::openapi::ApiDoc::openapi())
        .route("/api/v1/health", get(health))
        .merge(crate::game::router())
        .with_state(state)
        .split_for_parts();

    router.merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", api))
}
```

Imports to add: `use utoipa::OpenApi;`, `use utoipa_axum::router::OpenApiRouter;`, `use utoipa_swagger_ui::SwaggerUi;`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS — every route appears in the spec.

- [ ] **Step 7: Verify the build did not fetch anything from the network**

Run: `grep -n 'utoipa-swagger-ui' crates/server/Cargo.toml`
Expected: the line contains `features = ["axum", "vendored"]`. Without `vendored`, the build script downloads a zip and CI breaks.

- [ ] **Step 8: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all'
git add crates/server
git commit -m "feat(server): generate OpenAPI spec and serve Swagger UI"
```

---

### Task 6: Serve the SPA

**Files:**
- Create: `crates/server/src/spa.rs`
- Modify: `crates/server/src/lib.rs`, `crates/server/src/webserver.rs`
- Test: `crates/server/tests/spa.rs`

**Interfaces:**
- Consumes: `RestApiSettings::web_root` from Task 1, `build_router` from Task 5.
- Produces: `crate::spa::service(web_root: Option<&str>) -> Option<axum::routing::MethodRouter>`; `build_router` gains SPA fallback behavior.

- [ ] **Step 1: Write the failing test**

Create `crates/server/tests/spa.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_server::settings::RestApiSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn state_with_web_root(dir: &std::path::Path) -> AppState {
    AppState {
        instance: Arc::new(RwLock::new(None)),
        settings: Arc::new(RwLock::new(RestApiSettings {
            port: 7492,
            web_root: Some(dir.to_string_lossy().into_owned()),
        })),
    }
}

#[tokio::test]
async fn serves_index_html_at_root() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>spa</html>").unwrap();

    let response = build_router(state_with_web_root(dir.path()))
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(String::from_utf8(bytes.to_vec()).unwrap(), "<html>spa</html>");
}

#[tokio::test]
async fn unknown_path_falls_back_to_index_html() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>spa</html>").unwrap();

    let response = build_router(state_with_web_root(dir.path()))
        .oneshot(
            Request::builder()
                .uri("/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_404_is_not_swallowed_by_the_spa_fallback() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>spa</html>").unwrap();

    let response = build_router(state_with_web_root(dir.path()))
        .oneshot(
            Request::builder()
                .uri("/api/v1/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn server_starts_without_a_web_root() {
    let response = build_router(AppState {
        instance: Arc::new(RwLock::new(None)),
        settings: Arc::new(RwLock::new(RestApiSettings::default())),
    })
    .oneshot(
        Request::builder()
            .uri("/api/v1/health")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server --test spa'`
Expected: FAIL — `/` returns 404.

- [ ] **Step 3: Write the spa module**

Create `crates/server/src/spa.rs`:

```rust
use axum::routing::{get_service, MethodRouter};
use std::path::Path;
use tower_http::services::{ServeDir, ServeFile};

/// Builds a static-file service for the SPA. Returns `None` when no web root is
/// configured or the directory does not exist, in which case the API is served
/// without a frontend.
pub fn service(web_root: Option<&str>) -> Option<MethodRouter> {
    let root = web_root?;
    let path = Path::new(root);
    if !path.is_dir() {
        tracing::warn!("web root {root} does not exist, serving API only");
        return None;
    }
    let index = path.join("index.html");
    Some(get_service(
        ServeDir::new(path).fallback(ServeFile::new(index)),
    ))
}
```

Add `pub mod spa;` to `crates/server/src/lib.rs`.

- [ ] **Step 4: Wire the fallback**

`build_router` needs the web root before consuming `state`, and the API 404 must not be swallowed. Read the configured root first, then attach the fallback last:

```rust
pub fn build_router(state: AppState) -> Router {
    let web_root = state
        .settings
        .try_read()
        .ok()
        .and_then(|settings| settings.web_root.clone());

    let (router, api) = OpenApiRouter::with_openapi(crate::openapi::ApiDoc::openapi())
        .route("/api/v1/health", get(health))
        .merge(crate::game::router())
        .with_state(state)
        .split_for_parts();

    let router = router.merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", api));

    match crate::spa::service(web_root.as_deref()) {
        Some(spa) => router
            .route("/api/v1/{*rest}", axum::routing::any(api_not_found))
            .fallback_service(spa),
        None => router,
    }
}

async fn api_not_found() -> crate::error::ErrorResponse {
    crate::error::ErrorResponse::new("not found".into(), 404)
}
```

The explicit `/api/v1/{*rest}` catch-all is what keeps unmatched API paths returning a JSON 404 instead of the SPA's `index.html`. Note axum 0.8 uses `{*rest}` syntax, not the older `:rest`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-server'`
Expected: PASS — all four SPA tests plus everything earlier.

- [ ] **Step 6: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all'
git add crates/server
git commit -m "feat(server): serve the built SPA with index.html fallback"
```

---

### Task 7: Cut the workspace over and delete Rocket

**Files:**
- Modify: `app/src-tauri/Cargo.toml`, `app/src-tauri/src/gui/command/restapi.rs:1-15`, `app/src-tauri/src/repl/restapi_control.rs:1-15`, `app/src-tauri/src/settings.rs:25-26`, `app/src-tauri/build.rs:44-45,90`, `Cargo.toml` (workspace members)
- Delete: `crates/restapi/` (entire directory)
- Test: the whole workspace suite

**Interfaces:**
- Consumes: `factorio_bot_server::webserver::start` and `factorio_bot_server::settings::RestApiSettings` — the same names and signature `crates/restapi` exposed.

- [ ] **Step 1: Confirm the call sites compile against the new crate**

The only changes are import paths. In `app/src-tauri/Cargo.toml`, replace

```toml
factorio-bot-restapi = { path = "../../crates/restapi", optional = true }
```

with

```toml
factorio-bot-server = { path = "../../crates/server", optional = true }
```

and update the feature: `restapi = ["dep:factorio-bot-server"]`.

Then replace every `factorio_bot_restapi::` with `factorio_bot_server::` — the affected files are `app/src-tauri/src/gui/command/restapi.rs`, `app/src-tauri/src/repl/restapi_control.rs`, `app/src-tauri/src/settings.rs` and `app/src-tauri/build.rs`.

Run: `grep -rn "factorio_bot_restapi\|factorio-bot-restapi" app crates Cargo.toml`
Expected: no matches once the edits are done.

- [ ] **Step 2: Build to verify the cutover**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo build --all-features'`
Expected: success. `RestApiSettings` gained a `web_root` field in Task 1, so `app/src/models/types.ts` will be regenerated by `build.rs` — that is expected and should be committed.

- [ ] **Step 3: Delete the Rocket crate**

```bash
git rm -r crates/restapi
```

Remove `"crates/restapi",` from `[workspace] members` in the root `Cargo.toml`.

- [ ] **Step 4: Verify Rocket is gone from the lockfile**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo update -w'` then
`grep -c '^name = "rocket' Cargo.lock`
Expected: `0`. Also expect `okapi`, `figment`, `devise`, `pear` and `multer` to disappear — roughly 28 packages in total.

- [ ] **Step 5: Run the full suite**

Run:
```
nix develop --command bash -c 'eval "$(mise env -s bash)"; \
  cargo build --all-features && \
  cargo build --no-default-features && \
  cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated && \
  cargo test --workspace --all-features'
```
Expected: all four succeed. The `crates/scripting_lua/tests/*.dot|.md|.png` snapshot files will be rewritten by the test run; revert them with `git checkout -- crates/scripting_lua/tests/` — they are stale in the repository and unrelated to this change.

- [ ] **Step 6: Smoke-test the running server**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo run --no-default-features --features cli,repl,restapi,lua -- repl'`
then at the prompt: `restapi start`

In another terminal:
```bash
curl -s localhost:7492/api/v1/health
curl -s localhost:7492/openapi.json | head -c 200
curl -s "localhost:7492/api/v1/game/all-players"
```
Expected: `ok`; a JSON spec; and `{"message":"not started","code":2}` with HTTP 400.

- [ ] **Step 7: Commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt --all'
git add -A
git commit -m "refactor: replace rocket restapi crate with axum server

Removes rocket, rocket_okapi, okapi and ~28 transitive dependencies."
```

---

## Self-Review

**Spec coverage.** Every spec item in scope for plan 1 maps to a task: the new crate and `start()` signature (T1), all 16 game routes (T2-T5, with `plan_path` gated by the T5 completeness test), kebab-case `/api/v1/game/` paths and POST for mutations (T4), `#[serde(default)]` on optional params (T2 test), unwrap removal (T3/T4 grep steps), utoipa with vendored Swagger UI (T5), `ServeDir` with index fallback and API-404 preservation (T6), deletion of Rocket and its dead files (T7). Spec items deliberately deferred to plans 2 and 3: the job registry and SSE, the 19 management endpoints, script serialization and `spawn_blocking`, graceful shutdown, the `serve` subcommand, the frontend, and the CI/flake/packaging cleanup.

**Deviation from the spec, flagged.** The spec says mutating game routes become `POST`; this plan additionally moves their parameters from the query string into a JSON body, which the spec did not state. It follows from the verb change and keeps the six-field inventory routes readable. If wire-compatible query parameters on POST are preferred, swap `Json<T>` for `Query<T>` in Task 4 — the handler bodies are unaffected.

**Placeholder scan.** No TBD/TODO entries. Tasks 3 and 4 describe nine and six handler bodies by transcription rule plus exact line references rather than reproducing ~400 lines of existing code; every non-mechanical part (the `parts[1]` index, the `Direction::from_u8` unwrap, the `not started` branch) is written out in full.

**Type consistency.** `AppState { instance, settings }`, `ErrorResponse::{new, not_started, bad_request}`, `ApiResult<T>`, `build_router`, `crate::game::router()` and `crate::spa::service()` are used identically in every task that references them. `game::router()` changes return type in Task 5 (`Router<AppState>` → `OpenApiRouter<AppState>`), which is called out explicitly in that task.
