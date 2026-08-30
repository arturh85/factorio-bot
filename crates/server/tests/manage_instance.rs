use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::parking_lot;
use factorio_bot_core::process::process_control::{FactorioInstance, SharedFactorioInstance};
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn state_with(instance: SharedFactorioInstance) -> AppState {
    AppState::new(instance, AppSettings::default().into_shared())
}

/// A `FactorioInstance` that owns no real child processes: no server or
/// client `InteractiveProcess`, so `FactorioInstance::stop` has nothing to
/// kill and cannot hang or fail. Copied from
/// `crates/server/tests/shutdown.rs::empty_factorio_instance` rather than
/// inventing a second version.
fn empty_factorio_instance() -> FactorioInstance {
    FactorioInstance {
        world: Some(Arc::new(FactorioWorld::new())),
        rcon: Arc::new(FactorioRcon::new_empty()),
        server_process: None,
        client_processes: Vec::new(),
        silent: Arc::new(parking_lot::RwLock::new(true)),
        server_host: None,
        server_port: None,
        rcon_port: 0,
        client_count: 0,
        map_exchange_string: None,
        seed: None,
    }
}

#[tokio::test]
async fn instance_status_reports_stopped_when_nothing_runs() {
    let response = build_router(state_with(FactorioInstance::new_shared()), None)
        .oneshot(
            Request::builder()
                .uri("/api/v1/instance")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status["started"], false);
}

#[tokio::test]
async fn stopping_when_nothing_runs_is_an_error_not_a_panic() {
    let response = build_router(state_with(FactorioInstance::new_shared()), None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/instance/stop")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn stopping_takes_the_instance_out_of_shared_state() {
    // an instance with no child processes: stop() on it is a no-op that succeeds
    let instance: SharedFactorioInstance = Arc::new(RwLock::new(Some(empty_factorio_instance())));

    let response = build_router(state_with(instance.clone()), None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/instance/stop")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success(), "got {}", response.status());
    assert!(instance.read().await.is_none(), "instance was not taken");
}
