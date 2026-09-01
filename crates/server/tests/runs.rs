//! `/api/v1/runs/*` over fixture run directories on disk.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_server::state::AppState;
use factorio_bot_server::webserver::build_router;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tower::ServiceExt;

fn state_with_workspace(dir: &Path) -> AppState {
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

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fb-runs-api-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a run directory. `manifest` absent = a run that never finished.
fn seed_run(ws: &Path, id: &str, events: &str, manifest: Option<&str>, splits: Option<&str>) {
    let dir = ws.join("runs").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("events.jsonl"), events).unwrap();
    if let Some(m) = manifest {
        std::fs::write(dir.join("manifest.json"), m).unwrap();
    }
    if let Some(s) = splits {
        std::fs::write(dir.join("splits.json"), s).unwrap();
    }
}

async fn get(state: AppState, uri: &str) -> (StatusCode, Vec<u8>) {
    let response = build_router(state, None)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, bytes)
}

async fn get_json(state: AppState, uri: &str) -> (StatusCode, Value) {
    let (status, bytes) = get(state, uri).await;
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

const MILESTONES: &str = concat!(
    r#"{"tick":10,"wall_ms":1,"kind":"milestone_started","index":0,"goal":"researched(automation)"}"#,
    "\n",
    r#"{"tick":310,"wall_ms":2,"kind":"milestone_satisfied","index":0,"iterations":2,"elapsed_ticks":300}"#,
    "\n",
);

const MANIFEST: &str = r#"{"run_id":"alpha","started_unix":1000,"finished_unix":1100,
  "outcome":"done","elapsed_ticks":400,"events":3,"frames":0,"splits":1}"#;

#[tokio::test]
async fn a_workspace_that_never_recorded_a_run_lists_nothing_rather_than_404() {
    let ws = workspace("empty");
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs").await;
    assert_eq!(status, StatusCode::OK, "no runs yet is a state, not a 404");
    assert_eq!(body["runs"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_finished_run_lists_its_manifest_values() {
    let ws = workspace("finished");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs").await;
    assert_eq!(status, StatusCode::OK);
    let run = &body["runs"][0];
    assert_eq!(run["run_id"], "alpha");
    assert_eq!(run["finished"], true);
    assert_eq!(run["outcome"], "done");
    assert_eq!(run["elapsed_ticks"], 400);
}

#[tokio::test]
async fn a_run_that_never_finished_is_listed_with_nulls_not_hidden() {
    let ws = workspace("unfinished");
    seed_run(&ws, "crashed", MILESTONES, None, None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs").await;
    assert_eq!(status, StatusCode::OK);
    let run = &body["runs"][0];
    assert_eq!(run["run_id"], "crashed");
    assert_eq!(run["finished"], false);
    assert!(
        run.get("outcome").is_some() && run["outcome"].is_null(),
        "outcome must be present-and-null, not omitted: {run}"
    );
}

#[tokio::test]
async fn an_unfinished_run_still_shows_the_splits_it_got_through() {
    let ws = workspace("derived");
    // No splits.json -- the run died before finishing. Splits derive from the log.
    seed_run(&ws, "crashed", MILESTONES, None, None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/crashed").await;
    assert_eq!(status, StatusCode::OK);
    let splits = body["splits"].as_array().unwrap();
    assert_eq!(splits.len(), 1, "a crashed run is the one worth opening");
    assert_eq!(splits[0]["elapsed_ticks"], 300);
    assert_eq!(splits[0]["outcome"], "satisfied");
}

#[tokio::test]
async fn a_truncated_event_log_is_served_with_its_skipped_count() {
    let ws = workspace("truncated");
    let truncated = format!("{MILESTONES}{{\"tick\":400,\"wall_ms\":3,\"kind\":\"run_fin");
    seed_run(&ws, "cut", &truncated, None, None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/cut/events").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["events"].as_array().unwrap().len(), 2);
    assert_eq!(body["skipped"], 1, "the lost line is reported, not hidden");
}

#[tokio::test]
async fn events_can_be_filtered_by_kind() {
    let ws = workspace("filter");
    seed_run(&ws, "alpha", MILESTONES, None, None);
    let (status, body) = get_json(
        state_with_workspace(&ws),
        "/api/v1/runs/alpha/events?kind=milestone_satisfied",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let events = body["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "milestone_satisfied");
}

#[tokio::test]
async fn a_run_with_no_frames_reports_an_empty_index_rather_than_404() {
    let ws = workspace("noframes");
    seed_run(&ws, "planning", MILESTONES, None, None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/planning/frames").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a planning-only run is valid, not degenerate"
    );
    assert_eq!(body["frames"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_missing_run_is_404() {
    let ws = workspace("missing");
    seed_run(&ws, "alpha", MILESTONES, None, None);
    let (status, _) = get_json(state_with_workspace(&ws), "/api/v1/runs/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_run_id_escaping_the_runs_directory_is_refused() {
    let ws = workspace("traversal");
    seed_run(&ws, "alpha", MILESTONES, None, None);
    // Something worth stealing, one level up from runs/.
    std::fs::write(ws.join("AppSettings.toml"), "secret").unwrap();

    for id in ["..", "..%2f..", "alpha/../.."] {
        let (status, _) = get_json(state_with_workspace(&ws), &format!("/api/v1/runs/{id}")).await;
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST,
            "id {id:?} must not resolve outside runs/, got {status}"
        );
    }
}

#[tokio::test]
async fn a_frame_name_escaping_the_run_is_refused() {
    let ws = workspace("frametraversal");
    seed_run(&ws, "alpha", MILESTONES, None, None);
    let bot = ws.join("runs").join("alpha").join("frames").join("1");
    std::fs::create_dir_all(&bot).unwrap();
    std::fs::write(bot.join("tick-0000300-front.jpg"), b"jpeg").unwrap();
    std::fs::write(ws.join("runs").join("alpha").join("splits.json"), "[]").unwrap();

    let (ok, _) = get(
        state_with_workspace(&ws),
        "/api/v1/runs/alpha/frames/1/tick-0000300-front.jpg",
    )
    .await;
    assert_eq!(ok, StatusCode::OK, "the real frame must still be served");

    let (status, _) = get(
        state_with_workspace(&ws),
        "/api/v1/runs/alpha/frames/1/../../splits.json",
    )
    .await;
    assert!(
        status != StatusCode::OK,
        "a frame name must not reach outside its bot directory, got {status}"
    );
}
