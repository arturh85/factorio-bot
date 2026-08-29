use crate::error::{ApiResult, ErrorResponse};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::Json;
use factorio_bot_core::types::{AreaFilter, FactorioEntity};
use serde::Deserialize;
use utoipa::IntoParams;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(default)]
pub struct FindEntitiesParams {
    pub area: Option<String>,
    pub position: Option<String>,
    pub radius: Option<f64>,
    pub name: Option<String>,
    pub entity_type: Option<String>,
}

/// Finds entities in given area/radius
#[utoipa::path(
    get,
    path = "/api/v1/game/find-entities",
    tag = "Query",
    params(FindEntitiesParams),
    responses(
        (status = 200, body = Vec<serde_json::Value>, description = "List of entities matching the filter (serialized FactorioEntity)"),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn find_entities(
    State(state): State<AppState>,
    Query(params): Query<FindEntitiesParams>,
) -> ApiResult<Vec<FactorioEntity>> {
    let area_filter = match &params.area {
        Some(area) => AreaFilter::Rect(
            area.parse()
                .map_err(|_| ErrorResponse::bad_request(format!("invalid area: {area}")))?,
        ),
        None => match &params.position {
            Some(position) => AreaFilter::PositionRadius((
                position.parse().map_err(|_| {
                    ErrorResponse::bad_request(format!("invalid position: {position}"))
                })?,
                params.radius,
            )),
            None => {
                return Err(ErrorResponse::bad_request(
                    "area or position + optional radius needed",
                ))
            }
        },
    };

    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let entities = instance
        .rcon
        .find_entities_filtered(
            &area_filter,
            params.name.clone(),
            params.entity_type.clone(),
        )
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(entities))
}
