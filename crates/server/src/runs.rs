//! Durable run archives: list them, open one, seek through it.
//!
//! A run lives at `<workspace>/runs/<id>/` and is written by
//! `factorio_bot_core::record`. Unlike `GET /api/v1/frames`, which reports what
//! is on disk for the *current* run, these routes read runs that have already
//! finished -- or that crashed, which are the ones worth opening.
//!
//! Every route treats an unfinished run as a first-class case. A run with no
//! `manifest.json` never reached `finish`; it still has its events, its frames
//! and whatever splits it got through, and refusing to serve it would hide
//! exactly the runs someone is trying to debug.

use crate::error::ErrorResponse;
use crate::manage::frames::workspace_root;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use factorio_bot_core::record::{
    ArchivedFrame, Event, Manifest, Split, derive_splits, read_events,
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
    pub frames: Option<usize>,
    pub splits: Option<usize>,
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
            frames: None,
            splits: None,
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
            frames: Some(manifest.frames),
            splits: Some(manifest.splits),
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

/// `GET /api/v1/runs/{id}/frames` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RunFramesResponse {
    pub frames: Vec<ArchivedFrame>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct EventFilter {
    /// Return only events of this `kind`.
    pub kind: Option<String>,
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
        b.started_unix
            .cmp(&a.started_unix)
            .then_with(|| a.run_id.cmp(&b.run_id))
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

/// A run's frame index.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/frames",
    tag = "Runs",
    params(("id" = String, Path, description = "the run id")),
    responses(
        (status = 200, body = RunFramesResponse),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_frames(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunFramesResponse>, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    // A planning-only run has no frames at all. Empty list, not 404.
    let frames = std::fs::read(dir.join("frames").join("index.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    Ok(Json(RunFramesResponse { frames }))
}

/// One archived frame's bytes.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/frames/{bot}/{name}",
    tag = "Runs",
    params(
        ("id" = String, Path, description = "the run id"),
        ("bot" = u8, Path, description = "the bot number, as reported by ArchivedFrame.bot"),
        ("name" = String, Path, description = "the frame filename, as reported by GET /api/v1/runs/{id}/frames"),
    ),
    responses(
        (status = 200, content_type = "image/jpeg", description = "the frame's JPEG bytes"),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_run_frame(
    State(state): State<AppState>,
    Path((id, bot, name)): Path<(String, u8, String)>,
) -> Result<Response, ErrorResponse> {
    let dir = run_dir(&runs_root(&state).await?, &id)?;
    let bot_dir = std::fs::canonicalize(dir.join("frames").join(bot.to_string()))
        .map_err(|_| ErrorResponse::not_found(format!("no frames for bot {bot} in run {id}")))?;
    let resolved = resolve_script_path(&bot_dir, &name).map_err(ErrorResponse::from)?;
    if !resolved.is_file() {
        return Err(ErrorResponse::not_found(format!("no such frame: {name}")));
    }
    let bytes = std::fs::read(&resolved)
        .map_err(|err| ErrorResponse::internal(format!("failed to read frame: {err}")))?;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg")),
            (
                header::CACHE_CONTROL,
                // An archived frame is immutable: its run is over and a tick
                // never recurs. A scrubber re-requests the same frames
                // constantly, and this header is what makes that usable.
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        bytes,
    )
        .into_response())
}

pub fn router() -> utoipa_axum::router::OpenApiRouter<AppState> {
    use utoipa_axum::routes;
    utoipa_axum::router::OpenApiRouter::new()
        .routes(routes!(list_runs))
        .routes(routes!(get_run))
        .routes(routes!(get_run_events))
        .routes(routes!(get_run_frames))
        .routes(routes!(get_run_frame))
}
