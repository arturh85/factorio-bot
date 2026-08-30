use crate::error::{ApiResult, ErrorResponse};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstanceStatus {
    pub started: bool,
    pub client_count: u8,
    pub server_port: Option<u16>,
    pub rcon_port: Option<u16>,
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
    let instance = state.instance.read().await;
    let status = match instance.as_ref() {
        Some(instance) => InstanceStatus {
            started: true,
            client_count: instance.client_count,
            server_port: instance.server_port,
            rcon_port: Some(instance.rcon_port),
        },
        None => InstanceStatus {
            started: false,
            client_count: 0,
            server_port: None,
            rcon_port: None,
        },
    };
    Ok(Json(status))
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
