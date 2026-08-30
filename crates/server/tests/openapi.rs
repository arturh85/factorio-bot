use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState::new(
        Arc::new(RwLock::new(None)),
        AppSettings::default().into_shared(),
    )
}

async fn openapi_spec() -> serde_json::Value {
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
];
#[cfg(not(feature = "lua"))]
const OPERATIONS_WITH_A_PATH_PARAMETER: &[(&str, &str)] = &[];

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
        // Management
        "/api/v1/settings",
        "/api/v1/instance",
        "/api/v1/instance/stop",
        "/api/v1/rcon",
        "/api/v1/scripts",
        "/api/v1/scripts/file",
        "/api/v1/fs/exists",
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
        ("/api/v1/instance/stop", "post"),
        ("/api/v1/rcon", "post"),
        ("/api/v1/scripts", "get"),
        ("/api/v1/scripts/file", "get"),
        ("/api/v1/scripts/file", "put"),
        ("/api/v1/scripts/file", "post"),
        ("/api/v1/scripts/file", "delete"),
        ("/api/v1/fs/exists", "get"),
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
/// Renamed from `no_operation_publishes_a_path_parameter`, which asserted the
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
            let mut allowed_so_far = 0;
            for parameter in parameters {
                // An allow-listed operation is exempted for exactly the one
                // path parameter it is listed for -- `{id}` -- and for nothing
                // else. Exempting the whole *operation* instead would blind
                // this sweep on precisely the operation nobody would look at
                // again: add `GET /api/v1/jobs/{id}?since=...` through
                // `ApiQuery<T>` and plan 3's finding I1 resurfaces there,
                // republishing `since` as a path parameter, with this file
                // silently agreeing.
                if expected && parameter["in"] == "path" && parameter["name"] == "id" {
                    allowed_so_far += 1;
                    assert_eq!(
                        allowed_so_far,
                        1,
                        "{} {path} publishes more than one `id` path parameter",
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
