//! `POST /api/v1/scripts/execute` and the job-inspection endpoints.
//!
//! The whole module is gated: the execute routes only exist in a build that
//! has an interpreter, and `factorio-bot-scripting-lua` is an optional
//! dependency of this crate.
//!
//! **`cargo test -p factorio-bot-server` alone does not run any of this.** The
//! crate declares no default features, so the `cfg` below empties the file and
//! the run reports `0 passed` and exits `0` -- a silent green, which is exactly
//! what you do not want while iterating on it. Use
//! `cargo test -p factorio-bot-server --features lua`. A whole-workspace
//! `cargo test` does turn the feature on, through `app/src-tauri`'s
//! `lua = [..., "factorio-bot-server?/lua"]`, so CI and the precommit gate see
//! these tests.
#![cfg(feature = "lua")]

use axum::body::Body;
use axum::http::{header, Request, Response, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::parking_lot;
use factorio_bot_core::process::process_control::FactorioInstance;
// `OutputSink` is what puts `JobHandle::line` in scope: the event-stream tests
// drive a job's output directly rather than through a real script, so that
// what they assert about the wire format does not depend on the interpreter.
use factorio_bot_scripting::{OutputSink, Stream};
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tower::ServiceExt;

/// A `FactorioInstance` that owns no real child processes, so nothing here
/// spawns or kills a process. Copied from `tests/manage_instance.rs` rather
/// than inventing a third version.
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

/// A workspace whose `scripts/` directory holds `hello.lua`, with a second
/// real script as a *sibling* of that directory.
///
/// The sibling is what makes the traversal test mean something: `../outside.lua`
/// resolves to a file that actually exists, so `canonicalize` succeeds and the
/// bounds check is what refuses it. A path that simply does not exist (the
/// obvious `../../etc/passwd` from inside a temp directory) would be answered
/// `404` by the *missing-file* arm, and the test would pass while asserting
/// nothing about the traversal guard.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("scripts")).expect("mkdir");
    std::fs::write(
        dir.path().join("scripts").join("hello.lua"),
        "print(\"hello from the script\")",
    )
    .expect("write");
    std::fs::write(dir.path().join("outside.lua"), "print(\"outside\")").expect("write");
    dir
}

fn state_for(dir: &tempfile::TempDir, instance: Option<FactorioInstance>) -> AppState {
    let mut settings = AppSettings::default();
    settings.factorio.workspace_path = dir.path().to_string_lossy().into_owned().into();
    AppState {
        instance: Arc::new(RwLock::new(instance)),
        settings: settings.into_shared(),
        settings_path: dir.path().join("AppSettings.toml"),
        starting: Default::default(),
        last_start_error: Default::default(),
        stop_generation: Default::default(),
        jobs: factorio_bot_server::jobs::JobRegistry::new(8),
    }
}

/// No Factorio instance. Everything that must be answered *before* the
/// instance is looked at uses this, so a check that quietly moved below the
/// instance check would start answering `503` here and fail.
fn test_state() -> (tempfile::TempDir, AppState) {
    let dir = workspace();
    let state = state_for(&dir, None);
    (dir, state)
}

/// An instance is running, so requests get past the `503` and reach the
/// script resolution and the execution slot.
fn running_state() -> (tempfile::TempDir, AppState) {
    let dir = workspace();
    let state = state_for(&dir, Some(empty_factorio_instance()));
    (dir, state)
}

async fn post_execute(state: &AppState, body: serde_json::Value) -> Response<Body> {
    build_router(state.clone(), None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/scripts/execute")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn get(state: &AppState, uri: &str) -> Response<Body> {
    build_router(state.clone(), None)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Drains a response body to a string.
///
/// For an SSE response this only returns once the stream *ends*, which is why
/// every caller below wraps it in a `tokio::time::timeout`: a stream that
/// stayed open would otherwise hang the suite, and a hang is not a test
/// result -- CI reports it as an unattributed timeout and a developer running
/// the suite locally cannot tell it from a deadlock somewhere else.
async fn collect_body(response: Response<Body>) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn body_json(response: Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|err| {
        panic!(
            "body was not JSON ({err}): {}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

/// The honest status for "the server is fine, the game is not running" -- and
/// the 503-before-404 leg of the ordering contract.
///
/// The script deliberately does *not* exist. Asking for one that does cannot
/// discriminate: both orderings answer `503`, so hoisting the script
/// resolution above the instance check would fail nothing. With a missing
/// script the mutation answers `404` instead, which is the leak it describes --
/// the endpoint telling a caller what is and is not in the scripts root before
/// it will admit the game is not running.
#[tokio::test]
async fn executing_without_a_running_instance_is_service_unavailable() {
    let (_dir, state) = test_state();
    let response = post_execute(&state, serde_json::json!({ "path": "/nope.lua" })).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// Answered without an instance, which is also the ordering assertion: body
/// validation runs before the instance check, so this is `400` and not `503`.
#[tokio::test]
async fn executing_with_neither_path_nor_code_is_a_bad_request() {
    let (_dir, state) = test_state();
    let response = post_execute(&state, serde_json::json!({})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn executing_with_both_path_and_code_is_a_bad_request() {
    let (_dir, state) = test_state();
    let response = post_execute(
        &state,
        serde_json::json!({ "path": "/a.lua", "code": "print(1)" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// A script the caller could have typed wrong is a `404`.
///
/// Asserted as a status, never as a message: the point of the typed
/// `RunScriptError` is that this split survives someone rewording the error
/// text, and a test that matched on the text would pass just as happily
/// against the flattened-to-a-string version this replaced.
#[tokio::test]
async fn executing_a_missing_script_is_not_found() {
    let (_dir, state) = running_state();
    let response = post_execute(&state, serde_json::json!({ "path": "/nope.lua" })).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// A script that exists but lives outside the scripts root is a refused
/// request, not a missing resource -- and the response deliberately does not
/// say what is on the other side.
#[tokio::test]
async fn executing_a_script_outside_the_scripts_root_is_a_bad_request() {
    let (dir, state) = running_state();
    let response = post_execute(&state, serde_json::json!({ "path": "../outside.lua" })).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    let message = body["message"].as_str().expect("message is a string");
    // The workspace's absolute path, not the requested name: the request is
    // echoed back deliberately (the caller sent it), so a test that only
    // forbade `outside.lua` would forbid nothing the caller does not already
    // know. What must not leak is where the server keeps its files -- a
    // message like `(resolved to /srv/workspace/outside.lua)` passes a
    // filename check and is a real disclosure.
    assert!(
        !message.contains(dir.path().to_string_lossy().as_ref()),
        "the refusal must not disclose the server's filesystem layout: {message}"
    );
}

#[tokio::test]
async fn a_second_execution_is_refused_with_the_running_job_id() {
    // Occupy the slot directly rather than racing two real scripts: the
    // contract under test is the 409 body, not the scheduler's timing.
    let (_dir, state) = running_state();
    let holder = state
        .jobs
        .try_start(Some("busy.lua".into()))
        .expect("slot taken");
    let response = post_execute(&state, serde_json::json!({ "path": "/hello.lua" })).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert_eq!(
        body["running_job_id"],
        serde_json::json!(holder.id().to_string())
    );
}

/// The ordering contract, stated as the damage that breaking it does.
///
/// Hoisting `try_start` above the script resolution still answers `404` --
/// `JobHandle`'s `Drop` frees the slot on the way out -- so a status-only test
/// cannot see it. What it does do is fabricate a failed job for a script that
/// never ran, and it makes the slot's release depend on a `Drop` running,
/// which is precisely the thing not to depend on: the slot is single, and one
/// exit path that leaks it makes every later execution answer `409` for the
/// lifetime of the process.
#[tokio::test]
async fn a_refused_execution_never_takes_the_slot() {
    let (_dir, state) = running_state();
    let response = post_execute(&state, serde_json::json!({ "path": "/nope.lua" })).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    assert!(
        state.jobs.list().is_empty(),
        "a request that never ran a script must not leave a job behind: {:?}",
        state.jobs.list()
    );
    assert!(
        state.jobs.running().is_none(),
        "the slot must still be free"
    );
    state
        .jobs
        .try_start(Some("after.lua".into()))
        .expect("the slot must still be free");
}

/// The fix in `spawn_run`, not the premise underneath it.
///
/// `a_job_handle_does_not_keep_its_registry_alive` (in `src/jobs.rs`) asserts
/// that a `JobHandle` holds only a `Weak` -- which is true with or without the
/// strong `Arc` the spawned task captures, so it constrains nothing here. This
/// asserts the capture itself: after the request's `AppState` is gone, the
/// detached run is the only thing left holding the registry, and `finish`
/// silently discards the outcome if that upgrade fails.
///
/// Deterministic, not a race. `#[tokio::test]` runs a current-thread runtime,
/// so the spawned task is not polled at all between `post_execute` returning
/// and the assertion below -- there is no window in which the run could have
/// completed and dropped its reference. The earlier belief that this needed a
/// slow script was simply wrong.
#[tokio::test]
async fn a_spawned_run_keeps_the_registry_alive_after_state_is_dropped() {
    let (_dir, state) = running_state();
    let weak = Arc::downgrade(&state.jobs);
    let response = post_execute(
        &state,
        serde_json::json!({ "code": "print(1)", "bot_count": 1 }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    drop(state);
    assert!(
        weak.upgrade().is_some(),
        "the detached run must keep the registry alive after AppState is gone"
    );
}

#[tokio::test]
async fn listing_jobs_returns_them_newest_first() {
    let (_dir, state) = test_state();
    for name in ["a.lua", "b.lua"] {
        let handle = state.jobs.try_start(Some(name.into())).expect("start");
        handle.finish(Ok((String::new(), String::new())));
    }
    let response = get(&state, "/api/v1/jobs").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body[0]["script"], serde_json::json!("b.lua"));
    assert_eq!(body[1]["script"], serde_json::json!("a.lua"));
}

/// `code: 4` is `ErrorResponse::not_found`, which only the handler produces.
/// The router's catch-all for unmatched `/api/v1/*` paths answers `404` too,
/// with `code: 404` -- so without the code assertion this test would pass just
/// as well against a build where the route was never registered at all.
#[tokio::test]
async fn an_unknown_job_is_not_found() {
    let (_dir, state) = test_state();
    let response = get(&state, "/api/v1/jobs/9999").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(response).await["code"], 4);
}

/// An id that is not a number is the same mistake as an id that is not there.
/// Without this the handler could answer a `400` (or, if it ever unwrapped the
/// parse, abort the process) on an input a browser can produce by hand.
#[tokio::test]
async fn a_job_id_that_is_not_a_number_is_not_found() {
    let (_dir, state) = test_state();
    let response = get(&state, "/api/v1/jobs/abc").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(response).await["code"], 4);
}

#[tokio::test]
async fn a_finished_job_reports_its_output() {
    let (_dir, state) = test_state();
    let handle = state.jobs.try_start(Some("a.lua".into())).expect("start");
    let id = handle.id();
    handle.finish(Ok(("printed".into(), String::new())));
    let response = get(&state, &format!("/api/v1/jobs/{id}")).await;
    let body = body_json(response).await;
    assert_eq!(body["status"], serde_json::json!("succeeded"));
    assert_eq!(body["stdout"], serde_json::json!("printed"));
}

/// Polls a job until it leaves `running`, or gives up.
///
/// The endpoint answers `202` before the script has run, so every assertion
/// about a real run has to wait for one. The bound is generous because it is
/// a failure bound, not a timing assumption: a script that has not finished a
/// `print` in ten seconds is broken, not slow.
async fn await_job(state: &AppState, id: &str) -> serde_json::Value {
    for _ in 0..1000 {
        let job = body_json(get(state, &format!("/api/v1/jobs/{id}")).await).await;
        if job["status"] != serde_json::json!("running") {
            return job;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("job {id} never finished");
}

/// The end-to-end path: a real script runs, its output reaches the job, and
/// the execution slot comes back.
///
/// Without this, every other test in this file could pass against a handler
/// that validated everything correctly and then spawned nothing at all.
#[tokio::test]
async fn executing_a_script_runs_it_and_records_its_output() {
    let (_dir, state) = running_state();
    let response = post_execute(
        &state,
        serde_json::json!({ "path": "/hello.lua", "bot_count": 1 }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = body_json(response).await;
    let id = accepted["job_id"].as_str().expect("job_id is a string");

    let job = await_job(&state, id).await;
    assert_eq!(
        job["status"],
        serde_json::json!("succeeded"),
        "job was {job}"
    );
    assert!(
        job["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("hello from the script"),
        "job was {job}"
    );
    assert!(
        state.jobs.running().is_none(),
        "the slot must be free once the run is over"
    );
}

/// Inline code takes the same path, with no file behind it. `script` is
/// `null` for it, which is what tells a job list which runs came from the
/// editor's "run selection".
#[tokio::test]
async fn executing_inline_code_runs_it_with_no_script_path() {
    let (_dir, state) = running_state();
    let response = post_execute(
        &state,
        serde_json::json!({ "code": "print(\"inline\")", "bot_count": 1 }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = body_json(response).await;
    let id = accepted["job_id"].as_str().expect("job_id is a string");

    let job = await_job(&state, id).await;
    assert_eq!(
        job["status"],
        serde_json::json!("succeeded"),
        "job was {job}"
    );
    assert_eq!(job["script"], serde_json::Value::Null, "job was {job}");
    assert!(
        job["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("inline"),
        "job was {job}"
    );
}

/// A script that raises fails the job rather than the request: the request was
/// accepted long before the failure existed.
#[tokio::test]
async fn a_script_that_fails_is_reported_on_the_job_not_the_request() {
    let (_dir, state) = running_state();
    let response = post_execute(
        &state,
        serde_json::json!({ "code": "error(\"boom\")", "bot_count": 1 }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = body_json(response).await;
    let id = accepted["job_id"].as_str().expect("job_id is a string");

    let job = await_job(&state, id).await;
    assert_eq!(job["status"], serde_json::json!("failed"), "job was {job}");
    assert!(
        job["error"].as_str().unwrap_or_default().contains("boom"),
        "job was {job}"
    );
    assert!(
        state.jobs.running().is_none(),
        "a failed run must still release the slot"
    );
}

/// `language` defaults to Lua, and anything else is refused before a job is
/// created -- there is no second interpreter to fall through to.
#[tokio::test]
async fn inline_code_in_an_unknown_language_is_a_bad_request() {
    let (_dir, state) = running_state();
    let response = post_execute(
        &state,
        serde_json::json!({ "code": "print(1)", "language": "python" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(state.jobs.list().is_empty(), "no job should have started");
}

/// Every other error body must be byte-for-byte what it was before
/// `running_job_id` existed. `skip_serializing_if` is what keeps that true;
/// without it every failure in the API would sprout a `"running_job_id": null`.
#[tokio::test]
async fn an_unrelated_error_body_carries_no_running_job_id() {
    let (_dir, state) = test_state();
    let body = body_json(post_execute(&state, serde_json::json!({})).await).await;
    assert!(
        body.get("running_job_id").is_none(),
        "unrelated errors must not carry the field: {body}"
    );
}

// ---------------------------------------------------------------------------
// `GET /api/v1/jobs/{id}/events`
// ---------------------------------------------------------------------------

/// The common case, not the exotic one: a fast script is over before a browser
/// can open the stream, so the finished-job path is what almost every real
/// request takes. `JobRegistry::subscribe` answers `None` both for a job that
/// never existed and for one whose channel was pruned on completion; reading
/// that `None` as a `404` would break this.
#[tokio::test]
async fn the_event_stream_replays_a_finished_job_and_ends() {
    let (_dir, state) = test_state();
    let handle = state.jobs.try_start(Some("a.lua".into())).expect("start");
    let id = handle.id();
    handle.line(Stream::Stdout, "hello");
    handle.finish(Ok(("hello".into(), String::new())));

    let response = get(&state, &format!("/api/v1/jobs/{id}/events")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    // That the whole body is collectable is itself the assertion: a stream
    // that stayed open would hang here, so the timeout turns that hang into a
    // named failure.
    let body = tokio::time::timeout(Duration::from_secs(5), collect_body(response))
        .await
        .expect("the stream must end once the job has finished");
    assert!(body.contains("event: output"), "body was {body:?}");
    assert!(body.contains("event: finished"), "body was {body:?}");
    // The exact `data:` payload, not a substring of it. Every other SSE
    // assertion in this file is a substring check, and a substring check
    // survives the payload being re-shaped around the text it looks for:
    // dropping `#[serde(untagged)]` from `WireEvent` emits
    // `data: {"output":{"stream":..,"text":..}}` and still contains "hello",
    // so the whole suite stays green while
    // `addEventListener("output", e => JSON.parse(e.data).text)` reads
    // `undefined` on every line and the output pane goes blank.
    assert!(
        body.contains(r#"data: {"stream":"stdout","text":"hello"}"#),
        "the `data:` payload must be the event's own fields, unwrapped; body was {body:?}"
    );
}

/// stderr, which nothing else in this file mentions.
///
/// `stream_name` has two arms and `backlog` chains two buffers, and until this
/// existed both halves of that were free: inverting `stream_name` to answer
/// `"stdout"` for `Stream::Stderr`, or dropping `backlog`'s second `chain`
/// entirely, left the suite green. What that costs is specific -- a script
/// that fails writes its diagnostic to stderr, so a late subscriber (the case
/// this endpoint exists for) would see `failed` with no error text at all, and
/// CI would agree that was fine.
#[tokio::test]
async fn the_event_stream_labels_stderr_as_its_own_stream() {
    let (_dir, state) = test_state();
    let handle = state.jobs.try_start(Some("bad.lua".into())).expect("start");
    let id = handle.id();
    handle.line(Stream::Stdout, "starting up");
    handle.line(Stream::Stderr, "something went wrong");
    handle.finish(Ok(("starting up".into(), "something went wrong".into())));

    let response = get(&state, &format!("/api/v1/jobs/{id}/events")).await;
    let body = tokio::time::timeout(Duration::from_secs(5), collect_body(response))
        .await
        .expect("the stream must end once the job has finished");
    assert!(
        body.contains(r#"data: {"stream":"stderr","text":"something went wrong"}"#),
        "a diagnostic written to stderr must reach the stream labelled as stderr; body was {body:?}"
    );
    assert!(
        body.contains(r#"data: {"stream":"stdout","text":"starting up"}"#),
        "and stdout must not be mislabelled on the way; body was {body:?}"
    );
}

#[tokio::test]
async fn the_event_stream_delivers_output_produced_after_subscribing() {
    let (_dir, state) = test_state();
    let handle = state.jobs.try_start(Some("a.lua".into())).expect("start");
    let id = handle.id();
    let response = get(&state, &format!("/api/v1/jobs/{id}/events")).await;

    tokio::spawn(async move {
        handle.line(Stream::Stdout, "late");
        handle.finish(Ok(("late".into(), String::new())));
    });

    let body = tokio::time::timeout(Duration::from_secs(5), collect_body(response))
        .await
        .expect("the stream must end when the job finishes");
    assert!(body.contains("late"), "body was {body:?}");
}

#[tokio::test]
async fn a_subscriber_that_falls_behind_is_told_it_missed_messages() {
    // The `lagged` event is in the wire contract and the handler is explicitly
    // told not to filter the `Lagged` arm away -- so it needs a test that
    // produces a real one, not a hand-built event. A hand-built event would
    // pass with the filter_map in place.
    //
    // The broadcast channel's capacity is 256. Subscribe, publish past
    // capacity WITHOUT reading -- `oneshot` returns the response before its
    // body has been polled even once, so none of these lines is consumed as it
    // is sent -- then read: the receiver reports how many it dropped.
    let (_dir, state) = test_state();
    let handle = state
        .jobs
        .try_start(Some("noisy.lua".into()))
        .expect("start");
    let id = handle.id();
    let response = get(&state, &format!("/api/v1/jobs/{id}/events")).await;
    assert_eq!(response.status(), StatusCode::OK);

    for index in 0..400 {
        handle.line(Stream::Stdout, &format!("line {index}"));
    }
    handle.finish(Ok((String::new(), String::new())));

    let body = tokio::time::timeout(Duration::from_secs(5), collect_body(response))
        .await
        .expect("the stream must end when the job finishes");
    assert!(
        body.contains("event: lagged"),
        "a subscriber that fell behind must be told, not silently short-changed; body was {body:?}"
    );
}

#[tokio::test]
async fn a_failed_job_reports_failed_in_its_terminal_event() {
    // `finished` carries a status, and the two arms are what the UI switches
    // on. A test that only ever exercises `succeeded` would pass with the
    // status hardcoded.
    let (_dir, state) = test_state();
    let handle = state.jobs.try_start(Some("bad.lua".into())).expect("start");
    let id = handle.id();
    handle.finish(Err(miette::miette!("script exploded")));

    let response = get(&state, &format!("/api/v1/jobs/{id}/events")).await;
    let body = tokio::time::timeout(Duration::from_secs(5), collect_body(response))
        .await
        .expect("the stream must end once the job has finished");
    assert!(body.contains("event: finished"), "body was {body:?}");
    assert!(
        body.contains("failed"),
        "the terminal event must carry the real status; body was {body:?}"
    );
}

#[tokio::test]
async fn the_event_stream_of_an_unknown_job_is_not_found() {
    let (_dir, state) = test_state();
    let response = get(&state, "/api/v1/jobs/9999/events").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // `code: 4` is `ErrorResponse::not_found`, which only the handler
    // produces; the router's catch-all for unmatched `/api/v1/*` paths
    // answers `404` with `code: 404`, so without this the test would pass
    // just as well against a build where the route was never registered.
    assert_eq!(body_json(response).await["code"], 4);
}

/// The keep-alive, which is otherwise free to delete.
///
/// Its consequence -- a proxy dropping a connection that has been idle through
/// a long silent script -- is not reproducible in-process, but the thing that
/// prevents it is: axum emits a comment frame on an idle stream, and without
/// `.keep_alive(..)` an idle stream emits nothing at all, ever.
///
/// `start_paused` is what makes that a test rather than a fifteen-second
/// sleep: with no other work to do, tokio's clock jumps straight to the next
/// pending timer, so the await below returns immediately in real time. Delete
/// the keep-alive and the only timer left is the `timeout`, which then fires
/// instead and names this test.
#[tokio::test(start_paused = true)]
async fn an_idle_stream_is_kept_alive() {
    use tokio_stream::StreamExt as _;

    let (_dir, state) = test_state();
    let handle = state
        .jobs
        .try_start(Some("quiet.lua".into()))
        .expect("start");
    let id = handle.id();
    // Deliberately silent: no output, and the job never finishes, so the only
    // thing that can ever come down this stream is a keep-alive.
    let response = get(&state, &format!("/api/v1/jobs/{id}/events")).await;
    let mut frames = response.into_body().into_data_stream();

    let frame = tokio::time::timeout(Duration::from_secs(600), frames.next())
        .await
        .expect("an idle stream must be kept alive, not left silent")
        .expect("the stream is still open")
        .expect("the frame is readable");
    assert!(
        frame.starts_with(b":"),
        "a keep-alive is an SSE comment; got {:?}",
        String::from_utf8_lossy(&frame)
    );

    // Held to the end: dropping it completes the job, which would end the
    // stream for a reason that is not the one under test.
    drop(handle);
}
