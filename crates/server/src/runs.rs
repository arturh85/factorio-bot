//! Durable run archives: list them, open one, seek through it.
//!
//! A run lives at `<workspace>/runs/<id>/` and is written by
//! `factorio_bot_core::record`. These routes read runs that have already
//! finished -- or that crashed, which are the ones worth opening.
//!
//! Every route treats an unfinished run as a first-class case. A run with no
//! `manifest.json` never reached `finish`; it still has its events, its samples
//! and whatever splits it got through, and refusing to serve it would hide
//! exactly the runs someone is trying to debug.

use crate::error::ErrorResponse;
use crate::manage::video::{CacheFor, VideoTicksResponse, serve_video_file};
use crate::manage::workspace_root;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, Request, State};
use axum::response::Response;
use factorio_bot_core::record::map::{MapRecord, read_map};
use factorio_bot_core::record::provenance::{Provenance, read_provenance};
use factorio_bot_core::record::video::{
    TICKS_FILE, VIDEO_DIR, VideoManifest, clock::read_tick_samples, read_video_dir,
};
use factorio_bot_core::record::{
    Event, Lane, Manifest, Sample, SampleKind, Split, derive_lanes, derive_splits, read_events,
    read_samples,
};
use factorio_bot_core::scripts::resolve_script_path;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use utoipa::ToSchema;

/// One run, as it appears in a listing.
///
/// Every field except `run_id` and `finished` is nullable, because an
/// unfinished run genuinely does not have them. They are present-and-null
/// rather than omitted: a caller must be able to tell "this run never finished"
/// from "this build of the API does not report outcomes".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RunSummary {
    pub run_id: String,
    /// Whether the run reached `finish`. False means crashed or still going --
    /// this endpoint cannot tell those apart, and does not pretend to.
    pub finished: bool,
    pub started_unix: Option<u64>,
    pub finished_unix: Option<u64>,
    pub outcome: Option<String>,
    pub elapsed_ticks: Option<u64>,
    pub events: Option<usize>,
    pub splits: Option<usize>,
    /// Archived sample lines. `None` for an unfinished run; `Some(0)` for a
    /// finished run with no samples file.
    pub samples: Option<usize>,
    /// Lines in `map.jsonl`, same convention.
    pub map: Option<usize>,
    /// Ticks of the run's span the samples do NOT cover (see `Manifest`).
    /// `None` when the run has no samples at all or never finished.
    pub samples_lag_ticks: Option<u64>,
}

impl RunSummary {
    fn unfinished(run_id: String) -> Self {
        Self {
            run_id,
            finished: false,
            started_unix: None,
            finished_unix: None,
            outcome: None,
            elapsed_ticks: None,
            events: None,
            splits: None,
            samples: None,
            map: None,
            samples_lag_ticks: None,
        }
    }

    fn from_manifest(manifest: Manifest) -> Self {
        Self {
            run_id: manifest.run_id,
            finished: true,
            started_unix: Some(manifest.started_unix),
            finished_unix: manifest.finished_unix,
            outcome: manifest.outcome,
            elapsed_ticks: manifest.elapsed_ticks,
            events: Some(manifest.events),
            splits: Some(manifest.splits),
            samples: Some(manifest.samples),
            map: Some(manifest.map),
            samples_lag_ticks: manifest.samples_lag_ticks,
        }
    }
}

/// `GET /api/v1/runs` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RunsResponse {
    /// Newest first, by start time; runs with no manifest sort last, since
    /// nothing on disk says when they began.
    pub runs: Vec<RunSummary>,
}

/// `GET /api/v1/runs/{id}` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RunDetail {
    pub summary: RunSummary,
    /// From `splits.json` when the run finished, otherwise derived from the
    /// events on the fly -- so a crashed run still shows the milestones it got
    /// through.
    pub splits: Vec<Split>,
}

/// `GET /api/v1/runs/{id}/events` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EventsResponse {
    pub events: Vec<Event>,
    /// Lines that did not parse -- in practice the truncated last line of a
    /// crashed run. Reported rather than swallowed.
    pub skipped: usize,
}

/// `GET /api/v1/runs/{id}/lanes` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RunLanesResponse {
    /// What each bot did, in dispatch order.
    pub lanes: Vec<Lane>,
}

/// `GET /api/v1/runs/{id}/samples` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RunSamplesResponse {
    pub samples: Vec<Sample>,
    /// Lines that did not parse -- in practice `bots = {}` serialising to
    /// `"{}"` rather than `"[]"` when no player is connected. Reported rather
    /// than swallowed, matching `EventsResponse.skipped`.
    pub skipped: usize,
}

/// `GET /api/v1/runs/{id}/map` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RunMapResponse {
    pub map: Vec<MapRecord>,
    /// Lines that did not parse. Reported rather than swallowed, matching
    /// `EventsResponse.skipped`.
    pub skipped: usize,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct EventFilter {
    /// Return only events of this `kind`.
    pub kind: Option<String>,
}

/// Narrows a stream to a tick window. Both bounds inclusive; either may be
/// absent. The whole file is still read -- the record is line-oriented and
/// has no index -- so this saves the wire and the browser, not the disk.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct TickWindow {
    /// First tick to include.
    pub from: Option<u64>,
    /// Last tick to include.
    pub to: Option<u64>,
}

impl TickWindow {
    fn contains(&self, tick: u64) -> bool {
        self.from.is_none_or(|f| tick >= f) && self.to.is_none_or(|t| tick <= t)
    }
}

/// `TickWindow` plus the sample `kind` (`bots`, `force`, `machines`).
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct SampleFilter {
    pub from: Option<u64>,
    pub to: Option<u64>,
    /// Return only samples of this `kind`.
    pub kind: Option<String>,
}

/// The wire name of a sample's kind, without a second serialisation: the
/// enum's `#[serde(tag = "kind")]` is the contract, and matching the variants
/// here keeps this in step with it at compile time.
fn sample_kind(sample: &Sample) -> &'static str {
    match sample.kind {
        SampleKind::Bots { .. } => "bots",
        SampleKind::Force { .. } => "force",
        SampleKind::Machines { .. } => "machines",
        SampleKind::Unknown => "unknown",
    }
}

async fn runs_root(state: &AppState) -> Result<PathBuf, ErrorResponse> {
    Ok(workspace_root(state).await?.join("runs"))
}

/// Resolves `<runs>/<id>`, refusing any id that would leave the runs
/// directory.
fn run_dir(runs: &std::path::Path, id: &str) -> Result<PathBuf, ErrorResponse> {
    let root = std::fs::canonicalize(runs)
        .map_err(|_| ErrorResponse::not_found("no runs directory yet".to_string()))?;
    let dir = resolve_script_path(&root, id).map_err(ErrorResponse::from)?;
    if !dir.is_dir() {
        return Err(ErrorResponse::not_found(format!("no such run: {id}")));
    }
    Ok(dir)
}

fn read_manifest(dir: &std::path::Path) -> Option<Manifest> {
    let bytes = std::fs::read(dir.join("manifest.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn summary_for(dir: &std::path::Path, id: &str) -> RunSummary {
    read_manifest(dir).map_or_else(
        || RunSummary::unfinished(id.to_string()),
        RunSummary::from_manifest,
    )
}

/// When a run began, for *ordering only*.
///
/// A run with no manifest has no recorded start time, and `started_unix` stays
/// null because that is the truth. But it still has to sort somewhere, and
/// putting every unfinished run at the bottom buries exactly the ones worth
/// opening -- a run that crashed is usually the newest and the most
/// interesting.
///
/// Run ids are minted here as `run-<unix seconds>-<sub-second>`, so the prefix
/// is a start time we produced ourselves. Parsing it is a coupling to our own
/// id format, which is why it is confined to this function and why a failure
/// to parse falls back to zero rather than guessing.
fn sort_key(run: &RunSummary) -> u64 {
    run.started_unix.unwrap_or_else(|| {
        run.run_id
            .strip_prefix("run-")
            .and_then(|rest| rest.split('-').next())
            .and_then(|secs| secs.parse().ok())
            .unwrap_or(0)
    })
}

/// Lists archived runs.
#[utoipa::path(
    get,
    path = "/api/v1/runs",
    tag = "Runs",
    responses(
        (status = 200, body = RunsResponse, description = "archived runs, newest first"),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn list_runs(State(state): State<AppState>) -> Result<Json<RunsResponse>, ErrorResponse> {
    let root = runs_root(&state).await?;
    // A workspace that has never recorded a run answers with an empty list, not
    // a 404: "no runs yet" is a state, not a missing resource.
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(Json(RunsResponse { runs: Vec::new() }));
    };

    let mut runs: Vec<RunSummary> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_owned();
            Some(summary_for(&e.path(), &id))
        })
        .collect();

    runs.sort_by(|a, b| {
        sort_key(b)
            .cmp(&sort_key(a))
            .then_with(|| b.run_id.cmp(&a.run_id))
    });
    Ok(Json(RunsResponse { runs }))
}

/// One run's summary and splits.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id")),
    responses(
        (status = 200, body = RunDetail),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunDetail>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    let summary = summary_for(&dir, &id);

    let splits = match std::fs::read(dir.join("splits.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        // No splits.json means the run never finished. Deriving them from the
        // events costs a read and is the difference between a crashed run you
        // can see into and one that looks empty.
        Err(_) => read_events(&dir.join("events.jsonl"))
            .map(|read| derive_splits(&read.events))
            .unwrap_or_default(),
    };
    Ok(Json(RunDetail { summary, splits }))
}

/// What a run was launched with -- the fields that decide whether two runs
/// may be compared at all.
///
/// A 404, not an empty object, when `provenance.json` is missing: the file is
/// written at run *start* since 2026-09-06, so its absence means an older run
/// and the client must show "not captured", never a default.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/provenance",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id")),
    responses(
        (status = 200, body = Provenance),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_provenance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Provenance>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    read_provenance(&dir)
        .map(Json)
        .ok_or_else(|| ErrorResponse::not_found(format!("run {id} recorded no provenance")))
}

/// A run's event log.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/events",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id"), EventFilter),
    responses(
        (status = 200, body = EventsResponse),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(filter): Query<EventFilter>,
) -> Result<Json<EventsResponse>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    let read = read_events(&dir.join("events.jsonl"))
        .map_err(|err| ErrorResponse::not_found(format!("no event log for {id}: {err}")))?;

    let events = match filter.kind {
        None => read.events,
        Some(kind) => read
            .events
            .into_iter()
            .filter(|event| {
                serde_json::to_value(event)
                    .ok()
                    .and_then(|value| value.get("kind")?.as_str().map(str::to_owned))
                    .is_some_and(|k| k == kind)
            })
            .collect(),
    };
    Ok(Json(EventsResponse {
        events,
        skipped: read.skipped,
    }))
}

/// What each bot did, derived from the run's event log.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/lanes",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id")),
    responses(
        (status = 200, body = RunLanesResponse),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_lanes(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunLanesResponse>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    // A run with no action events -- planning only, or recorded before actions
    // were captured -- has no lanes. Empty list, not an error.
    let lanes = read_events(&dir.join("events.jsonl"))
        .map(|read| derive_lanes(&read.events))
        .unwrap_or_default();
    Ok(Json(RunLanesResponse { lanes }))
}

/// A run's world-state samples.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/samples",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id"), SampleFilter),
    responses(
        (status = 200, body = RunSamplesResponse),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_samples(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(filter): Query<SampleFilter>,
) -> Result<Json<RunSamplesResponse>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    let path = dir.join("samples.jsonl");
    // A run recorded before sampling existed -- or one for which the mod
    // never captured any -- is not an error; it just has none.
    if !path.exists() {
        return Ok(Json(RunSamplesResponse {
            samples: Vec::new(),
            skipped: 0,
        }));
    }
    let read = read_samples(&path)
        .map_err(|err| ErrorResponse::internal(format!("failed to read samples: {err}")))?;
    let window = TickWindow {
        from: filter.from,
        to: filter.to,
    };
    let samples = read
        .samples
        .into_iter()
        .filter(|s| window.contains(s.tick))
        .filter(|s| filter.kind.as_deref().is_none_or(|k| sample_kind(s) == k))
        .collect();
    Ok(Json(RunSamplesResponse {
        samples,
        skipped: read.skipped,
    }))
}

/// A run's entity map: what got built, and whether the game agreed.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/map",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id"), TickWindow),
    responses(
        (status = 200, body = RunMapResponse),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_map(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(window): Query<TickWindow>,
) -> Result<Json<RunMapResponse>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    let path = dir.join("map.jsonl");
    // A run recorded before this feature existed -- or one that never placed
    // anything -- is not an error; it just has no map.
    if !path.exists() {
        return Ok(Json(RunMapResponse {
            map: Vec::new(),
            skipped: 0,
        }));
    }
    let read = read_map(&path)
        .map_err(|err| ErrorResponse::internal(format!("failed to read map: {err}")))?;
    let map = read
        .records
        .into_iter()
        .filter(|r| window.contains(r.tick))
        .collect();
    Ok(Json(RunMapResponse {
        map,
        skipped: read.skipped,
    }))
}

/// A run's archived video manifest.
///
/// A run recorded before video existed -- or one that never asked for it, which
/// is every run by default -- answers an empty manifest, not a 404. Video is
/// opt-in, so "there is none" is the ordinary case and must be distinguishable
/// from a route that is not there.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/video",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id")),
    responses(
        (status = 200, body = VideoManifest),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_video(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<VideoManifest>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    Ok(Json(read_video_dir(&dir.join(VIDEO_DIR))))
}

/// A run's archived video clock.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/video/ticks",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id")),
    responses(
        (status = 200, body = VideoTicksResponse),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_video_ticks(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<VideoTicksResponse>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    let read = read_tick_samples(&dir.join(VIDEO_DIR).join(TICKS_FILE))
        .map_err(|err| ErrorResponse::internal(format!("failed to read the video clock: {err}")))?;
    Ok(Json(VideoTicksResponse {
        samples: read.samples,
        skipped: read.skipped,
    }))
}

/// One archived recording's bytes, with range support.
///
/// Cached `immutable`, unlike the live route: this run is over, so these bytes
/// never change. The live `<workspace>/video/video.mp4` is overwritten by the
/// next run at the same URL and must not be.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/video/file",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id")),
    responses(
        (status = 200, content_type = "video/mp4", description = "the whole recording"),
        (status = 206, content_type = "video/mp4", description = "a byte range, for seeking"),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_video_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?.join(VIDEO_DIR);
    let name = read_video_dir(&dir)
        .video
        .map(|record| record.file)
        .ok_or_else(|| ErrorResponse::not_found(format!("run {id} recorded no video")))?;
    serve_video_file(&dir, &name, request, CacheFor::Ever).await
}

pub fn router() -> utoipa_axum::router::OpenApiRouter<AppState> {
    use utoipa_axum::routes;
    utoipa_axum::router::OpenApiRouter::new()
        .routes(routes!(list_runs))
        .routes(routes!(get_run))
        .routes(routes!(get_run_provenance))
        .routes(routes!(get_run_events))
        .routes(routes!(get_run_lanes))
        .routes(routes!(get_run_samples))
        .routes(routes!(get_run_map))
        .routes(routes!(get_run_video))
        .routes(routes!(get_run_video_ticks))
        .routes(routes!(get_run_video_file))
}
