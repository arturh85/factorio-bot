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

#[tokio::test]
async fn starting_when_an_instance_already_runs_is_a_conflict() {
    let instance: SharedFactorioInstance = Arc::new(RwLock::new(Some(empty_factorio_instance())));
    let response = build_router(state_with(instance), None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/instance/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_second_start_while_one_is_in_flight_is_a_conflict() {
    // Occupy the starting slot directly rather than racing two real starts:
    // the contract under test is the refusal, not the scheduler's timing.
    // What makes the refusal safe under real concurrency is the
    // `compare_exchange` in `start_instance`, which this test cannot observe;
    // see the comment there.
    let state = state_with(FactorioInstance::new_shared());
    state
        .starting
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let response = build_router(state, None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/instance/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn instance_status_reports_the_in_flight_start() {
    // The UI polls this flag instead of holding a request open for the 8-10
    // minutes a first-run archive extraction takes.
    let state = state_with(FactorioInstance::new_shared());
    state
        .starting
        .store(true, std::sync::atomic::Ordering::SeqCst);
    *state.last_start_error.write().await = Some("previous attempt exploded".into());

    let response = build_router(state, None)
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
    assert_eq!(status["starting"], true);
    assert_eq!(status["last_error"], "previous attempt exploded");
}

/// A start that fails must leave the slot free *and* a readable reason. The
/// handler writes `last_error` before clearing `starting`, so a poller that
/// sees `starting == false` always sees the error too, rather than concluding
/// from an empty `last_error` that the start succeeded.
///
/// Driven through the real handler with a settings object that cannot
/// possibly start Factorio (no workspace configured), so the failure comes
/// from `FactorioInstance::start` itself rather than from a stubbed error.
#[tokio::test]
async fn a_failed_start_releases_the_slot_and_records_why() {
    let state = state_with(FactorioInstance::new_shared());
    // `workspace_path` is empty in the defaults, which `setup_factorio_instance`
    // rejects immediately -- no archive is touched and no process is spawned.
    let response = build_router(state.clone(), None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/instance/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // The spawned task is the thing under test, so wait for it rather than
    // asserting on a slot it has not reached yet.
    for _ in 0..200 {
        if !state.starting.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        !state.starting.load(std::sync::atomic::Ordering::SeqCst),
        "the starting slot was never released, so every later start would 409"
    );
    let last_error = state.last_start_error.read().await.clone();
    assert!(
        last_error.is_some(),
        "the failure left no last_error behind, so a poller seeing starting == false \
         would conclude the start succeeded"
    );
    assert!(
        state.instance.read().await.is_none(),
        "a failed start must not publish an instance"
    );
}
