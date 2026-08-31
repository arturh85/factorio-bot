use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use serde_json::Value;
use tower::ServiceExt;

/// Points `workspace_path` at a temp dir. Never touches the real workspace.
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

fn frames_dir(workspace: &std::path::Path, client: u8) -> std::path::PathBuf {
    workspace
        .join(format!("client{client}"))
        .join("script-output")
        .join("frames")
}

async fn get(state: AppState, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = build_router(state, None)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, bytes.to_vec())
}

async fn get_json(state: AppState, uri: &str) -> (StatusCode, Value) {
    let (status, _headers, bytes) = get(state, uri).await;
    let body: Value = serde_json::from_slice(&bytes).unwrap_or_else(|err| {
        panic!(
            "{uri} did not answer JSON: {err}; body = {:?}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, body)
}

/// Test 1: a parseable name yields its tick and camera.
#[tokio::test]
async fn a_parseable_name_yields_its_tick_and_camera() {
    let dir = tempfile::tempdir().unwrap();
    let frames = frames_dir(dir.path(), 1);
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(frames.join("tick-0000000300-front.jpg"), b"jpeg-bytes").unwrap();

    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/frames").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let entries = body["frames"].as_array().expect("frames array");
    assert_eq!(entries.len(), 1, "{body}");
    assert_eq!(entries[0]["client"], 1, "{body}");
    assert_eq!(entries[0]["tick"], 300, "{body}");
    assert_eq!(entries[0]["camera"], "front", "{body}");
    assert_eq!(entries[0]["name"], "tick-0000000300-front.jpg", "{body}");
    assert_eq!(entries[0]["bytes"], 10, "{body}");
}

/// Test 2: a gap survives. Ticks 300 and 900 present, 600 absent -> the
/// manifest has two entries and nothing at 600. This is the property the
/// whole design exists for: a manifest computed from a start tick and a
/// stride would fabricate an entry at 600 and it would look entirely
/// plausible.
#[tokio::test]
async fn a_gap_in_the_tick_sequence_survives_to_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let frames = frames_dir(dir.path(), 1);
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(frames.join("tick-0000000300-front.jpg"), b"a").unwrap();
    std::fs::write(frames.join("tick-0000000900-front.jpg"), b"b").unwrap();

    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/frames").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let entries = body["frames"].as_array().expect("frames array");
    assert_eq!(entries.len(), 2, "{body}");

    let ticks: Vec<Option<u64>> = entries.iter().map(|e| e["tick"].as_u64()).collect();
    assert_eq!(ticks, vec![Some(300), Some(900)], "{body}");
    assert!(
        !ticks.contains(&Some(600)),
        "the manifest must never fabricate an entry at a dropped tick: {body}"
    );
}

/// Test 3: an unparseable name is reported with a null tick, not dropped.
#[tokio::test]
async fn an_unparseable_name_is_reported_with_a_null_tick() {
    let dir = tempfile::tempdir().unwrap();
    let frames = frames_dir(dir.path(), 1);
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(frames.join("not-a-frame-name.jpg"), b"c").unwrap();

    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/frames").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let entries = body["frames"].as_array().expect("frames array");
    assert_eq!(
        entries.len(),
        1,
        "an unparseable file must not be dropped: {body}"
    );
    assert_eq!(entries[0]["name"], "not-a-frame-name.jpg", "{body}");
    assert!(entries[0]["tick"].is_null(), "{body}");
    assert!(entries[0]["camera"].is_null(), "{body}");
    // The keys must be present, not omitted -- a consumer must not have to
    // distinguish "absent from the document" from "absent as a fact".
    assert!(
        entries[0].as_object().unwrap().contains_key("tick"),
        "{body}"
    );
    assert!(
        entries[0].as_object().unwrap().contains_key("camera"),
        "{body}"
    );
}

/// Test 4: a traversal attempt is refused, both the literal form and a
/// URL-encoded variant.
#[tokio::test]
async fn traversal_outside_the_frames_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let frames = frames_dir(dir.path(), 1);
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(dir.path().join("secret.txt"), "top secret").unwrap();

    for uri in [
        "/api/v1/frames/1/../../secret.txt",
        "/api/v1/frames/1/..%2F..%2Fsecret.txt",
    ] {
        let (status, _headers, body) = get(state_with_workspace(dir.path()), uri).await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
            "{uri}: expected a refusal, got {status}"
        );
        let text = String::from_utf8_lossy(&body);
        assert!(
            !text.contains("top secret"),
            "{uri} leaked file contents: {text}"
        );
    }
}

/// Test 5: frame bytes come back with `image/jpeg` and the immutable cache
/// header -- a scrubber re-requests the same frame constantly, and this
/// header is the difference between a usable one and a slow one.
#[tokio::test]
async fn frame_bytes_are_served_as_immutable_jpeg() {
    let dir = tempfile::tempdir().unwrap();
    let frames = frames_dir(dir.path(), 1);
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(frames.join("tick-0000000300-front.jpg"), b"totally-a-jpeg").unwrap();

    let (status, headers, body) = get(
        state_with_workspace(dir.path()),
        "/api/v1/frames/1/tick-0000000300-front.jpg",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("content-type").map(|v| v.to_str().unwrap()),
        Some("image/jpeg")
    );
    assert_eq!(
        headers.get("cache-control").map(|v| v.to_str().unwrap()),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(body, b"totally-a-jpeg");
}

/// Test 6: the three empty/missing states differ.
#[tokio::test]
async fn the_three_empty_states_are_distinguishable() {
    // State 1: no workspace / no client directories at all -- the run has
    // never happened. `dir` exists (it must, to build a valid AppState) but
    // holds nothing under it.
    let dir = tempfile::tempdir().unwrap();
    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/frames").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["clients"].as_array().unwrap().len(), 0, "{body}");
    assert_eq!(body["frames"].as_array().unwrap().len(), 0, "{body}");
    let never_happened = body;

    // State 2: a client directory exists, but capture has not produced a
    // frame yet.
    let dir2 = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(frames_dir(dir2.path(), 1)).unwrap();
    let (status, body) = get_json(state_with_workspace(dir2.path()), "/api/v1/frames").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["clients"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["frames"].as_array().unwrap().len(), 0, "{body}");
    let capture_not_run = body;

    // State 3: frames are present.
    let dir3 = tempfile::tempdir().unwrap();
    let frames3 = frames_dir(dir3.path(), 1);
    std::fs::create_dir_all(&frames3).unwrap();
    std::fs::write(frames3.join("tick-0000000300-front.jpg"), b"x").unwrap();
    let (status, body) = get_json(state_with_workspace(dir3.path()), "/api/v1/frames").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["clients"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["frames"].as_array().unwrap().len(), 1, "{body}");
    let frames_present = body;

    assert_ne!(
        never_happened, capture_not_run,
        "states 1 and 2 must differ"
    );
    assert_ne!(
        capture_not_run, frames_present,
        "states 2 and 3 must differ"
    );
    assert_ne!(never_happened, frames_present, "states 1 and 3 must differ");
}

/// A hyphenated camera id must survive whole, end to end through the route --
/// not just in the unit-level parser test in `frames.rs`.
#[tokio::test]
async fn a_hyphenated_camera_id_survives_through_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let frames = frames_dir(dir.path(), 1);
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(frames.join("tick-0001800-bot-1.jpg"), b"y").unwrap();

    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/frames").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["frames"].as_array().expect("frames array");
    assert_eq!(entries.len(), 1, "{body}");
    assert_eq!(entries[0]["tick"], 1800, "{body}");
    assert_eq!(entries[0]["camera"], "bot-1", "{body}");
}

/// Multiple `client<N>` directories are all represented, tagged with their
/// own client number, rather than only the first being served.
#[tokio::test]
async fn multiple_clients_are_all_represented() {
    let dir = tempfile::tempdir().unwrap();
    let frames1 = frames_dir(dir.path(), 1);
    let frames2 = frames_dir(dir.path(), 2);
    std::fs::create_dir_all(&frames1).unwrap();
    std::fs::create_dir_all(&frames2).unwrap();
    std::fs::write(frames1.join("tick-0000000100-front.jpg"), b"a").unwrap();
    std::fs::write(frames2.join("tick-0000000100-front.jpg"), b"b").unwrap();

    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/frames").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["clients"].as_array().unwrap().len(), 2, "{body}");
    assert_eq!(body["frames"].as_array().unwrap().len(), 2, "{body}");
}

/// Two clients capturing the same tick is the ONE legitimate duplicate: per-bot
/// cameras mean `client1` and `client2` both fire at tick 300, and both files
/// are correct and different. They are distinguished by their directory, which
/// is what `FrameEntry.client` reports.
///
/// This is a regression test for a real defect. The first version of
/// `get_frame` took only a name and searched the client directories in order,
/// returning the first match — so the manifest listed two entries and the byte
/// route could serve only one of them, and the higher-numbered client's frame
/// was unreachable. A scrubber showing two bots would have shown the same
/// picture twice with nothing saying so.
#[tokio::test]
async fn two_clients_at_the_same_tick_are_both_reachable() {
    let dir = tempfile::tempdir().unwrap();
    for (client, body) in [(1u8, b"client-one-pixels"), (2u8, b"client-two-pixels")] {
        let frames = frames_dir(dir.path(), client);
        std::fs::create_dir_all(&frames).unwrap();
        std::fs::write(frames.join("tick-0000300-follow.jpg"), body).unwrap();
    }

    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/frames").await;
    assert_eq!(status, StatusCode::OK);
    let frames = body["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 2, "the manifest must report both: {body}");
    assert_eq!(frames[0]["name"], frames[1]["name"], "same name, by design");

    // The bytes must differ, which is only possible if the client addresses it.
    let (s1, _, b1) = get(
        state_with_workspace(dir.path()),
        "/api/v1/frames/1/tick-0000300-follow.jpg",
    )
    .await;
    let (s2, _, b2) = get(
        state_with_workspace(dir.path()),
        "/api/v1/frames/2/tick-0000300-follow.jpg",
    )
    .await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(b1, b"client-one-pixels".as_slice());
    assert_eq!(
        b2,
        b"client-two-pixels".as_slice(),
        "client 2 got client 1's frame — the route is ignoring the client"
    );
}

/// A client that exists in the manifest but is not the one asked for must not
/// be substituted. A fallback is how the previous version returned the wrong
/// image under the right name, which is worse than a 404 because the caller
/// cannot tell it happened.
#[tokio::test]
async fn a_frame_is_not_served_from_a_different_client() {
    let dir = tempfile::tempdir().unwrap();
    let frames = frames_dir(dir.path(), 1);
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(frames.join("tick-0000300-follow.jpg"), b"only-client-one").unwrap();
    std::fs::create_dir_all(frames_dir(dir.path(), 2)).unwrap();

    let (status, _, body) = get(
        state_with_workspace(dir.path()),
        "/api/v1/frames/2/tick-0000300-follow.jpg",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "got: {}",
        String::from_utf8_lossy(&body)
    );
    assert!(!body.starts_with(b"only-client-one"));
}

/// `run.json` is surfaced as the manifest's run id and **not** as a frame row.
///
/// It is the one name excluded from "report every file you find". That rule
/// exists so an unexplained file cannot be hidden, and this file's meaning is
/// known and reported — excluding it from `frames` is not hiding it. Reporting
/// it as a null-tick frame instead would put a permanent unparseable row in
/// every manifest that has ever captured anything.
#[tokio::test]
async fn the_run_sidecar_becomes_the_run_id_not_a_frame_row() {
    let dir = tempfile::tempdir().unwrap();
    let frames = frames_dir(dir.path(), 1);
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(frames.join("run.json"), r#"{"run":"job-7"}"#).unwrap();
    std::fs::write(frames.join("tick-0000300-follow.jpg"), b"pixels").unwrap();

    let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/frames").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["run"], "job-7");
    let names: Vec<&str> = body["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["tick-0000300-follow.jpg"],
        "run.json must not appear as a frame row"
    );
}

/// A malformed or absent sidecar yields `null`, which means **unknown** and
/// never **no match**.
///
/// A consumer that read `null` as a mismatch would refuse a join that is
/// perfectly good; one that read it as a match would assert something nobody
/// established. Both are wrong, and the manifest can only avoid causing either
/// by saying nothing rather than guessing.
#[tokio::test]
async fn an_unreadable_run_sidecar_is_null_rather_than_a_guess() {
    for (label, contents) in [
        ("not json", "this is not json"),
        ("no run key", r#"{"other":"x"}"#),
        ("run is not a string", r#"{"run":42}"#),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let frames = frames_dir(dir.path(), 1);
        std::fs::create_dir_all(&frames).unwrap();
        std::fs::write(frames.join("run.json"), contents).unwrap();
        std::fs::write(frames.join("tick-0000300-follow.jpg"), b"pixels").unwrap();

        let (status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/frames").await;
        assert_eq!(status, StatusCode::OK, "{label}");
        assert!(body["run"].is_null(), "{label}: got {}", body["run"]);
        assert_eq!(
            body["frames"].as_array().unwrap().len(),
            1,
            "{label}: a bad sidecar must not cost us the frames"
        );
    }
}

/// No sidecar at all is also `null` — capture started without an id.
#[tokio::test]
async fn no_run_sidecar_is_null() {
    let dir = tempfile::tempdir().unwrap();
    let frames = frames_dir(dir.path(), 1);
    std::fs::create_dir_all(&frames).unwrap();
    std::fs::write(frames.join("tick-0000300-follow.jpg"), b"pixels").unwrap();

    let (_status, body) = get_json(state_with_workspace(dir.path()), "/api/v1/frames").await;
    assert!(body["run"].is_null());
}
