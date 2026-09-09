use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState::new(
        Arc::new(RwLock::new(None)),
        AppSettings::default().into_shared(),
        factorio_bot_core::paths::settings_file(),
    )
}

async fn openapi_spec() -> Value {
    let response = build_router(test_state(), None)
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
    serde_json::from_slice(&bytes).unwrap()
}

/// Every operation that takes query parameters, as
/// `(path, method, [(parameter name, required)])`.
///
/// `required` is asserted too: `#[serde(default)] Option<T>` fields are
/// genuinely optional, and publishing them as required makes a generated
/// client demand values the server does not want.
type QueryOperation = (&'static str, &'static str, &'static [(&'static str, bool)]);

const QUERY_OPERATIONS: &[QueryOperation] = &[
    (
        "/api/v1/game/find-entities",
        "get",
        &[
            ("area", false),
            ("position", false),
            ("radius", false),
            ("name", false),
            ("entity_type", false),
        ],
    ),
    (
        "/api/v1/game/find-tiles",
        "get",
        &[
            ("area", false),
            ("position", false),
            ("radius", false),
            ("name", false),
        ],
    ),
    (
        "/api/v1/game/inventory-contents-at",
        "get",
        &[("query", true)],
    ),
    ("/api/v1/game/player-info", "get", &[("player_id", true)]),
    (
        "/api/v1/game/plan-path",
        "get",
        &[
            ("entity_name", true),
            ("entity_type", true),
            ("underground_entity_name", true),
            ("underground_entity_type", true),
            ("underground_max", true),
            ("from_position", true),
            ("to_position", true),
            ("to_direction", true),
        ],
    ),
    ("/api/v1/scripts", "get", &[("path", true)]),
    ("/api/v1/scripts/file", "get", &[("path", true)]),
    ("/api/v1/scripts/file", "put", &[("path", true)]),
    ("/api/v1/scripts/file", "post", &[("path", true)]),
    ("/api/v1/scripts/file", "delete", &[("path", true)]),
    ("/api/v1/fs/exists", "get", &[("path", true)]),
];

/// Operations that exist only in a build that has an interpreter.
///
/// `factorio-bot-scripting-lua` is an optional dependency of this crate and
/// `cargo build --no-default-features` leaves it out, so `manage::router`
/// registers these behind the same `cfg`. `cargo test` over the workspace
/// turns the feature on through `app/src-tauri`, so the list is exercised
/// rather than skipped in practice.
#[cfg(feature = "lua")]
const SCRIPTING_OPERATIONS: &[(&str, &str)] = &[
    ("/api/v1/scripts/execute", "post"),
    ("/api/v1/jobs", "get"),
    ("/api/v1/jobs/{id}", "get"),
    ("/api/v1/jobs/{id}/events", "get"),
];
#[cfg(not(feature = "lua"))]
const SCRIPTING_OPERATIONS: &[(&str, &str)] = &[];

/// The operations that legitimately publish a templated path segment, as
/// `(method, path)`.
///
/// `no_operation_publishes_an_unexpected_path_parameter` below started life
/// asserting that *nothing* in this API was templated, with a message naming
/// that premise so whoever first added a templated route would have to make a
/// decision instead of patching around it. `GET /api/v1/jobs/{id}` is that
/// route. The decision is to keep the sweep and narrow it rather than relax
/// it: a templated path that is not listed here still fails, and an entry
/// listed here that does *not* actually publish a path parameter fails too, so
/// a stale entry cannot silently widen the guard.
#[cfg(feature = "lua")]
const OPERATIONS_WITH_A_PATH_PARAMETER: &[(&str, &str)] = &[
    ("get", "/api/v1/jobs/{id}"),
    ("get", "/api/v1/jobs/{id}/events"),
    ("get", "/api/v1/runs/{id}"),
    ("get", "/api/v1/runs/{id}/provenance"),
    ("get", "/api/v1/runs/{id}/events"),
    ("get", "/api/v1/runs/{id}/lanes"),
    ("get", "/api/v1/runs/{id}/samples"),
    ("get", "/api/v1/runs/{id}/map"),
    ("get", "/api/v1/runs/{id}/video"),
    ("get", "/api/v1/runs/{id}/video/ticks"),
    ("get", "/api/v1/runs/{id}/video/file"),
];
// The runs and video routes are registered unconditionally (they do not need
// an interpreter), so they are templated in both builds -- unlike the jobs
// routes above, which exist only behind `lua`.
#[cfg(not(feature = "lua"))]
const OPERATIONS_WITH_A_PATH_PARAMETER: &[(&str, &str)] = &[
    ("get", "/api/v1/runs/{id}"),
    ("get", "/api/v1/runs/{id}/provenance"),
    ("get", "/api/v1/runs/{id}/events"),
    ("get", "/api/v1/runs/{id}/lanes"),
    ("get", "/api/v1/runs/{id}/samples"),
    ("get", "/api/v1/runs/{id}/map"),
    ("get", "/api/v1/runs/{id}/video"),
    ("get", "/api/v1/runs/{id}/video/ticks"),
    ("get", "/api/v1/runs/{id}/video/file"),
];

/// The published response set for `POST /api/v1/instance/start` has to match
/// what the handler actually answers.
///
/// Every other guard in this file checks paths and methods, so the spec listed
/// only `202` and `409` for months after the handler grew a `400` (a relative
/// `workspace_path`) and a `500` (one that is not valid UTF-8), and nothing
/// noticed. A client generated from that spec has no case for either, and both
/// are answered *synchronously* -- they are the first thing a misconfigured
/// install sees.
#[tokio::test]
async fn the_start_operation_documents_every_status_it_answers() {
    let spec = openapi_spec().await;
    let responses = spec["paths"]["/api/v1/instance/start"]["post"]["responses"]
        .as_object()
        .expect("the start operation publishes a responses object");
    for status in ["202", "400", "409", "500"] {
        assert!(
            responses.contains_key(status),
            "POST /api/v1/instance/start answers {status} but does not document it; \
             a generated client has no case for it. documented: {:?}",
            responses.keys().collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn openapi_json_lists_every_route() {
    let spec = openapi_spec().await;
    let paths = spec["paths"].as_object().expect("paths object");

    for path in [
        // Game
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
        "/api/v1/runs",
        "/api/v1/runs/{id}",
        "/api/v1/runs/{id}/events",
        "/api/v1/runs/{id}/lanes",
        // Management
        "/api/v1/settings",
        "/api/v1/instance",
        "/api/v1/instance/start",
        "/api/v1/instance/stop",
        "/api/v1/rcon",
        "/api/v1/scripts",
        "/api/v1/scripts/file",
        "/api/v1/fs/exists",
        "/api/v1/video",
        "/api/v1/video/file",
        "/api/v1/video/ticks",
        "/api/v1/runs/{id}/video",
        "/api/v1/runs/{id}/video/file",
        "/api/v1/runs/{id}/video/ticks",
    ] {
        assert!(paths.contains_key(path), "spec is missing {path}");
    }

    // The management routes carry more than one method on a path, so listing
    // the path alone is not enough: a missing `PUT /api/v1/settings` would
    // still leave `/api/v1/settings` in the spec.
    for (path, method) in [
        ("/api/v1/settings", "get"),
        ("/api/v1/settings", "put"),
        ("/api/v1/instance", "get"),
        ("/api/v1/instance/start", "post"),
        ("/api/v1/instance/stop", "post"),
        ("/api/v1/rcon", "post"),
        ("/api/v1/scripts", "get"),
        ("/api/v1/scripts/file", "get"),
        ("/api/v1/scripts/file", "put"),
        ("/api/v1/scripts/file", "post"),
        ("/api/v1/scripts/file", "delete"),
        ("/api/v1/fs/exists", "get"),
        ("/api/v1/video", "get"),
        ("/api/v1/video/file", "get"),
        ("/api/v1/video/ticks", "get"),
    ] {
        assert!(
            paths[path].get(method).is_some(),
            "spec is missing {} {path}",
            method.to_uppercase()
        );
    }

    for (path, method) in SCRIPTING_OPERATIONS {
        assert!(paths.contains_key(*path), "spec is missing {path}");
        assert!(
            paths[*path].get(*method).is_some(),
            "spec is missing {} {path}",
            method.to_uppercase()
        );
    }
}

/// The load-bearing half of this file.
///
/// `utoipa-gen` infers `parameter_in` only from an argument literally typed
/// `Query<T>` or `Path<T>`. This crate's handlers take the local `ApiQuery<T>`
/// wrapper, which matches neither, so inference yields `None` and the derive
/// falls back to its default — `ParameterIn::Path`. Every query parameter was
/// therefore published as a *path* parameter (and, because OpenAPI requires
/// path parameters to be required, every `#[serde(default)]` optional was
/// published as required too). Swagger UI's "Try it out" built wrong URLs from
/// that, and so would any generated client.
///
/// The fix is `#[into_params(parameter_in = Query)]` on every `IntoParams`
/// struct reached through `ApiQuery`. This test is what stops it regressing
/// the next time one is added.
#[tokio::test]
async fn query_parameters_are_published_as_query_parameters() {
    let spec = openapi_spec().await;
    let paths = spec["paths"].as_object().expect("paths object");

    for (path, method, expected) in QUERY_OPERATIONS {
        let operation = paths[*path]
            .get(*method)
            .unwrap_or_else(|| panic!("spec is missing {} {path}", method.to_uppercase()));
        let parameters = operation["parameters"]
            .as_array()
            .unwrap_or_else(|| panic!("{} {path} publishes no parameters", method.to_uppercase()));

        for (name, required) in *expected {
            let parameter = parameters
                .iter()
                .find(|parameter| parameter["name"] == *name)
                .unwrap_or_else(|| {
                    panic!(
                        "{} {path} is missing parameter {name}",
                        method.to_uppercase()
                    )
                });
            assert_eq!(
                parameter["in"],
                "query",
                "{} {path} publishes {name} as {:?}, expected a query parameter",
                method.to_uppercase(),
                parameter["in"]
            );
            assert_eq!(
                parameter["required"],
                serde_json::Value::Bool(*required),
                "{} {path} publishes {name} with required={:?}, expected {required}",
                method.to_uppercase(),
                parameter["required"]
            );
        }

        assert_eq!(
            parameters.len(),
            expected.len(),
            "{} {path} publishes {} parameters, expected {}",
            method.to_uppercase(),
            parameters.len(),
            expected.len()
        );
    }
}

/// Sweep of the whole document rather than an enumerated list: apart from the
/// handful of operations in `OPERATIONS_WITH_A_PATH_PARAMETER`, no route in
/// this API takes a templated path segment, so *any* other `in: path`
/// parameter anywhere in the spec is the `ApiQuery` inference bug resurfacing
/// on a handler that `QUERY_OPERATIONS` above does not yet know about.
///
/// Renamed from `no_operation_publishes_a_path_parameter` (doclint-allow: the
/// retired name is the point of the sentence), which asserted the
/// stronger "none at all". That premise stopped holding when `GET
/// /api/v1/jobs/{id}` landed; the guard was narrowed to an allow-list rather
/// than relaxed, so it still fails on an *unexpected* path parameter.
#[tokio::test]
async fn no_operation_publishes_an_unexpected_path_parameter() {
    let spec = openapi_spec().await;
    let paths = spec["paths"].as_object().expect("paths object");

    for (path, methods) in paths {
        for (method, operation) in methods.as_object().expect("operations object") {
            let expected =
                OPERATIONS_WITH_A_PATH_PARAMETER.contains(&(method.as_str(), path.as_str()));
            assert!(
                !path.contains('{') || expected,
                "{} {path} is templated and is not listed in \
                 OPERATIONS_WITH_A_PATH_PARAMETER; publishing a path parameter is a \
                 decision this test exists to force, so list it there rather than \
                 relaxing the sweep",
                method.to_uppercase()
            );
            let Some(parameters) = operation.get("parameters").and_then(|p| p.as_array()) else {
                continue;
            };
            // EVERY segment name the path templates, e.g. `id` for
            // `/api/v1/jobs/{id}` -- derived from the path rather than
            // hardcoded, so a route with differently named segments does not
            // need this test rewritten to know about it.
            //
            // This read only the FIRST `{...}` until a route grew a second
            // templated segment. A path parameter that the path really does
            // template is not a finding, and rejecting it would have pushed
            // the fix towards exempting the operation -- which is exactly what
            // the comment below explains must never happen.
            let expected_names: Vec<&str> = path
                .split('{')
                .skip(1)
                .filter_map(|rest| rest.split('}').next())
                .collect();
            let mut allowed: std::collections::BTreeMap<&str, u32> =
                std::collections::BTreeMap::new();
            for parameter in parameters {
                // An allow-listed operation is exempted for exactly the one
                // path parameter it is listed for, and for nothing else.
                // Exempting the whole *operation* instead would blind this
                // sweep on precisely the operation nobody would look at
                // again: add `GET /api/v1/jobs/{id}?since=...` through
                // `ApiQuery<T>` and plan 3's finding I1 resurfaces there,
                // republishing `since` as a path parameter, with this file
                // silently agreeing.
                let published = parameter["name"].as_str().unwrap_or_default();
                if expected && parameter["in"] == "path" && expected_names.contains(&published) {
                    let seen = allowed.entry(published).or_insert(0);
                    *seen += 1;
                    assert_eq!(
                        *seen,
                        1,
                        "{} {path} publishes more than one {published:?} path parameter",
                        method.to_uppercase()
                    );
                    continue;
                }
                assert_ne!(
                    parameter["in"],
                    "path",
                    "{} {path} publishes {:?} as a path parameter",
                    method.to_uppercase(),
                    parameter["name"]
                );
            }
        }
    }
}

/// The other half of the allow-list above: an entry that does not correspond
/// to an operation which really does publish a path parameter is a stale
/// exemption, and a stale exemption silently widens the sweep for whatever
/// path happens to match it next.
#[tokio::test]
async fn every_allowed_path_parameter_is_really_published_as_one() {
    let spec = openapi_spec().await;
    let paths = spec["paths"].as_object().expect("paths object");

    for (method, path) in OPERATIONS_WITH_A_PATH_PARAMETER {
        let operation = paths
            .get(*path)
            .and_then(|methods| methods.get(*method))
            .unwrap_or_else(|| panic!("spec is missing {} {path}", method.to_uppercase()));
        let parameters = operation["parameters"]
            .as_array()
            .unwrap_or_else(|| panic!("{} {path} publishes no parameters", method.to_uppercase()));
        assert!(
            parameters.iter().any(|parameter| parameter["in"] == "path"),
            "{} {path} is exempted as carrying a path parameter but publishes none: {parameters:?}",
            method.to_uppercase()
        );
    }
}

/// `ErrorResponse::status` drives the HTTP status line; it is deliberately
/// `#[serde(skip)]`ped out of the body, so it must not appear in the schema a
/// client is generated from either.
#[tokio::test]
async fn the_error_schema_does_not_publish_the_internal_status_field() {
    let spec = openapi_spec().await;
    let schema = &spec["components"]["schemas"]["ErrorResponse"];
    let properties = schema["properties"]
        .as_object()
        .expect("ErrorResponse properties");
    assert!(properties.contains_key("message"), "{schema}");
    assert!(properties.contains_key("code"), "{schema}");
    assert!(
        !properties.contains_key("status"),
        "status must stay out of the wire format: {schema}"
    );
}

#[tokio::test]
async fn swagger_ui_is_served() {
    let response = build_router(test_state(), None)
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

// ---------------------------------------------------------------------------
// The committed snapshot the frontend is pinned to.
// ---------------------------------------------------------------------------

/// Where the browser client's copy of this document lives.
///
/// `app/src/api/client.ts` and `app/src/api/types.ts` are hand-written against
/// this API, and nothing in the TypeScript build can see `crates/server`. The
/// committed snapshot is the seam between the two halves, and it only works as
/// one if *both* ends are guarded:
///
/// * this test keeps the snapshot equal to what the server really publishes,
///   so it cannot go stale the way `app/src/models/types.ts` did; and
/// * `app/src/api/openapi.contract.spec.ts` checks the client's assumptions
///   against the snapshot.
///
/// Either half alone is worthless. A snapshot nobody regenerates is a second
/// `models/types.ts`; a frontend test with no upstream guard only ever proves
/// that the snapshot matches itself.
///
/// Note what the pair does and does not promise. Either test *can* be
/// satisfied on its own by editing the other's input -- hand-edit the snapshot
/// and this test is what goes red; regenerate it and the TypeScript test is.
/// The guarantee is the conjunction: **the two cannot both pass unless the
/// server, the snapshot and the client all say the same thing.**
const SNAPSHOT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/src/api/openapi.snapshot.json"
);

/// Printed on every failure below, because "the snapshot is out of date" is
/// only half the instruction -- the other half is that a moved field is a
/// *frontend* change, not a file to silently re-bless.
const REGENERATE_HINT: &str = "\n\nRegenerating is a deliberate act with a visible diff:\n    \
     UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p factorio-bot-server --features lua --test openapi\n    \
     git diff --no-ext-diff app/src/api/openapi.snapshot.json\n\
     Whatever moved has to be mirrored in app/src/api/types.ts, app/src/api/client.ts\n\
     and app/src/api/openapi.contract.spec.ts -- those are what fail next if it is not,\n\
     and the browser is what fails if neither does.";

/// Whether this run was asked to rewrite the committed snapshot instead of
/// checking it -- and a hard stop if that request arrives from CI.
///
/// `UPDATE_OPENAPI_SNAPSHOT=1` turns both guards below off, which is fine at a
/// developer's terminal where the point is to see the diff before committing
/// it. In an automated run it would be a disaster of the quiet kind: the job
/// would rewrite a tracked file, report `ok`, and agree with whatever the
/// server had just started publishing. No job in this repo sets it today; this
/// makes sure that stays a fact rather than a hope.
///
/// It panics rather than ignoring the variable and checking anyway, because a
/// CI job that sets it is misconfigured, and a misconfiguration that silently
/// does the right thing is one nobody fixes.
fn snapshot_update_requested() -> bool {
    let requested = std::env::var_os("UPDATE_OPENAPI_SNAPSHOT").is_some();
    assert!(
        !(requested && std::env::var_os("CI").is_some()),
        "UPDATE_OPENAPI_SNAPSHOT is set in a CI environment.\n\
         Regenerating the committed OpenAPI snapshot is a deliberate local act with a\n\
         reviewed diff; doing it automatically would re-bless every server change and\n\
         report a green run. Unset UPDATE_OPENAPI_SNAPSHOT in the CI environment, and\n\
         regenerate locally instead:{REGENERATE_HINT}"
    );
    requested
}

fn read_snapshot() -> Value {
    let raw = std::fs::read_to_string(SNAPSHOT_PATH).unwrap_or_else(|err| {
        panic!("cannot read the committed snapshot at {SNAPSHOT_PATH}: {err}{REGENERATE_HINT}")
    });
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("{SNAPSHOT_PATH} is not valid JSON: {err}{REGENERATE_HINT}"))
}

/// Every `(path, method)` in a spec document, as flat strings, so two
/// documents can be compared operation by operation instead of as one opaque
/// blob.
fn operations(spec: &Value) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let Some(paths) = spec.get("paths").and_then(|paths| paths.as_object()) else {
        return found;
    };
    for (path, methods) in paths {
        let Some(methods) = methods.as_object() else {
            continue;
        };
        for method in methods.keys() {
            found.push((path.clone(), method.clone()));
        }
    }
    found
}

fn operation<'a>(spec: &'a Value, path: &str, method: &str) -> Option<&'a Value> {
    spec.get("paths")?.get(path)?.get(method)
}

/// A human-readable account of how two spec documents differ, listing the
/// operations and schemas that moved rather than dumping two ~70 KB documents
/// into the failure output.
///
/// Only the `lua` build compares whole documents; the subset guard below
/// reports its own differences one operation at a time, so this would be dead
/// code (and `-D warnings` an error) without the `cfg`.
#[cfg(feature = "lua")]
fn describe_differences(snapshot: &Value, published: &Value) -> String {
    let mut lines = Vec::new();

    let snapshot_ops = operations(snapshot);
    let published_ops = operations(published);
    for op @ (path, method) in &published_ops {
        if !snapshot_ops.contains(op) {
            lines.push(format!(
                "  + {} {path} is published but missing from the snapshot",
                method.to_uppercase()
            ));
        } else if operation(snapshot, path, method) != operation(published, path, method) {
            lines.push(format!(
                "  ~ {} {path} differs (parameters, request body or responses moved)",
                method.to_uppercase()
            ));
        }
    }
    for (path, method) in &snapshot_ops {
        if !published_ops.contains(&(path.clone(), method.clone())) {
            lines.push(format!(
                "  - {} {path} is in the snapshot but no longer published",
                method.to_uppercase()
            ));
        }
    }

    let empty = serde_json::Map::new();
    let snapshot_schemas = snapshot["components"]["schemas"]
        .as_object()
        .unwrap_or(&empty);
    let published_schemas = published["components"]["schemas"]
        .as_object()
        .unwrap_or(&empty);
    for (name, schema) in published_schemas {
        match snapshot_schemas.get(name) {
            None => lines.push(format!(
                "  + schema {name} is published but not in the snapshot"
            )),
            Some(committed) if committed != schema => lines.push(format!(
                "  ~ schema {name} differs\n      snapshot:  {committed}\n      published: {schema}"
            )),
            Some(_) => {}
        }
    }
    for name in snapshot_schemas.keys() {
        if !published_schemas.contains_key(name) {
            lines.push(format!(
                "  - schema {name} is in the snapshot but no longer published"
            ));
        }
    }

    if lines.is_empty() {
        // Something outside `paths` and `components.schemas` moved: `info`,
        // `tags`, a security scheme. Rare enough not to itemise, and a bare
        // "they differ" with no detail would be useless, so fall back to both
        // documents.
        return format!(
            "  the documents differ outside paths and component schemas\n    snapshot:  {snapshot}\n    published: {published}"
        );
    }
    lines.join("\n")
}

/// The guard that makes the snapshot worth having: rename a field in a
/// `crates/server` DTO, drop an operation from `manage::router`, or add a
/// required request field, and this fails by name here -- in the same
/// workspace `cargo test` run that made the change compile.
///
/// Gated on `lua` because `manage::router` registers `/api/v1/scripts/execute`
/// and the three job routes behind the same feature, so a `--no-default-
/// features` build genuinely publishes a smaller document. `cargo test
/// --workspace` (what `just test` and `precommit:check` run) turns the feature
/// on through `app/src-tauri`'s defaults, so this is the variant that actually
/// runs. `the_snapshot_covers_every_operation_a_build_without_an_interpreter_publishes`
/// below is the weaker guard that survives without it, so the no-default-
/// features build is not silently unguarded.
#[cfg(feature = "lua")]
#[tokio::test]
async fn the_committed_openapi_snapshot_matches_the_published_spec() {
    let published = openapi_spec().await;

    // Opt-in regeneration, never automatic: a snapshot a build rewrites on its
    // own is a snapshot that agrees with every change, including the ones that
    // break the browser. `snapshot_update_requested` refuses the opt-in under
    // `CI` rather than honouring it silently.
    if snapshot_update_requested() {
        let mut rendered =
            serde_json::to_string_pretty(&published).expect("the spec serialises back to JSON");
        rendered.push('\n');
        std::fs::write(SNAPSHOT_PATH, rendered)
            .unwrap_or_else(|err| panic!("cannot write {SNAPSHOT_PATH}: {err}"));
        eprintln!("wrote {SNAPSHOT_PATH}; review the diff before committing it");
        return;
    }

    let snapshot = read_snapshot();
    assert!(
        snapshot == published,
        "the committed OpenAPI snapshot no longer matches what this server publishes:\n{}{REGENERATE_HINT}",
        describe_differences(&snapshot, &published)
    );
}

/// The half of the guard above that survives `--no-default-features`.
///
/// Such a build publishes strictly fewer operations (no interpreter, so no
/// `/api/v1/scripts/execute` and no jobs), which is why it cannot assert
/// equality. Everything it *does* publish must still be byte-for-byte what
/// the snapshot promises the frontend -- so a renamed field on, say,
/// `InstanceStatus` fails here too rather than only in the `lua` build.
#[tokio::test]
async fn the_snapshot_covers_every_operation_a_build_without_an_interpreter_publishes() {
    // The regenerating run is writing the very file this reads, from a sibling
    // test thread. Reading it here would race the write and fail on a
    // half-written document rather than on anything real. Skipping is only
    // acceptable because `snapshot_update_requested` has already ruled out the
    // case where the opt-in was not deliberate.
    if snapshot_update_requested() {
        return;
    }
    let published = openapi_spec().await;
    let snapshot = read_snapshot();

    for (path, method) in operations(&published) {
        let committed = operation(&snapshot, &path, &method).unwrap_or_else(|| {
            panic!(
                "{} {path} is published but missing from the committed snapshot{REGENERATE_HINT}",
                method.to_uppercase()
            )
        });
        assert_eq!(
            committed,
            operation(&published, &path, &method).expect("just enumerated"),
            "{} {path} differs from the committed snapshot{REGENERATE_HINT}",
            method.to_uppercase()
        );
    }

    let empty = serde_json::Map::new();
    let snapshot_schemas = snapshot["components"]["schemas"]
        .as_object()
        .unwrap_or(&empty);
    for (name, schema) in published["components"]["schemas"]
        .as_object()
        .unwrap_or(&empty)
    {
        let committed = snapshot_schemas.get(name).unwrap_or_else(|| {
            panic!("schema {name} is published but missing from the committed snapshot{REGENERATE_HINT}")
        });
        assert_eq!(
            committed, schema,
            "schema {name} differs from the committed snapshot{REGENERATE_HINT}"
        );
    }
}
