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

/// Sweep of the whole document rather than an enumerated list: no route in
/// this API takes a templated path segment, so *any* `in: path` parameter
/// anywhere in the spec is the `ApiQuery` inference bug resurfacing on a
/// handler that `QUERY_OPERATIONS` above does not yet know about.
#[tokio::test]
async fn no_operation_publishes_a_path_parameter() {
    let spec = openapi_spec().await;
    let paths = spec["paths"].as_object().expect("paths object");

    for (path, methods) in paths {
        assert!(
            !path.contains('{'),
            "{path} is templated; this test's premise no longer holds"
        );
        for (method, operation) in methods.as_object().expect("operations object") {
            let Some(parameters) = operation.get("parameters").and_then(|p| p.as_array()) else {
                continue;
            };
            for parameter in parameters {
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
