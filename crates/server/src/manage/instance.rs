use crate::error::{ApiResult, ErrorResponse};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use factorio_bot_core::process::process_control::{FactorioInstance, FactorioParams};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstanceStatus {
    pub started: bool,
    /// A start accepted by `POST /api/v1/instance/start` is still running.
    pub starting: bool,
    pub client_count: u8,
    pub server_port: Option<u16>,
    pub rcon_port: Option<u16>,
    /// Why the last start attempt failed, or `null`.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StartAccepted {
    pub accepted: bool,
}

/// Reports whether a Factorio instance is running
#[utoipa::path(
    get,
    path = "/api/v1/instance",
    tag = "Admin",
    responses(
        (status = 200, body = InstanceStatus),
    )
)]
pub async fn get_instance(State(state): State<AppState>) -> ApiResult<InstanceStatus> {
    let starting = state.starting.load(Ordering::SeqCst);
    let last_error = state.last_start_error.read().await.clone();
    let instance = state.instance.read().await;
    let status = match instance.as_ref() {
        Some(instance) => InstanceStatus {
            started: true,
            starting,
            client_count: instance.client_count,
            server_port: instance.server_port,
            rcon_port: Some(instance.rcon_port),
            last_error,
        },
        None => InstanceStatus {
            started: false,
            starting,
            client_count: 0,
            server_port: None,
            rcon_port: None,
            last_error,
        },
    };
    Ok(Json(status))
}

/// Starts a Factorio instance in the background.
///
/// Answers `202` and never blocks: `FactorioInstance::start` runs the
/// archive extraction on first use, which takes 8-10 minutes, and a browser
/// (or any proxy in front of it) would time out long before that. Progress is
/// observed through `GET /api/v1/instance`.
#[utoipa::path(
    post,
    path = "/api/v1/instance/start",
    tag = "Admin",
    responses(
        (status = 202, body = StartAccepted),
        // Both answered synchronously, before the start is spawned: a
        // `workspace_path` that is relative (400) or not valid UTF-8 (500).
        // Listed because a client generated from this spec otherwise has no
        // case for them -- and the OpenAPI guards in `tests/openapi.rs` check
        // paths and methods, so nothing else would have caught the omission.
        // `the_start_operation_documents_every_status_it_answers` does.
        (status = 400, body = crate::error::ErrorResponse),
        (status = 409, body = crate::error::ErrorResponse),
        (status = 500, body = crate::error::ErrorResponse),
    )
)]
pub async fn start_instance(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<StartAccepted>), ErrorResponse> {
    // A courtesy check, not a gate: two requests arriving together can both
    // pass this before either reaches the atomic claim below. It earns
    // its place only by giving the common single-request case the accurate
    // message ("already started" rather than "already starting").
    if state.instance.read().await.is_some() {
        return Err(ErrorResponse::conflict("instance already started"));
    }
    // One resolution rule for `workspace_path`, shared with the scripts routes
    // through `paths::resolve_workspace`: empty means the data-local
    // workspace, relative is refused. Without it this route handed the raw
    // settings string to `setup_factorio_instance`, which rejects an empty one
    // with "no workspace configured" -- so a browser could list and edit
    // scripts under the data-local workspace and then fail to start Factorio
    // in that same workspace.
    //
    // Resolved here rather than in the task so a misconfiguration answers on
    // the request that caused it, instead of surfacing minutes later in
    // `last_error`.
    let workspace_path = {
        let settings = state.settings.read().await;
        factorio_bot_core::paths::resolve_workspace(&settings.factorio.workspace_path)
            .map_err(|err| ErrorResponse::bad_request(err.to_string()))?
    };
    // `FactorioSettings::workspace_path` is a `str`, so a path that is not
    // UTF-8 cannot be handed on at all. Vanishingly unlikely and still not
    // worth a silent `to_string_lossy`, which would start Factorio in a
    // *different* directory than the one configured.
    //
    // Through `paths::workspace_to_string` rather than restated here: the two
    // settings loaders used to `to_string_lossy` the same value, so the same
    // configured workspace was refused by this route and mangled by them. One
    // rule, one policy -- refuse. Still a 500: a data-local directory the
    // caller cannot name in the request is the server's problem, not a
    // malformed request.
    let workspace_path = factorio_bot_core::paths::workspace_to_string(workspace_path)
        .map_err(|err| ErrorResponse::internal(err.to_string()))?;
    // *This* is what serialises concurrent starts: `AppState::claim_start_slot`
    // takes the slot in a single `compare_exchange`, where a read-then-write
    // pair would leave a window in which two requests both see `false` and
    // both spawn a start, and the loser's Factorio would die on the winner's
    // ports and lock files. Pinned by
    // `state::tests::the_start_slot_is_claimed_by_exactly_one_caller`.
    if !state.claim_start_slot() {
        return Err(ErrorResponse::conflict("instance is already starting"));
    }
    *state.last_start_error.write().await = None;
    // Captured before the start begins: `complete_start` publishes only if this
    // has not moved, so a stop pressed during the 8-10 minutes an extraction
    // takes is not undone by the start eventually succeeding.
    let stop_requests_before = state.stop_requests();

    // The whole state moves into the task: it outlives this request by minutes
    // and has to publish its result somewhere the next `GET /api/v1/instance`
    // will look.
    tokio::spawn(async move {
        let (factorio_settings, params) = {
            let settings = state.settings.read().await;
            let seed = settings.factorio.seed.to_string();
            let map_exchange_string = settings.factorio.map_exchange_string.to_string();
            let params = FactorioParams {
                client_count: settings.factorio.client_count,
                recreate: settings.factorio.recreate,
                seed: if seed.is_empty() { None } else { Some(seed) },
                map_exchange_string: if map_exchange_string.is_empty() {
                    None
                } else {
                    Some(map_exchange_string)
                },
                ..FactorioParams::default()
            };
            // The *resolved* workspace, not the configured one: this is the
            // single value that makes `setup_factorio_instance` agree with
            // every other route about where the workspace is.
            let mut factorio_settings = settings.factorio.clone();
            factorio_settings.workspace_path = std::borrow::Cow::Owned(workspace_path);
            (factorio_settings, params)
        };

        let started = FactorioInstance::start(&factorio_settings, params);
        complete_start(state, stop_requests_before, started).await;
    });

    Ok((StatusCode::ACCEPTED, Json(StartAccepted { accepted: true })))
}

/// Everything the spawned start does once `FactorioInstance::start` returns:
/// publish or discard the instance, record a failure, release the slot.
///
/// Split out of [`start_instance`] and generic over the start future so a test
/// can drive it with a Factorio that starts (`empty_factorio_instance`), which
/// no test can otherwise obtain -- the real one needs an installed game and
/// eight minutes. Without this seam the entire success path, including the
/// stop-during-start race below, is unreachable from the suite.
pub async fn complete_start(
    state: AppState,
    stop_requests_before: u64,
    start: impl std::future::Future<Output = miette::Result<FactorioInstance>>,
) {
    match start.await {
        Ok(started) => {
            // Publish before clearing `starting`, for the same reason the error
            // arm publishes its message first: a poller that catches
            // `starting == false` with neither an instance nor an error would
            // report "not started, no problem" for a start that succeeded.
            if let Some(orphan) = state
                .publish_started_instance(stop_requests_before, started)
                .await
            {
                // A stop arrived while this was starting. The user asked for
                // no Factorio, so it is not published -- but it is running,
                // and nothing else holds a handle to it, so this is the only
                // place it can be shut down. `last_error` stays clear: "not
                // started, no error" is the honest report of a start the user
                // cancelled.
                tracing::info!("start cancelled by a stop; shutting the new instance down again");
                if let Err(err) = orphan.stop() {
                    tracing::error!("failed to stop the cancelled instance: {err:?}");
                    *state.last_start_error.write().await =
                        Some(format!("failed to stop the cancelled instance: {err:?}"));
                }
            }
        }
        Err(err) => {
            tracing::error!("failed to start factorio instance: {err:?}");
            // Written *before* `starting` is cleared. The other order has a
            // window in which a poll sees `starting == false` and
            // `last_error == None` and concludes the start succeeded -- the
            // failure would be invisible until the next attempt.
            *state.last_start_error.write().await = Some(format!("{err:?}"));
        }
    }
    // Released on every path, including the error path: leaving it set would
    // wedge the server into permanent 409s with nothing running.
    state.release_start_slot();
}

/// Stops the running Factorio instance
#[utoipa::path(
    post,
    path = "/api/v1/instance/stop",
    tag = "Admin",
    responses(
        (status = 204),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 500, body = crate::error::ErrorResponse),
    )
)]
pub async fn stop_instance(State(state): State<AppState>) -> Result<StatusCode, ErrorResponse> {
    let taken = {
        let mut instance = state.instance.write().await;
        // Under the same lock the start publishes through, and *whether or not*
        // there is anything to take: a start that is still extracting has not
        // published yet, so the 400 below is the answer to "is one running"
        // while this is the answer to "did the user ask for one to stop".
        // Without it, Stop during a start looks like it worked and Factorio
        // appears minutes later.
        state.note_stop_request(&mut instance);
        instance.take()
    };
    match taken {
        // stop() consumes self and is synchronous; propagate its error rather
        // than unwrapping, which would abort the process under panic = "abort"
        Some(instance) => {
            // Killing the child processes failing is a server-side fault, not
            // the caller's: answer 500 rather than 400.
            instance.stop().map_err(|err| {
                ErrorResponse::internal(format!("failed to stop instance: {err}"))
            })?;
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err(ErrorResponse::not_started()),
    }
}
