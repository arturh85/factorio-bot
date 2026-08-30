use crate::error::ErrorResponse;
use crate::extract::ApiJson;
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RconBody {
    pub command: String,
}

/// Sends a raw RCON command to the running Factorio server
#[utoipa::path(
    post,
    path = "/api/v1/rcon",
    tag = "Admin",
    request_body = RconBody,
    responses(
        (status = 204),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn send_rcon(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<RconBody>,
) -> Result<StatusCode, ErrorResponse> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    instance
        .rcon
        .send(&body.command)
        .await
        .map_err(ErrorResponse::from)?;
    Ok(StatusCode::NO_CONTENT)
}
