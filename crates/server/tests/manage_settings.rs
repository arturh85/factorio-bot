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

    assert!(response.status().is_success(), "got {}", response.status());
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
    assert!(
        content_type.starts_with("application/json"),
        "{content_type}"
    );
}
