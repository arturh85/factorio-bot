# Replacing the Tauri GUI with an axum web server

**Date:** 2026-08-29
**Status:** Approved design, not yet implemented

## Why

The GUI needs to be reachable from another machine while Factorio runs on the
server. A Tauri window cannot do that: the Vue app talks to the backend
exclusively over Tauri IPC (`invoke`), which only exists inside the desktop
webview.

The desktop shell is dropped rather than kept alongside HTTP. Maintaining both
transports means every backend operation exists twice.

## Decisions

| Decision | Choice |
|---|---|
| Desktop shell | Deleted. Browser-only. |
| Server | One axum server: SPA + management API + game API. |
| Rocket | Deleted, handlers ported to axum. |
| OpenAPI | `utoipa` + `utoipa-swagger-ui` (`vendored` feature). |
| Auth | None. Bind address is config, defaults to `127.0.0.1`. |
| Long operations | Job registry + Server-Sent Events from the start. |
| Script concurrency | Serialized. Second request gets `409` plus the running job id. |
| Server-side paths | Plain text input with server-side validation. No filesystem browsing endpoint. |
| Game routes | All 16 ported and mounted (only 2 are reachable today). |

### On the lack of authentication

The API executes arbitrary Lua, starts and stops processes, and reads and writes
files on the host. It is remote code execution by design. Binding it to anything
other than loopback or a VPN interface hands the machine to whoever reaches the
port. The default bind is `127.0.0.1`; a wider bind is an explicit opt-in.

## Architecture

### Crates

- **`crates/server`** (new) — the axum application: static SPA serving, the
  management API, the game API ported from `crates/restapi`, the job registry,
  and OpenAPI generation.
- **`crates/restapi`** — deleted once its handlers move.
- **`crates/cli`** (moved from `app/src-tauri`) — the `factorio-bot` binary.
  Keeps `cli/` and `repl/`; `gui/` is deleted.
- **`app/`** — the Vue SPA only, which is what the directory already claims to be.

`app/src-tauri/build.rs` generates `app/src/models/types.ts` from
`crates/core/src/types.rs` under `#[cfg(feature = "gui")]`. It moves to
`crates/server/build.rs`. Without this, dropping the `gui` feature silently stops
TypeScript type generation.

### Features

`gui` and `custom-protocol` are removed along with `tauri`, `tauri-build`,
`open` and `port_scanner`. `restapi` becomes `server`. `cli`, `repl` and `lua`
are unchanged. Default: `["cli", "repl", "lua", "server"]`.

### Process

```
factorio-bot serve --bind 127.0.0.1:8000 [--web-root DIR]
```

Startup takes over what `Context::new()` (`app/src-tauri/src/context.rs:21-38`)
does today: `color_eyre::install()`, directory creation, and settings load. The
server holds the same `Context` — it already derives `Clone` and every field is
an `Arc`, so it becomes the axum state directly, minus `restapi_handle`.

Autostart currently fires on first page load (`app/src/App.vue:188-190`), which
with a browser means "whenever anyone opens a tab". It moves to server startup.

### Shutdown

`FactorioInstance` has no `Drop` impl and `stop()` is only ever called
explicitly (`gui/command/instances.rs:78`, `repl/quit.rs:10`, `repl/mod.rs:53`,
`cli/lua.rs:180`). On the desktop, closing the window let the OS reap the child
processes. A long-lived server that is `SIGTERM`ed would orphan them, so
`axum::serve(...).with_graceful_shutdown(...)` takes the instance and stops it.

## HTTP API

JSON in and out, under `/api/v1`. One error type implementing `IntoResponse`,
rendering `{ "error": …, "detail": … }` with a real status code, replacing both
Tauri's stringly-typed `Result<T, String>` and `crates/restapi/src/error.rs`.

### Management endpoints

| Command today | Endpoint | Notes |
|---|---|---|
| `load_settings` | `GET /api/v1/settings` | |
| `update_settings` | `PUT /api/v1/settings` | |
| `save_settings` | — | No caller. Deleted. |
| `is_instance_started` | `GET /api/v1/instance` | Enriched with `client_count`, ports, `silent` |
| `start_instances` | `POST /api/v1/instance/start` | `202` + job id |
| `stop_instances` | `POST /api/v1/instance/stop` | |
| `execute_rcon` | `POST /api/v1/rcon` | Returns `204`, matching today. The RCON reply is discarded at `gui/command/rcon.rs:13-17`; surfacing it is a follow-up, not part of this change. |
| `load_scripts_in_directory` | `GET /api/v1/scripts?path=` | |
| `load_script` | `GET /api/v1/scripts/file?path=` | |
| `save_script` | `PUT /api/v1/scripts/file?path=` | |
| `execute_script` | `POST /api/v1/scripts/run` | `202` + job id |
| `execute_code` | `POST /api/v1/scripts/eval` | `202` + job id |
| `file_exists` | `GET /api/v1/fs/exists?path=` | Server-side path check |
| `maximize_window` | — | Desktop-only. Deleted. |
| `open_in_browser` | — | Becomes `<a target="_blank">`. Deleted. |
| `start_restapi` / `stop_restapi` / `is_restapi_started` | — | Meaningless once the server *is* the API. Deleted. |
| `is_port_available` | — | Existed only to validate the REST API port. Deleted with it. |

`execute_script`/`execute_code` currently serialize a Rust tuple to a JSON array
that the stores index as `[0]`/`[1]`. Responses become `{stdout, stderr}`.

Every `path` is a tree key with a leading `/`, stripped Rust-side with
`&path[1..]`. The traversal guard is a literal `if path.contains("..")`
(`gui/command/script.rs:98,152,186`). With a remote server this stops being a
local-user footgun and becomes an attack surface; it gets a real
canonicalize-and-verify-prefix check.

### Game endpoints

All 16 handlers in `crates/restapi/src/restapi.rs` are ported and mounted. Only
`findEntities` and `planPath` are reachable today (`webserver.rs:45-48`); the
other 14 compile but are dead.

They mount under `/api/v1/game/` with kebab-case paths — `find-entities`,
`plan-path`, `move-player`, `place-entity`, `cheat-item` and so on — rather than
the current camelCase paths at the root. This breaks the wire format, which is
acceptable: only two routes were reachable, and the root path is needed for the
SPA. Query parameter names stay snake_case, unchanged.

Three behavioral notes:

- **Verbs.** Every route is `GET` today, including `movePlayer`, `placeEntity`
  and `cheatItem`. Mutations become `POST` while the handlers are being rewritten
  anyway.
- **Query parameters.** Rocket maps an absent or unparseable `Option<T>` to
  `None`. axum's `Query<T>` treats a missing key as a deserialization error, so
  every optional field needs `#[serde(default)]`, and empty values (`?radius=`)
  need `deserialize_with` to match. Getting this wrong silently breaks both live
  callers.
- **Panics.** `restapi.rs` has ~40 `.unwrap()`s on user input — `area.parse()`
  at `:34`, `Direction::from_u8(to_direction).unwrap()` at `:36` (any value > 7),
  `parts[1]` at `:98`. With `panic = "abort"` in release these abort the process,
  so today they are a remotely-triggerable kill switch. They become `?` with a
  `From<ParseError>` impl.

### Jobs and SSE

State gains `jobs: Arc<RwLock<HashMap<JobId, Job>>>` where
`Job { kind, status, started_at, lines, tx: broadcast::Sender<JobEvent> }`.

- Long operations return `202` with `{ "job_id": … }`.
- `GET /api/v1/jobs/{id}` — status snapshot.
- `GET /api/v1/jobs/{id}/events` — SSE emitting `line`, `status`, `done`.

The `lines` buffer is replayed on connect so a late-joining or refreshed browser
sees the whole run. Jobs live in memory, capped (last 50, with per-job line
caps); nothing survives a restart.

### OpenAPI and static files

`utoipa` annotations plus `utoipa-axum`'s `OpenApiRouter::split_for_parts()`, so
one route table feeds both the server and the spec. Swagger UI stays at
`/swagger-ui` and the spec at `/openapi.json`, matching the URLs
`app/src/pages/SettingsPage.vue:208-212` advertises today.

`GET /` currently 308-redirects to RapiDoc. It becomes the SPA; RapiDoc moves to
`/rapidoc` or is dropped as redundant with Swagger UI.

The SPA is served with `tower_http::services::ServeDir` plus an `index.html`
fallback, rooted at `--web-root` (default: resolved relative to the executable).
Embedding via `rust-embed` stays behind an off-by-default `embed-spa` feature so
that a bare `cargo build` does not require a frontend build first.

`utoipa-swagger-ui` **must** be depended on with `features = ["vendored"]`. Its
build script otherwise downloads the Swagger UI distribution at compile time,
which breaks CI, offline builds, and the private registry mirror.

## Execution model

**Script output capture is process-global today.** `execute_code` and
`execute_script` pass `redirect = true`, reaching
`crates/scripting/src/lib.rs:5-15`, which hijacks fd 1 and 2 for the whole
process with `gag::BufferRedirect` and drains it only at the end
(`crates/scripting_lua/src/lua_runner.rs:91-93`). It cannot survive concurrent
requests and it swallows unrelated logging while held. The CLI and REPL already
pass `false` (`cli/lua.rs:126,168`, `repl/run_script.rs:29`), so the gag path is
GUI-only and is deleted. The per-run capture at `lua_runner.rs:26-27` — an
`Arc<Mutex<String>>` — becomes an `mpsc::Sender<String>` that both streams to
SSE and accumulates for the final response.

**Script execution is serialized.** There is one `Option<FactorioInstance>`
process-wide and script runs mutate it through a shared `Planner`. A mutex
guards execution; a second request returns `409 Conflict` carrying the running
job's id so the UI can attach to its stream instead.

**Blocking work goes through `spawn_blocking`.** `run_lua`
(`lua_runner.rs:51-90`) spawns an OS thread with a nested `Runtime` and joins it,
because `mlua::Lua` is not `Send`. `extract_archive`
(`instance_setup.rs:67`) is synchronous inside an async fn and takes 8-10 minutes
on first run.

**Dropped without replacement:** the `instances_started`/`instances_stopped`
events emitted at `gui/command/instances.rs:51,80`. Nothing listens for them —
the UI derives state from local Pinia flags.

## Frontend

A typed `app/src/api/client.ts` replaces `invoke`, with the base URL from
`import.meta.env.VITE_API_BASE ?? ''` — same-origin in production, proxied in
dev so HMR still works. The five Pinia stores convert their ~20 call sites; the
long ones take a job id and subscribe with `EventSource`, appending into the
output state they already keep.

Deleted: `@tauri-apps/api`, `@tauri-apps/plugin-dialog`, `@tauri-apps/plugin-fs`,
`@tauri-apps/plugin-http`, `ynetwork`, and
`app/src/plugins/configure-ynetwork.js` (already effectively dead — it swaps in a
Tauri fetch runner that nothing routes through). `restapiStore.ts` and the
"Enable REST API" card at `SettingsPage.vue:206-223` collapse into a link to
`/swagger-ui`.

The native file pickers at `SettingsPage.vue:91,105` choose *server-side* paths,
which a browser cannot do. They become text inputs; validation already runs
server-side via `file_exists` (`SettingsPage.vue:114`).

The router stays on `createWebHashHistory` (`app/src/router.ts:62`), so no
server-side rewrite subtleties.

## Deletions

Verified dead, removed in the same change:

- `crates/restapi/src/map_tiles.rs` and `graph_tiles.rs` — not declared in
  `lib.rs:1-4`, so not modules of the crate at all. Orphaned actix-web files that
  would not compile if added.
- `crates/restapi/src/rapidoc.html` — referenced only from a comment.
- `restapi.rs:491-858` — ~370 lines of commented-out actix handlers.
- `webserver.rs:40` — a `.manage()`d `RestApiSettings` no handler reads.
- `app/src/assets/layout/flags/` — imported at `main.ts:14`, zero usage.

Also latent and fixed in passing: `gui/command/script.rs:202` returns
`Ok(String::new())` from a `-> Result<(), String>` function, so `gui` without
`lua` does not compile today.

## Build, CI and packaging

The Nix devShell loses `webkitgtk_4_1`, `gtk3`, `gtksourceview3`, `libsoup_3`,
`glib`, `cairo`, `pango`, `gdk-pixbuf`, `atk`, `librsvg` and `fuse`, leaving
`lua5_4`, `openssl` and `pkg-config`. The same libraries leave CI's apt install
line (`.github/workflows/test.yml:58`).

`publish.yml`'s Tauri bundling and auto-updater steps become plain `cargo build`s,
and the updater helper scripts under `app/scripts/` become dead. The desktop
auto-updater has no server equivalent; updating becomes "replace the binary".

Dependency delta: 28 packages leave (`rocket`, `rocket_codegen`, `rocket_http`,
`rocket_okapi`, `okapi`, `figment`, `devise`, `pear`, `multer` and their
transitive set); ~12-15 arrive, of which `axum 0.8.9`, `tower 0.5.3`,
`tower-http 0.6.11`, `hyper` and `http` are already in `Cargo.lock` via `tonic`
and `reqwest`. Net compile time should improve, since Rocket's proc-macro chain
is among the slowest in the ecosystem.

## Testing

axum handlers test through `tower::ServiceExt::oneshot` against the router with a
stubbed state — a real improvement over `#[tauri::command]`s, which could not be
tested without a Tauri runtime. Coverage to add:

- Each management endpoint: happy path and error path.
- Query-parameter compatibility for the ported game routes, specifically absent
  and empty optional parameters.
- One SSE test asserting a job streams lines and terminates.
- One test asserting a second concurrent script request gets `409` with the
  running job id.

The 22 existing Rust tests and the vitest suite are unaffected.

## Out of scope

- **The UI toolkit.** PrimeVue 5 is proprietary (free community tier, annual key,
  compiled packages); 4.5.5 is the last MIT release. PrimeFlex is archived, and
  all 75 PrimeFlex class occurrences in this app are already no-ops against the
  installed version. Worth deciding separately; it does not block this change.
- **Frontend major upgrades** (Vite 5→8, ESLint 8→10 flat config, Pinia 2→4,
  vue-router 4→5). Independent of the transport change.
- **Authentication and TLS.** Deliberately excluded; see above.
- **Map and graph tile rendering.** The old actix implementations are deleted
  rather than ported. If wanted, they are a fresh axum implementation against the
  still-public core APIs.
