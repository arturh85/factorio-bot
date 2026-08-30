use crate::error::{ApiResult, ErrorResponse};
use crate::extract::ApiJson;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use factorio_bot_core::app_settings::AppSettings;

/// Returns the current application settings
#[utoipa::path(
    get,
    path = "/api/v1/settings",
    tag = "Admin",
    responses(
        (status = 200, body = AppSettings),
    )
)]
pub async fn get_settings(State(state): State<AppState>) -> ApiResult<AppSettings> {
    Ok(Json(state.settings.read().await.clone()))
}

/// Replaces the application settings and persists them
#[utoipa::path(
    put,
    path = "/api/v1/settings",
    tag = "Admin",
    request_body = AppSettings,
    responses(
        (status = 200, body = AppSettings),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 500, body = crate::error::ErrorResponse),
    )
)]
pub async fn put_settings(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<AppSettings>,
) -> ApiResult<AppSettings> {
    let mut settings = state.settings.write().await;
    *settings = body;
    // A failed write is the server's own disk problem, not a malformed
    // request: answer 500 rather than blaming the caller.
    AppSettings::save(state.settings_path.clone(), &settings)
        .map_err(|err| ErrorResponse::internal(format!("failed to save settings: {err}")))?;
    Ok(Json(settings.clone()))
}
