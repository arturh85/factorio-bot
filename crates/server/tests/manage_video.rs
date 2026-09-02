//! HTTP-level coverage for the video routes.
//!
//! The one that matters most is `a_range_request_is_answered_with_206`: a
//! `<video>` element seeks by asking for a byte range, and the frames route it
//! would otherwise have reused (`std::fs::read` into a `Vec<u8>`, plain 200)
//! cannot answer one. A recording served that way plays from the start and
//! nowhere else, and nothing about the response says why.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use serde_json::Value;
use tower::ServiceExt;

fn state_with_workspace(dir: &std::path::Path) -> AppState {
    let mut settings = AppSettings::default();
    settings.factorio.workspace_path = dir.to_string_lossy().into_owned().into();
    AppState {
        instance: FactorioInstance::new_shared(),
        settings: settings.into_shared(),
        settings_path: dir.join("AppSettings.toml"),
        starting: Default::default(),
        last_start_error: Default::default(),
        stop_generation: Default::default(),
        jobs: factorio_bot_server::jobs::JobRegistry::new(8),
    }
}

const BYTES: &[u8] = b"0123456789abcdefghijABCDEFGHIJ";

/// A video directory that looks exactly like one the recorder left behind.
fn seed_video(dir: &std::path::Path, run: &str, status: &str) -> std::path::PathBuf {
    let video = dir.join("video");
    std::fs::create_dir_all(&video).unwrap();
    std::fs::write(video.join("run.json"), format!(r#"{{"run":"{run}"}}"#)).unwrap();
    std::fs::write(video.join("video.mp4"), BYTES).unwrap();
    std::fs::write(
        video.join("ticks.jsonl"),
        "{\"t\":60551,\"w\":0,\"k\":\"start\"}\n\
         {\"t\":60581,\"w\":508}\n\
         {\"w\":5000,\"k\":\"gap\",\"reason\":\"stalled\"}\n\
         {\"t\":60700,\"w\":9000}\n\
         {\"t\":60700,\"w\"\n",
    )
    .unwrap();
    std::fs::write(
        video.join("video.json"),
        serde_json::json!({
            "run": run,
            "file": "video.mp4",
            "width": 1278, "height": 715,
            "requested_width": 1280, "requested_height": 720,
            "fps": 15,
            "status": status,
            "reason": null,
            "ffmpeg_exit": 0,
            "calibration": [
                {"host_wall_ms": 812, "out_time_ms": 0},
                {"host_wall_ms": 60408, "out_time_ms": 59598}
            ],
            "rate_ok": true,
            "window_id": "0x2c00007"
        })
        .to_string(),
    )
    .unwrap();
    video
}

async fn send(
    state: AppState,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = build_router(state, None).oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, bytes.to_vec())
}

async fn get_json(state: AppState, uri: &str) -> (StatusCode, Value) {
    let (status, _headers, bytes) = send(
        state,
        Request::builder().uri(uri).body(Body::empty()).unwrap(),
    )
    .await;
    let body: Value = serde_json::from_slice(&bytes).unwrap_or_else(|err| {
        panic!(
            "{uri} did not answer JSON: {err}; body = {:?}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, body)
}

/// A workspace that never recorded video answers a manifest describing nothing,
/// not a 404. Video is opt-in, so "there is none" is the ordinary case and a
/// caller must be able to tell it from a route that is not there.
#[tokio::test]
async fn a_workspace_with_no_recording_answers_an_empty_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/video").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["run"], Value::Null, "{body}");
    assert_eq!(body["video"], Value::Null, "{body}");
    assert_eq!(body["bytes"], Value::Null, "{body}");
    assert_eq!(body["samples"], 0, "{body}");
    assert_eq!(body["tick_range"], Value::Null, "{body}");
}

#[tokio::test]
async fn the_manifest_reports_the_observed_geometry_and_the_tick_span() {
    let dir = tempfile::tempdir().unwrap();
    seed_video(dir.path(), "run-7", "stopped");
    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/video").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["run"], "run-7", "{body}");
    assert_eq!(body["bytes"], BYTES.len(), "{body}");
    // The observed size, not the 1280x720 that was asked for.
    assert_eq!(body["video"]["width"], 1278, "{body}");
    assert_eq!(body["video"]["requested_width"], 1280, "{body}");
    assert_eq!(body["tick_range"]["from"], 60551, "{body}");
    assert_eq!(body["tick_range"]["to"], 60700, "{body}");
    // The torn last line is counted, not hidden.
    assert_eq!(body["skipped"], 1, "{body}");
}

/// The clock reaches the client whole, gap lines included. Filtering them would
/// leave the viewer with two ordinary samples straddling a stall and no way to
/// know, which is the one thing this clock exists to prevent.
#[tokio::test]
async fn the_clock_is_served_with_its_gaps_intact() {
    let dir = tempfile::tempdir().unwrap();
    seed_video(dir.path(), "run-7", "stopped");
    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/video/ticks").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let samples = body["samples"].as_array().expect("samples array");
    assert_eq!(samples.len(), 4, "{body}");
    assert_eq!(samples[2]["k"], "gap", "{body}");
    assert!(
        samples[2].get("t").is_none(),
        "a gap observed no tick: {body}"
    );
    assert_eq!(body["skipped"], 1, "{body}");
}

/// The whole point of not reusing `get_frame`. A `<video>` seeks by asking for
/// a byte range; a plain 200 with the whole file cannot answer one.
#[tokio::test]
async fn a_range_request_is_answered_with_206() {
    let dir = tempfile::tempdir().unwrap();
    seed_video(dir.path(), "run-7", "stopped");
    let (status, headers, bytes) = send(
        state_with_workspace(dir.path()),
        Request::builder()
            .uri("/api/v1/video/file")
            .header(header::RANGE, "bytes=0-9")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(bytes, b"0123456789");
    assert_eq!(
        headers.get(header::CONTENT_RANGE).unwrap(),
        &format!("bytes 0-9/{}", BYTES.len())
    );
}

#[tokio::test]
async fn the_whole_file_is_served_with_accept_ranges() {
    let dir = tempfile::tempdir().unwrap();
    seed_video(dir.path(), "run-7", "stopped");
    let (status, headers, bytes) = send(
        state_with_workspace(dir.path()),
        Request::builder()
            .uri("/api/v1/video/file")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, BYTES);
    assert_eq!(headers.get(header::ACCEPT_RANGES).unwrap(), "bytes");
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "video/mp4");
}

/// The live recording is overwritten by the next run *at the same URL*. Caching
/// it `immutable`, the way a frame is cached, would show the previous run's
/// recording beside this run's timeline with nothing saying so.
#[tokio::test]
async fn the_live_recording_is_never_cached_as_immutable() {
    let dir = tempfile::tempdir().unwrap();
    seed_video(dir.path(), "run-7", "stopped");
    let (_status, headers, _bytes) = send(
        state_with_workspace(dir.path()),
        Request::builder()
            .uri("/api/v1/video/file")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let cache = headers
        .get(header::CACHE_CONTROL)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(!cache.contains("immutable"), "{cache}");
}

#[tokio::test]
async fn asking_for_a_recording_that_does_not_exist_is_a_404() {
    let dir = tempfile::tempdir().unwrap();
    let (status, _headers, _bytes) = send(
        state_with_workspace(dir.path()),
        Request::builder()
            .uri("/api/v1/video/file")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A `video.json` naming a file outside the video directory must be refused by
/// `resolve_script_path` before `ServeFile` ever sees it. These routes are
/// unauthenticated, so that guard is the only thing between an HTTP caller and
/// the filesystem.
#[tokio::test]
async fn a_record_naming_a_file_outside_the_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let video = seed_video(dir.path(), "run-7", "stopped");
    std::fs::write(dir.path().join("secret.txt"), b"not yours").unwrap();
    let record = std::fs::read_to_string(video.join("video.json")).unwrap();
    std::fs::write(
        video.join("video.json"),
        record.replace("\"video.mp4\"", "\"../secret.txt\""),
    )
    .unwrap();

    let (status, _headers, bytes) = send(
        state_with_workspace(dir.path()),
        Request::builder()
            .uri("/api/v1/video/file")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "the traversal guard must refuse it");
    assert_ne!(bytes, b"not yours");
}

/// The archived side, including the caching difference: an archived run is over,
/// so its bytes never change.
#[tokio::test]
async fn an_archived_recording_is_served_by_run_id_and_cached_forever() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("runs").join("run-7");
    std::fs::create_dir_all(&run_dir).unwrap();
    seed_video(&run_dir, "run-7", "stopped");

    let (status, body) =
        get_json(state_with_workspace(dir.path()), "/api/v1/runs/run-7/video").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["run"], "run-7", "{body}");

    let (status, headers, bytes) = send(
        state_with_workspace(dir.path()),
        Request::builder()
            .uri("/api/v1/runs/run-7/video/file")
            .header(header::RANGE, "bytes=10-19")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(bytes, b"abcdefghij");
    let cache = headers
        .get(header::CACHE_CONTROL)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cache.contains("immutable"), "{cache}");
}

/// A run that never recorded video is not an error, and neither is one recorded
/// before video existed.
#[tokio::test]
async fn an_archived_run_with_no_video_answers_an_empty_manifest() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("runs").join("run-8")).unwrap();
    let (status, body) =
        get_json(state_with_workspace(dir.path()), "/api/v1/runs/run-8/video").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["video"], Value::Null, "{body}");

    let (status, body) = get_json(
        state_with_workspace(dir.path()),
        "/api/v1/runs/run-8/video/ticks",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["samples"].as_array().unwrap().len(), 0, "{body}");
}

/// **A `recording` status in an archived run is proof the recorder was never
/// stopped.** The server forwards it verbatim rather than normalising it to
/// something that reads as complete -- that judgement belongs to the viewer,
/// and it can only make it if the status survives the trip.
#[tokio::test]
async fn a_recorder_that_outlived_its_run_is_reported_as_still_recording() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("runs").join("run-9");
    std::fs::create_dir_all(&run_dir).unwrap();
    seed_video(&run_dir, "run-9", "recording");

    let (status, body) =
        get_json(state_with_workspace(dir.path()), "/api/v1/runs/run-9/video").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["video"]["status"], "recording", "{body}");
}
