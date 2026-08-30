use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use tower::ServiceExt;

/// `settings_path` points inside a fresh, per-call temp directory rather than
/// `AppState::new`'s real `paths::settings_file()`: a `PUT` that reaches
/// `AppSettings::save` must never write to a developer's actual settings
/// file just because the test suite ran.
///
/// The `TempDir` is returned rather than `keep()`-ed: `keep()` disarms the
/// deletion guard, so every run of this file left a directory behind in the
/// system temp directory forever. Callers bind it for the length of the test
/// and it is removed on drop.
fn test_state() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState {
        instance: FactorioInstance::new_shared(),
        settings: AppSettings::default().into_shared(),
        settings_path: dir.path().join("AppSettings.toml"),
        starting: Default::default(),
        last_start_error: Default::default(),
        stop_generation: Default::default(),
        jobs: factorio_bot_server::jobs::JobRegistry::new(8),
    };
    (dir, state)
}

#[tokio::test]
async fn get_settings_returns_the_current_settings() {
    let (_dir, state) = test_state();
    let response = build_router(state, None)
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

/// This is also the test that proves persistence, not just the in-memory
/// mutation: `settings_path` points inside a `tempfile::TempDir` that this
/// test owns, and it reads that exact file back off disk afterwards.
#[tokio::test]
async fn put_settings_updates_the_shared_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let settings_path = dir.path().join("AppSettings.toml");
    let state = AppState {
        instance: FactorioInstance::new_shared(),
        settings: AppSettings::default().into_shared(),
        settings_path: settings_path.clone(),
        starting: Default::default(),
        last_start_error: Default::default(),
        stop_generation: Default::default(),
        jobs: factorio_bot_server::jobs::JobRegistry::new(8),
    };
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

    let persisted = AppSettings::load(settings_path).expect("settings file was written");
    assert_eq!(
        persisted.factorio.client_count, 4,
        "PUT must persist to settings_path, not just mutate memory"
    );
}

#[tokio::test]
async fn put_settings_rejects_a_malformed_body_with_json() {
    let (_dir, state) = test_state();
    let response = build_router(state, None)
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
