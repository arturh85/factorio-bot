use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::settings::RestApiSettings;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn state_with_web_root(dir: &std::path::Path) -> AppState {
    AppState::new(
        Arc::new(RwLock::new(None)),
        AppSettings {
            restapi: RestApiSettings {
                port: 7492,
                web_root: Some(dir.to_string_lossy().into_owned()),
            },
            ..Default::default()
        }
        .into_shared(),
    )
}

#[tokio::test]
async fn serves_index_html_at_root() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>spa</html>").unwrap();

    let root = dir.path().to_string_lossy().into_owned();
    let response = build_router(state_with_web_root(dir.path()), Some(&root))
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(bytes.to_vec()).unwrap(),
        "<html>spa</html>"
    );
}

#[tokio::test]
async fn unknown_path_falls_back_to_index_html() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>spa</html>").unwrap();

    let root = dir.path().to_string_lossy().into_owned();
    let response = build_router(state_with_web_root(dir.path()), Some(&root))
        .oneshot(
            Request::builder()
                .uri("/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(bytes.to_vec()).unwrap(),
        "<html>spa</html>"
    );
}

#[tokio::test]
async fn api_404_is_not_swallowed_by_the_spa_fallback() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>spa</html>").unwrap();

    let root = dir.path().to_string_lossy().into_owned();
    let response = build_router(state_with_web_root(dir.path()), Some(&root))
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

fn state_without_web_root() -> AppState {
    AppState::new(
        Arc::new(RwLock::new(None)),
        AppSettings::default().into_shared(),
    )
}

/// The JSON error contract must not depend on whether a frontend is deployed.
#[tokio::test]
async fn api_404_is_json_without_a_web_root() {
    let response = build_router(state_without_web_root(), None)
        .oneshot(
            Request::builder()
                .uri("/api/v1/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"], 404);
    assert_eq!(body["message"], "not found");
}

/// SettingsPage.vue links to `http://localhost:<port>`; without a frontend that
/// root has nothing to serve, so it points at the API docs instead.
#[tokio::test]
async fn root_redirects_to_swagger_ui_without_a_web_root() {
    let response = build_router(state_without_web_root(), None)
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/swagger-ui/")
    );
}

#[tokio::test]
async fn server_starts_without_a_web_root() {
    let response = build_router(state_without_web_root(), None)
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
