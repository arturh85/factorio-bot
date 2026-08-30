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
        (status = 409, body = crate::error::ErrorResponse),
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
            (settings.factorio.clone(), params)
        };

        match FactorioInstance::start(&factorio_settings, params).await {
            Ok(started) => {
                // Publish the instance before clearing `starting`, for the
                // same reason the error arm publishes its message first: a
                // poller that catches `starting == false` with neither an
                // instance nor an error would report "not started, no
                // problem" for a start that actually succeeded.
                *state.instance.write().await = Some(started);
            }
            Err(err) => {
                tracing::error!("failed to start factorio instance: {err:?}");
                // Written *before* `starting` is cleared. The other order has
                // a window in which a poll sees `starting == false` and
                // `last_error == None` and concludes the start succeeded --
                // the failure would be invisible until the next attempt.
                *state.last_start_error.write().await = Some(format!("{err:?}"));
            }
        }
        // Released on every path, including the error path: leaving it set
        // would wedge the server into permanent 409s with nothing running.
        state.release_start_slot();
    });

    Ok((StatusCode::ACCEPTED, Json(StartAccepted { accepted: true })))
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
    let taken = state.instance.write().await.take();
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
