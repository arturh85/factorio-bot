use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState::new(
        FactorioInstance::new_shared(),
        AppSettings::default().into_shared(),
        factorio_bot_core::paths::settings_file(),
    )
}

async fn post_rcon(body: &str) -> (StatusCode, serde_json::Value) {
    let response = build_router(test_state(), None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/rcon")
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
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    (status, json)
}

/// `code: 2` is `ErrorResponse::not_started`. A handler that rejected every
/// request unconditionally (e.g. `bad_request` as its first line) would also
/// answer 400 here, but with `code: 1` and a different message, so asserting
/// on the body -- not just the status -- is what tells this failure mode
/// apart from a malformed-body rejection below.
#[tokio::test]
async fn rcon_without_a_running_instance_reports_not_started() {
    let (status, body) = post_rcon(r#"{"command":"/help"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 2);
    assert_eq!(body["message"], "not started");
}

/// The body extractor rejects this before the handler ever looks at
/// `state.instance`, so `code` is `1` (`bad_request`), not `2`
/// (`not_started`) -- the opposite of the test above. A handler that just
/// rejects everything would produce `code: 1` here too, but by accident: the
/// message assertion pins it to the actual missing-field rejection, not a
/// hardcoded string that happens to match.
#[tokio::test]
async fn rcon_rejects_a_body_without_a_command() {
    let (status, body) = post_rcon("{}").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 1);
    let message = body["message"].as_str().expect("message is a string");
    assert!(
        message.contains("command"),
        "expected the rejection to mention the missing `command` field, got: {message}"
    );
}
