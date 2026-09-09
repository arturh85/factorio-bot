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
  "outcome":"done","elapsed_ticks":400,"events":3,"splits":1}"#;

const MANIFEST_AT_1000: &str = r#"{"run_id":"run-1000-00001","started_unix":1000,
  "finished_unix":1100,"outcome":"done","elapsed_ticks":400,"events":3,"splits":1}"#;
const MANIFEST_AT_2000: &str = r#"{"run_id":"run-2000-00001","started_unix":2000,
  "finished_unix":2100,"outcome":"done","elapsed_ticks":400,"events":3,"splits":1}"#;

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
async fn a_run_with_no_sample_file_reports_an_empty_list_rather_than_404() {
    // The mod may not have shipped, or the run may predate sampling entirely.
    let ws = workspace("nosamples");
    seed_run(&ws, "planning", MILESTONES, None, None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/planning/samples").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a run recorded before sampling existed is not an error"
    );
    assert_eq!(body["samples"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_runs_archived_samples_are_served() {
    let ws = workspace("samples");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    std::fs::write(
        ws.join("runs").join("alpha").join("samples.jsonl"),
        r#"{"kind":"bots","schema":2,"tick":310,"run":"alpha","bots":[]}"#,
    )
    .unwrap();
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/samples").await;
    assert_eq!(status, StatusCode::OK);
    let samples = body["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0]["tick"], 310);
    assert_eq!(body["skipped"], 0);
}

#[tokio::test]
async fn a_run_with_an_unparseable_sample_line_reports_its_skipped_count() {
    // Concrete case this guards: the mod writes `bots = {}` when no player is
    // connected, and `helpers.table_to_json({})` yields `"{}"` rather than
    // `"[]"`, so that line fails to deserialise as a `Sample` -- it must be
    // counted, not vanish silently.
    let ws = workspace("samples-skipped");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    std::fs::write(
        ws.join("runs").join("alpha").join("samples.jsonl"),
        concat!(
            r#"{"kind":"bots","schema":2,"tick":310,"run":"alpha","bots":[]}"#,
            "\n",
            r#"{"kind":"bots","schema":2,"tick":320,"run":"alpha","bots":{}}"#,
            "\n"
        ),
    )
    .unwrap();
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/samples").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["samples"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["skipped"], 1,
        "the unparseable line is reported, not hidden"
    );
}

#[tokio::test]
async fn a_run_with_no_map_file_reports_an_empty_list_rather_than_404() {
    // A run recorded before this feature existed, or one that placed nothing.
    let ws = workspace("nomap");
    seed_run(&ws, "planning", MILESTONES, None, None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/planning/map").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a run recorded before the map existed is not an error"
    );
    assert_eq!(body["map"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_runs_archived_map_is_served() {
    let ws = workspace("map");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    std::fs::write(
        ws.join("runs").join("alpha").join("map.jsonl"),
        concat!(
            r#"{"tick":310,"kind":"placed","bot":1,"#,
            r#""intent":{"name":"stone-furnace","position":{"x":-12.0,"y":8.0},"direction":0},"#,
            r#""actual":{"name":"stone-furnace","position":{"x":-12.0,"y":8.0},"direction":0},"#,
            r#""drift":null}"#,
            "\n"
        ),
    )
    .unwrap();
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/map").await;
    assert_eq!(status, StatusCode::OK);
    let map = body["map"].as_array().unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map[0]["tick"], 310);
    assert_eq!(map[0]["kind"], "placed");
    assert_eq!(body["skipped"], 0);
}

#[tokio::test]
async fn a_run_with_a_truncated_map_line_reports_its_skipped_count() {
    let ws = workspace("map-skipped");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    std::fs::write(
        ws.join("runs").join("alpha").join("map.jsonl"),
        concat!(
            r#"{"tick":310,"kind":"placed","bot":1,"#,
            r#""intent":{"name":"stone-furnace","position":{"x":-12.0,"y":8.0},"direction":0},"#,
            r#""actual":{"name":"stone-furnace","position":{"x":-12.0,"y":8.0},"direction":0},"#,
            r#""drift":null}"#,
            "\n",
            r#"{"tick":320,"kind":"placed","bot":1,"intent":{"#,
            "\n"
        ),
    )
    .unwrap();
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/map").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["map"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["skipped"], 1,
        "the truncated line is reported, not hidden"
    );
}

#[tokio::test]
async fn an_unfinished_run_sorts_by_the_time_in_its_id_not_to_the_bottom() {
    // A crashed run is usually the newest and the most interesting; sorting
    // every manifest-less run last buries exactly the ones worth opening.
    let ws = workspace("ordering");
    seed_run(
        &ws,
        "run-1000-00001",
        MILESTONES,
        Some(MANIFEST_AT_1000),
        None,
    );
    seed_run(&ws, "run-3000-00001", MILESTONES, None, None); // newest, crashed
    seed_run(
        &ws,
        "run-2000-00001",
        MILESTONES,
        Some(MANIFEST_AT_2000),
        None,
    );

    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs").await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = body["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["run_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["run-3000-00001", "run-2000-00001", "run-1000-00001"]
    );
    assert!(
        body["runs"][0]["started_unix"].is_null(),
        "ordering may use the id; the field must still say we do not know"
    );
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

const PROVENANCE: &str = r#"{"schema":1,"run_id":"alpha","started_unix":1000,"started_tick":3242,
  "seed":"31337","map_exchange_string":null,"map":{"digest":"c161fa3f437221d0","tiles":{"iron-ore":940}},
  "factorio":"2.1.17","mods":{"base":"2.1.17","BotBridge":"0.0.1"},
  "git":{"commit":"492e513a261bde8f4c433ecdd4e749918f9a6160","dirty":false,"source":"working-tree-at-run-start"},
  "profile":"release","roster_requested":[1,2,3,4],"workspace":"/w","resumed_from":null,
  "bot_mode":"clients","game_speed":1.0,"peaceful":null}"#;

#[tokio::test]
async fn provenance_is_served_verbatim() {
    let ws = workspace("prov");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    std::fs::write(ws.join("runs/alpha/provenance.json"), PROVENANCE).unwrap();
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/provenance").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["seed"], "31337");
    assert_eq!(body["git"]["dirty"], false);
    assert_eq!(body["mods"]["BotBridge"], "0.0.1");
    assert_eq!(body["map"]["tiles"]["iron-ore"], 940);
    // Present-and-null survives the round trip: `None` is an answer.
    assert!(body.get("map_exchange_string").is_some());
    assert!(body["map_exchange_string"].is_null());
}

#[tokio::test]
async fn a_run_without_provenance_is_a_404_not_an_empty_object() {
    let ws = workspace("noprov");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha/provenance").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("recorded no provenance")
    );
}

const MANIFEST_WITH_COVERAGE: &str = r#"{"run_id":"alpha","started_unix":1000,"finished_unix":1100,
  "outcome":"done","elapsed_ticks":400,"events":3,"splits":1,"samples":2674,"map":24,"samples_lag_ticks":0}"#;

#[tokio::test]
async fn a_summary_carries_the_manifest_coverage_counts() {
    let ws = workspace("coverage");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST_WITH_COVERAGE), None);
    let (status, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["summary"]["samples"], 2674);
    assert_eq!(body["summary"]["map"], 24);
    assert_eq!(body["summary"]["samples_lag_ticks"], 0);
}

#[tokio::test]
async fn an_old_manifest_reports_lag_as_null_and_counts_as_zero() {
    // `samples`/`map` are `#[serde(default)]` on Manifest (0 = no such file),
    // `samples_lag_ticks` is Option (None = no samples). The summary must keep
    // that distinction rather than flatten it.
    let ws = workspace("oldmanifest");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    let (_, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/alpha").await;
    assert_eq!(body["summary"]["samples"], 0);
    assert!(body["summary"]["samples_lag_ticks"].is_null());
}

#[tokio::test]
async fn an_unfinished_run_has_null_coverage() {
    let ws = workspace("unfinished-cov");
    seed_run(&ws, "beta", MILESTONES, None, None);
    let (_, body) = get_json(state_with_workspace(&ws), "/api/v1/runs/beta").await;
    assert!(body["summary"]["samples"].is_null());
    assert!(body["summary"]["map"].is_null());
    assert!(body["summary"]["samples_lag_ticks"].is_null());
}

const SAMPLES: &str = concat!(
    r#"{"schema":3,"tick":600,"run":"alpha","kind":"bots","bots":[]}"#,
    "\n",
    r#"{"schema":3,"tick":600,"run":"alpha","kind":"force","research":null,"techs_unlocked":0,"production":{"made":{},"consumed":{}},"power":{"generated_kw":0.0,"consumed_kw":0.0,"satisfaction":1.0,"networks":{}},"pollution":null}"#,
    "\n",
    r#"{"schema":3,"tick":900,"run":"alpha","kind":"machines","machines":{},"truncated":0}"#,
    "\n",
    r#"{"schema":3,"tick":1200,"run":"alpha","kind":"force","research":null,"techs_unlocked":0,"production":{"made":{},"consumed":{}},"power":{"generated_kw":0.0,"consumed_kw":0.0,"satisfaction":1.0,"networks":{}},"pollution":null}"#,
    "\n",
);

#[tokio::test]
async fn samples_can_be_sliced_by_tick_and_kind() {
    let ws = workspace("slice");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    std::fs::write(ws.join("runs/alpha/samples.jsonl"), SAMPLES).unwrap();
    let st = || state_with_workspace(&ws);
    let (_, all) = get_json(st(), "/api/v1/runs/alpha/samples").await;
    assert_eq!(all["samples"].as_array().unwrap().len(), 4);
    let (_, window) = get_json(st(), "/api/v1/runs/alpha/samples?from=700&to=1200").await;
    let ticks: Vec<u64> = window["samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["tick"].as_u64().unwrap())
        .collect();
    assert_eq!(ticks, vec![900, 1200], "inclusive bounds");
    let (_, force) = get_json(st(), "/api/v1/runs/alpha/samples?kind=force").await;
    assert_eq!(force["samples"].as_array().unwrap().len(), 2);
    assert!(
        force["samples"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["kind"] == "force")
    );
    let (_, both) = get_json(st(), "/api/v1/runs/alpha/samples?kind=force&to=600").await;
    assert_eq!(both["samples"].as_array().unwrap().len(), 1);
}

const MAP_LINES: &str = concat!(
    r#"{"tick":10,"kind":"keyframe","bounds":{"left":0,"top":0,"right":1,"bottom":1},"game":[],"model":[],"divergence":[]}"#,
    "\n",
    r#"{"tick":500,"kind":"placed","bot":1,"intent":{"name":"stone-furnace","position":{"x":1,"y":2},"direction":0},"actual":{"name":"stone-furnace","position":{"x":1,"y":2},"direction":0},"drift":null}"#,
    "\n",
    r#"{"tick":900,"kind":"placed","bot":2,"intent":{"name":"wooden-chest","position":{"x":3,"y":2},"direction":0},"actual":{"name":"wooden-chest","position":{"x":3,"y":2},"direction":0},"drift":null}"#,
    "\n",
);

#[tokio::test]
async fn the_map_can_be_sliced_by_tick() {
    let ws = workspace("mapslice");
    seed_run(&ws, "alpha", MILESTONES, Some(MANIFEST), None);
    std::fs::write(ws.join("runs/alpha/map.jsonl"), MAP_LINES).unwrap();
    let (_, body) = get_json(
        state_with_workspace(&ws),
        "/api/v1/runs/alpha/map?from=100&to=600",
    )
    .await;
    let ticks: Vec<u64> = body["map"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["tick"].as_u64().unwrap())
        .collect();
    assert_eq!(ticks, vec![500]);
}
