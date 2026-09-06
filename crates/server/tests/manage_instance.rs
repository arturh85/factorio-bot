use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::parking_lot;
use factorio_bot_core::process::process_control::{FactorioInstance, SharedFactorioInstance};
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn state_with(instance: SharedFactorioInstance) -> AppState {
    AppState::new(
        instance,
        AppSettings::default().into_shared(),
        factorio_bot_core::paths::settings_file(),
    )
}

/// A `FactorioInstance` that owns no real child processes: no server or
/// client `InteractiveProcess`, so `FactorioInstance::stop` has nothing to
/// kill and cannot hang or fail. Copied from
/// `crates/server/tests/shutdown.rs::empty_factorio_instance` rather than
/// inventing a second version.
fn empty_factorio_instance() -> FactorioInstance {
    FactorioInstance {
        world: Some(Arc::new(FactorioSurface::new())),
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

/// `code: 2` is asserted alongside the status, and it is the half the client
/// actually reads.
///
/// "No Factorio instance is running" reaches the browser as `400` here and as
/// `503` from `POST /api/v1/scripts/execute`, so the stores identify the
/// condition by `code`, never by the status. Pinning only the status would let
/// `ErrorResponse::not_started` be renumbered with the whole Rust suite green
/// and the OpenAPI snapshot unmoved -- the number is a value, not part of the
/// schema -- while every client branch that recognises the condition silently
/// stops recognising it. The status assertion still earns its place: it is
/// what distinguishes this from the `503` leg.
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"], 2);
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

/// POSTs a start and waits for the detached task to finish, returning whatever
/// it left in `last_error`.
///
/// Every assertion about a start is made on this, never on the `202`: the
/// response is written before the spawned task has done anything at all, so it
/// cannot distinguish a start that worked from one that died immediately.
async fn start_and_settle(state: &AppState) -> Option<String> {
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
    state.last_start_error.read().await.clone()
}

/// `workspace_path` defaults to `""`, and stays `""` in every settings value
/// that did not come through `load_app_settings` -- most reachably a
/// `PUT /api/v1/settings` whose body leaves the field blank, which is what the
/// settings screen sends when the user clears it.
///
/// The scripts routes resolve that (empty means the data-local workspace, see
/// `manage::scripts::scripts_root_path`); the start route must resolve it the
/// same way rather than handing the raw string to `setup_factorio_instance`,
/// which rejects it out of hand. Two copies of one rule is the defect --
/// `GET /api/v1/scripts` working while `POST /api/v1/instance/start` fails on
/// the same configured value is the symptom.
///
/// The start still fails here: no Factorio archive is configured either. That
/// is the point -- the failure has to have moved *past* the workspace.
#[tokio::test]
async fn a_start_resolves_an_unconfigured_workspace_like_every_other_route() {
    let state = state_with(FactorioInstance::new_shared());
    assert!(
        state
            .settings
            .read()
            .await
            .factorio
            .workspace_path
            .is_empty(),
        "this test is about the empty default; the fixture no longer provides it"
    );

    let last_error = start_and_settle(&state).await.expect("the start failed");

    assert!(
        !last_error.contains("no workspace configured"),
        "the start rejected the same empty workspace_path the scripts routes \
         resolve happily: {last_error}"
    );
    assert!(
        !last_error.contains("failed to find workspace"),
        "the start looked for a workspace it never resolved: {last_error}"
    );
    assert!(
        last_error.contains("archive"),
        "expected the failure to have moved on to the unconfigured archive, got: {last_error}"
    );
}

/// The Stop button, pressed while Factorio is still coming up, must win.
///
/// A start is accepted, the archive extraction runs for minutes, the user gives
/// up and presses Stop — and then the start finally succeeds. Publishing at
/// that point hands the user a running game they explicitly cancelled, and one
/// `GET /api/v1/instance` has no way to explain.
///
/// The stop goes through the real route, so a `stop_instance` that stopped
/// recording the request would fail this test rather than pass it on a counter
/// the test bumped itself. The start is simulated: `FactorioInstance::start`
/// needs an installed game and eight minutes, so `complete_start` — the same
/// function the route spawns — is driven with an instance that owns no child
/// processes.
#[tokio::test]
async fn a_stop_during_a_start_is_not_undone_by_the_start_finishing() {
    let state = state_with(FactorioInstance::new_shared());
    assert!(state.claim_start_slot(), "the slot starts free");
    // What `start_instance` captures before handing off to the start.
    let stop_requests_before = state.stop_requests();

    // The user presses Stop. Nothing is published yet — the start is still
    // extracting — so the route answers 400; the cancellation is the part that
    // matters here.
    let response = build_router(state.clone(), None)
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

    // ... and only now does Factorio finish coming up.
    factorio_bot_server::manage::instance::complete_start(state.clone(), stop_requests_before, {
        async { Ok(empty_factorio_instance()) }
    })
    .await;

    assert!(
        state.instance.read().await.is_none(),
        "the start published an instance after the user stopped it"
    );
    assert!(
        !state.starting.load(std::sync::atomic::Ordering::SeqCst),
        "the starting slot was never released"
    );
    assert_eq!(
        *state.last_start_error.read().await,
        None,
        "a start the user cancelled is not an error to report"
    );

    let response = build_router(state.clone(), None)
        .oneshot(
            Request::builder()
                .uri("/api/v1/instance")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status["started"], false, "{status}");
    assert_eq!(status["starting"], false, "{status}");
}

/// The other half of the test above: without a stop, the start *must* publish.
///
/// Without this, "never publish anything" passes the whole suite — and the
/// route would answer 202 forever while `GET /api/v1/instance` reported
/// nothing running.
#[tokio::test]
async fn a_start_that_finishes_without_a_stop_publishes_its_instance() {
    let state = state_with(FactorioInstance::new_shared());
    assert!(state.claim_start_slot(), "the slot starts free");
    let stop_requests_before = state.stop_requests();

    factorio_bot_server::manage::instance::complete_start(state.clone(), stop_requests_before, {
        async { Ok(empty_factorio_instance()) }
    })
    .await;

    assert!(
        state.instance.read().await.is_some(),
        "an uninterrupted start published nothing"
    );
    assert!(
        !state.starting.load(std::sync::atomic::Ordering::SeqCst),
        "the starting slot was never released"
    );
}
