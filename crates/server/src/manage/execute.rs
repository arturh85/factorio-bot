//! Script execution over HTTP, and the job history it produces.
//!
//! This is the endpoint that makes the Lua interpreter reachable from the
//! network, so the order of its checks is part of its contract rather than an
//! implementation detail. See [`post_execute`].

use crate::error::{ApiResult, ErrorResponse};
use crate::extract::ApiJson;
use crate::jobs::{Job, JobHandle, JobId};
use crate::manage::scripts::scripts_root;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_scripting::{OutputSink, Stream};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use utoipa::ToSchema;

/// The language assumed for inline `code` when the caller does not say.
const DEFAULT_LANGUAGE: &str = "lua";

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExecuteRequest {
    /// Script to run, relative to the scripts root. Mutually exclusive with `code`.
    pub path: Option<String>,
    /// Inline code to run. Mutually exclusive with `path`.
    pub code: Option<String>,
    /// Defaults to `"lua"` when `code` is given; ignored when `path` is.
    pub language: Option<String>,
    /// Defaults to the configured `factorio.client_count`.
    pub bot_count: Option<u8>,
}

/// The `202` body: the job to poll or stream from.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExecuteAccepted {
    pub job_id: JobId,
}

/// What the caller asked to run, once the body has been validated.
enum ScriptSource {
    File(String),
    Code { language: String, code: String },
}

/// Bridges a running script's output into the job registry, and hands the
/// handle back when the run is over.
///
/// [`JobHandle`] is already an [`OutputSink`], so it looks like it could be
/// passed to the interpreter directly -- but the interpreter wants an
/// `Arc<dyn OutputSink>` while [`JobHandle::finish`] consumes `self`, and a
/// value inside an `Arc` can only be taken back out once every other clone has
/// been dropped. Relying on the interpreter to have dropped its clone by the
/// time the call returns is exactly the kind of "should hold" reasoning that
/// loses a job's outcome silently on the day it stops holding: `finish` would
/// never be called, and the job would be reported as the generic "ended
/// without reporting a result" instead of its real result. Parking the handle
/// behind a mutex makes taking it back unconditional.
struct JobSink(Mutex<Option<JobHandle>>);

impl JobSink {
    fn new(handle: JobHandle) -> Self {
        JobSink(Mutex::new(Some(handle)))
    }

    /// A poisoned mutex degrades rather than aborting, for the same reason
    /// [`crate::jobs::JobRegistry`] does it: the release profile sets
    /// `panic = "abort"`, so unwrapping a lock that a panicking script host
    /// poisoned would take the whole server down with it.
    fn lock(&self) -> MutexGuard<'_, Option<JobHandle>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records the run's outcome and frees the execution slot.
    ///
    /// A second call does nothing: the handle is gone, and the registry would
    /// refuse to overwrite a completed job's outcome anyway.
    fn finish(&self, outcome: miette::Result<(String, String)>) {
        if let Some(handle) = self.lock().take() {
            handle.finish(outcome);
        }
    }
}

impl OutputSink for JobSink {
    fn line(&self, stream: Stream, text: &str) {
        if let Some(handle) = self.lock().as_ref() {
            handle.line(stream, text);
        }
    }
}

/// Starts a script and answers immediately with the job that runs it
///
/// The order of the checks below is the contract, not an accident:
///
/// 1. the body (`400`) -- nothing else is knowable until it parses;
/// 2. a running Factorio instance (`503`) -- the server is fine, the game is
///    not, which is not the caller's fault;
/// 3. the script itself (`404` missing, `400` outside the scripts root);
/// 4. and only then the single execution slot (`409`).
///
/// Taking the slot before a check that can still fail would leave it held by a
/// request that never runs, and the slot is single: every later execution would
/// answer `409` until the process restarts. [`JobHandle`]'s `Drop` frees it on
/// any exit path once it *is* taken, so the happy path is not what keeps this
/// safe -- but not taking it early is cheaper than relying on that.
#[utoipa::path(
    post,
    path = "/api/v1/scripts/execute",
    tag = "Admin",
    request_body = ExecuteRequest,
    responses(
        (status = 202, description = "Execution started", body = ExecuteAccepted),
        (status = 400, description = "Bad request", body = crate::error::ErrorResponse),
        (status = 404, description = "Script not found", body = crate::error::ErrorResponse),
        (status = 409, description = "A script is already running", body = crate::error::ErrorResponse),
        (status = 503, description = "No running Factorio instance", body = crate::error::ErrorResponse),
    )
)]
pub async fn post_execute(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ExecuteRequest>,
) -> Result<(StatusCode, Json<ExecuteAccepted>), ErrorResponse> {
    // 1. The body. "Exactly one of" is stated as a match over both fields so
    //    neither the both-given nor the neither-given case can be forgotten.
    let source = match (request.path, request.code) {
        (Some(path), None) => ScriptSource::File(path),
        (None, Some(code)) => ScriptSource::Code {
            language: request
                .language
                .unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned()),
            code,
        },
        (Some(_), Some(_)) => {
            return Err(ErrorResponse::bad_request(
                "exactly one of `path` or `code` may be given, not both",
            ))
        }
        (None, None) => {
            return Err(ErrorResponse::bad_request(
                "one of `path` or `code` is required",
            ))
        }
    };

    // 2. A running instance. `world` is separate from the instance itself
    //    because an instance can exist without one (a `--connect` session
    //    builds its own), and a script with no world to plan against cannot
    //    run either.
    let (world, rcon) = {
        let instance = state.instance.read().await;
        let instance = instance
            .as_ref()
            .ok_or_else(|| ErrorResponse::not_running("not started"))?;
        let world = instance
            .world
            .clone()
            .ok_or_else(|| ErrorResponse::not_running("the running instance has no world"))?;
        (world, instance.rcon.clone())
    };

    let bot_count = match request.bot_count {
        Some(bot_count) => bot_count,
        None => state.settings.read().await.factorio.client_count,
    };
    let scripts_root = scripts_root(&state).await?;

    // 3. The script. `resolve_script` is the same resolution the run itself
    //    performs, shared rather than re-implemented here: this endpoint
    //    answers 202 and runs the script detached, so by the time the run
    //    could report a bad path there is no status code left to put it in.
    let script = match &source {
        ScriptSource::File(path) => {
            factorio_bot_scripting_lua::resolve_script(&scripts_root, path)
                .map_err(ErrorResponse::from)?;
            Some(path.clone())
        }
        ScriptSource::Code { language, .. } => {
            if language != DEFAULT_LANGUAGE {
                return Err(ErrorResponse::bad_request(format!(
                    "unknown language: {language}"
                )));
            }
            None
        }
    };

    // 4. The slot.
    let handle = state
        .jobs
        .try_start(script)
        .map_err(ErrorResponse::already_running)?;
    let job_id = handle.id();

    spawn_run(&state, handle, world, rcon, scripts_root, source, bot_count);

    Ok((StatusCode::ACCEPTED, Json(ExecuteAccepted { job_id })))
}

/// Runs the script on a detached task.
///
/// The task holds a strong `Arc<JobRegistry>` of its own. [`JobHandle`] only
/// carries a `Weak` -- the registry is built with `Arc::new_cyclic`, so a
/// handle holding a strong reference back would be a cycle -- and
/// [`JobHandle::finish`] *silently does nothing* if that upgrade fails. The
/// registry lives in `AppState`, which axum drops together with the router at
/// shutdown, while this task keeps running: without a strong reference of its
/// own, a script still running at shutdown would finish and report its result
/// nowhere, with no error logged anywhere.
fn spawn_run(
    state: &AppState,
    handle: JobHandle,
    world: Arc<FactorioWorld>,
    rcon: Arc<FactorioRcon>,
    scripts_root: PathBuf,
    source: ScriptSource,
    bot_count: u8,
) {
    // Not `let _ = ...`: this binding is the whole point, and a name that reads
    // as deliberate is what stops a tidy-up from deleting it as unused.
    let registry_kept_alive_for_the_run = state.jobs.clone();
    let sink = Arc::new(JobSink::new(handle));
    let sink_for_run: Arc<dyn OutputSink> = sink.clone();
    tokio::spawn(async move {
        let _registry = registry_kept_alive_for_the_run;
        let mut planner = Planner::new(world, Some(rcon));
        let outcome = match source {
            ScriptSource::File(path) => factorio_bot_scripting_lua::run_script_file(
                &mut planner,
                &scripts_root,
                &path,
                bot_count,
                Some(sink_for_run),
            )
            .await
            .map_err(factorio_bot_scripting_lua::RunScriptError::into_report),
            ScriptSource::Code { language, code } => {
                factorio_bot_scripting_lua::run_script(
                    &mut planner,
                    &language,
                    &code,
                    &scripts_root,
                    bot_count,
                    Some(sink_for_run),
                )
                .await
            }
        };
        sink.finish(outcome);
    });
}

/// Lists every remembered script run, newest first
#[utoipa::path(
    get,
    path = "/api/v1/jobs",
    tag = "Admin",
    responses((status = 200, body = Vec<Job>))
)]
pub async fn list_jobs(State(state): State<AppState>) -> ApiResult<Vec<Job>> {
    Ok(Json(state.jobs.list()))
}

/// Reports one script run
///
/// An id that does not parse is a `404` rather than a `400`: from the caller's
/// side "job `abc`" and "job `9999`" are the same mistake, and there is nothing
/// useful to say about the difference.
#[utoipa::path(
    get,
    path = "/api/v1/jobs/{id}",
    tag = "Admin",
    params(("id" = String, Path, description = "Job id, as returned by the execute endpoint")),
    responses(
        (status = 200, body = Job),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn get_job(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Job> {
    let not_found = || ErrorResponse::not_found(format!("no such job: {id}"));
    let job_id: JobId = id.parse().map_err(|_| not_found())?;
    state.jobs.get(job_id).map(Json).ok_or_else(not_found)
}
