use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use tower::ServiceExt;

// `AppState::new(...)` rather than a struct literal: `AppState` also carries
// `settings_path` (added in an earlier task), which the brief's snippet
// predates. None of these tests touch persistence, so the production
// constructor (which points `settings_path` at the real on-disk settings
// file) is fine here, matching the convention every other read-only
// `manage_*` test file in this crate already uses.
fn test_state() -> AppState {
    AppState::new(
        FactorioInstance::new_shared(),
        AppSettings::default().into_shared(),
    )
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
