//! Script execution over HTTP, and the job history it produces.
//!
//! This is the endpoint that makes the Lua interpreter reachable from the
//! network, so the order of its checks is part of its contract rather than an
//! implementation detail. See [`post_execute`].

use crate::error::{ApiResult, ErrorResponse};
use crate::extract::ApiJson;
use crate::jobs::{Job, JobEvent, JobHandle, JobId, JobStatus};
use crate::manage::scripts::scripts_root;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_scripting::{OutputSink, Stream};
// `Stream` above is the script's stdout/stderr discriminant, so the async
// trait of the same name is aliased rather than shadowing it.
use futures_util::stream::Stream as EventStream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::{broadcast, watch};
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};
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

/// The wire form of a job event.
///
/// [`JobEvent`] itself cannot derive `Serialize`: its [`Stream`] comes from
/// `factorio-bot-scripting`, which has no dependencies at all -- that crate is
/// a leaf on purpose, and adding serde to it to satisfy a transport concern
/// would put the wire format's tail in the wrong crate.
///
/// `untagged`, because SSE splits a tagged union across two lines already: the
/// variant name goes on the `event:` line (see [`WireEvent::name`]) and only
/// the payload belongs in `data:`. The default external tagging would repeat
/// the name inside the payload, so a browser handler registered for
/// `output` would receive `{"output":{...}}` instead of `{...}`.
#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
enum WireEvent<'a> {
    Output {
        stream: &'a str,
        text: &'a str,
    },
    Finished {
        status: JobStatus,
    },
    /// How many messages a subscriber that fell behind missed. Rendered as a
    /// visible gap rather than dropped: silently short-changing a reader is
    /// worse than telling it the transcript is incomplete.
    Lagged {
        skipped: u64,
    },
}

impl WireEvent<'_> {
    /// The SSE `event:` name, which is what a browser's `addEventListener`
    /// switches on.
    fn name(&self) -> &'static str {
        match self {
            WireEvent::Output { .. } => "output",
            WireEvent::Finished { .. } => "finished",
            WireEvent::Lagged { .. } => "lagged",
        }
    }

    fn into_sse(self) -> Event {
        let name = self.name();
        // `json_data` fails only if serialization fails, and every field here
        // is a string or a `u64`. It is still not unwrapped: the release
        // profile sets `panic = "abort"`, so an impossible branch that panics
        // is an impossible branch that kills the server.
        Event::default()
            .event(name)
            .json_data(&self)
            .unwrap_or_else(|_| Event::default().event(name).data("{}"))
    }
}

fn stream_name(stream: Stream) -> &'static str {
    match stream {
        Stream::Stdout => "stdout",
        Stream::Stderr => "stderr",
    }
}

/// The events replayed to a subscriber that attached mid-run, or after the run
/// was over.
///
/// stdout and stderr are two separate buffers in a [`Job`], so their relative
/// interleaving is not recoverable here -- the backlog is stdout then stderr.
/// Live events, which arrive one at a time, keep their real order.
fn backlog(job: &Job) -> Vec<Event> {
    let lines = std::iter::empty()
        .chain(job.stdout.lines().map(|text| (Stream::Stdout, text)))
        .chain(job.stderr.lines().map(|text| (Stream::Stderr, text)));
    let mut events: Vec<Event> = lines
        .map(|(stream, text)| {
            WireEvent::Output {
                stream: stream_name(stream),
                text,
            }
            .into_sse()
        })
        .collect();
    if job.status != JobStatus::Running {
        events.push(WireEvent::Finished { status: job.status }.into_sse());
    }
    events
}

/// Maps one broadcast item to an SSE event, and says whether it terminates the
/// stream.
fn live_event(item: Result<JobEvent, BroadcastStreamRecvError>) -> (Event, bool) {
    match item {
        Ok(JobEvent::Output { stream, text }) => (
            WireEvent::Output {
                stream: stream_name(stream),
                text: &text,
            }
            .into_sse(),
            false,
        ),
        Ok(JobEvent::Finished { status }) => (WireEvent::Finished { status }.into_sse(), true),
        // Not filtered away. `BroadcastStreamRecvError` has exactly one
        // variant, and dropping it is the tidy-looking edit that deletes the
        // gap indicator the reader is meant to see -- the transcript would
        // silently skip `skipped` lines with nothing marking the hole.
        Err(BroadcastStreamRecvError::Lagged(skipped)) => {
            (WireEvent::Lagged { skipped }.into_sse(), false)
        }
    }
}

/// The body of `GET /api/v1/jobs/{id}/events`: backlog, then live events.
///
/// Ends on three things, and the first two are why this is a function rather
/// than a chain inlined into the handler -- each is separately testable:
///
/// 1. the terminal [`JobEvent::Finished`], *inclusively*: the event is
///    delivered and then the stream ends. Not merely `take_while`, which stops
///    at the first item failing the predicate and so would need one *further*
///    event to arrive before it could notice -- an event that, for a job that
///    has just finished, never comes;
/// 2. `shutdown`, so that a stream for a job which never finishes cannot hold
///    axum's unbounded graceful drain open;
/// 3. the channel closing, which [`crate::jobs::JobRegistry::complete`] causes
///    by dropping the sender.
fn job_event_stream(
    job: &Job,
    receiver: Option<broadcast::Receiver<JobEvent>>,
    mut shutdown: watch::Receiver<bool>,
) -> impl EventStream<Item = Result<Event, Infallible>> + Send + 'static {
    // `iter` over an `Option` is 0 or 1 receivers, which is how the
    // finished-job case (no live channel left) and the running case share one
    // stream type without boxing a branch.
    let live = futures_util::stream::iter(receiver)
        .flat_map(BroadcastStream::new)
        .map(live_event)
        .take_until(async move {
            let _ = shutdown.wait_for(|shutting_down| *shutting_down).await;
        });
    let live = futures_util::stream::unfold(Some(Box::pin(live)), |state| async move {
        let mut live = state?;
        let (event, terminal) = live.next().await?;
        Some((event, if terminal { None } else { Some(live) }))
    });
    futures_util::stream::iter(backlog(job)).chain(live).map(Ok)
}

/// Streams one script run's output as Server-Sent Events
///
/// Carries the script's output only -- not the Factorio server process's
/// stdout, which belongs to the instance rather than to any one job.
///
/// A subscriber that attaches late is not punished for it: the job's buffered
/// output is replayed first, and a job that already finished gets that
/// backlog, a `finished` event and end-of-stream. That case is the common one
/// -- a fast script is over before a browser can open the stream -- and it is
/// why the handler distinguishes "no such job" from "the job's channel was
/// pruned when it completed" via [`crate::jobs::JobRegistry::attach`]. Reading
/// a missing subscription as `404` would fail almost every real request.
#[utoipa::path(
    get,
    path = "/api/v1/jobs/{id}/events",
    tag = "Admin",
    params(("id" = String, Path, description = "Job id, as returned by the execute endpoint")),
    responses(
        (
            status = 200,
            description = "Server-Sent Events: `output`, `lagged`, then a terminal `finished`",
            content_type = "text/event-stream"
        ),
        (status = 404, body = crate::error::ErrorResponse),
    )
)]
pub async fn job_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Sse<impl EventStream<Item = Result<Event, Infallible>>>, ErrorResponse> {
    let not_found = || ErrorResponse::not_found(format!("no such job: {id}"));
    let job_id: JobId = id.parse().map_err(|_| not_found())?;
    let (job, receiver) = state.jobs.attach(job_id).ok_or_else(not_found)?;
    let stream = job_event_stream(&job, receiver, state.jobs.shutdown_signal());
    // Without a keep-alive a proxy is free to drop a connection that has been
    // idle for its timeout, and a long-running script can easily print nothing
    // for minutes.
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobRegistry;
    use axum::response::IntoResponse;
    use std::time::Duration;

    /// Renders a stream exactly as the handler would, so these tests read the
    /// same bytes a client does.
    async fn collect(
        stream: impl EventStream<Item = Result<Event, Infallible>> + Send + 'static,
    ) -> String {
        let bytes = axum::body::to_bytes(Sse::new(stream).into_response().into_body(), usize::MAX)
            .await
            .expect("the body is readable");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// The inclusive terminator, isolated from the channel closing.
    ///
    /// The integration tests cannot see this on its own: the registry drops
    /// the sender when a job completes, so there the stream would end anyway.
    /// Here the sender is deliberately *held open* after the terminal event,
    /// which is the only shape in which the terminator is the thing doing the
    /// work -- delete it and this test hangs, then fails on the timeout below.
    #[tokio::test]
    async fn the_stream_ends_at_the_terminal_event_even_while_the_sender_stays_open() {
        let (sender, receiver) = broadcast::channel(16);
        let job = Job {
            id: JobId(1),
            script: None,
            status: JobStatus::Running,
            started_at_ms: 0,
            finished_at_ms: None,
            stdout: String::new(),
            stderr: String::new(),
            error: None,
        };
        let (_never_fires, shutdown) = watch::channel(false);
        let stream = job_event_stream(&job, Some(receiver), shutdown);

        sender
            .send(JobEvent::Output {
                stream: Stream::Stdout,
                text: "one".into(),
            })
            .expect("a subscriber exists");
        sender
            .send(JobEvent::Finished {
                status: JobStatus::Succeeded,
            })
            .expect("a subscriber exists");

        let body = tokio::time::timeout(Duration::from_secs(5), collect(stream))
            .await
            .expect("the stream must end at the terminal event, not wait for the sender to drop");
        // `_sender` is still alive here on purpose: dropping it would end the
        // stream for the other reason and make this test prove nothing.
        drop(sender);
        assert!(body.contains("event: finished"), "body was {body:?}");
        assert!(body.contains("one"), "body was {body:?}");
    }

    /// The shutdown bound, isolated from the grace period that
    /// `tests/shutdown.rs` measures: a stream for a job that never finishes
    /// must end when the signal fires, with nothing else ending it.
    #[tokio::test]
    async fn a_stream_for_a_job_that_never_finishes_ends_at_shutdown() {
        let registry = JobRegistry::new(8);
        let handle = registry
            .try_start(Some("forever.lua".into()))
            .expect("start");
        handle.line(Stream::Stdout, "still going");
        let (job, receiver) = registry.attach(handle.id()).expect("the job exists");
        let stream = job_event_stream(&job, receiver, registry.shutdown_signal());
        registry.shutdown();

        let body = tokio::time::timeout(Duration::from_secs(5), collect(stream))
            .await
            .expect("a stream must not outlive the shutdown signal");
        assert!(
            body.contains("still going"),
            "the backlog is still owed to the subscriber: {body:?}"
        );
        assert!(
            !body.contains("event: finished"),
            "the job never finished, so nothing may claim it did: {body:?}"
        );
        // Held to the end: dropping it would complete the job and close the
        // channel, which is the other way this stream could have ended.
        drop(handle);
    }
}
