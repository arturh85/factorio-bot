use crate::error::{ApiResult, ErrorResponse};
use crate::extract::ApiQuery;
use crate::game::{require_player, require_world};
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::types::{
    AreaFilter, Direction, FactorioEntity, FactorioEntityPrototype, FactorioItemPrototype,
    FactorioPlayer, FactorioTile, InventoryResponse, PlayerId, Position, RequestEntity,
};
use serde::Deserialize;
use std::collections::HashMap;
use utoipa::IntoParams;

/// Builds an [`AreaFilter`] from the mutually exclusive `area` and
/// `position` + `radius` query parameters.
fn area_filter_from(
    area: Option<&str>,
    position: Option<&str>,
    radius: Option<f64>,
) -> Result<AreaFilter, ErrorResponse> {
    match area {
        Some(area) => Ok(AreaFilter::Rect(area.parse().map_err(|_| {
            ErrorResponse::bad_request(format!("invalid area: {area}"))
        })?)),
        None => match position {
            Some(position) => Ok(AreaFilter::PositionRadius((
                position.parse().map_err(|_| {
                    ErrorResponse::bad_request(format!("invalid position: {position}"))
                })?,
                radius,
            ))),
            None => Err(ErrorResponse::bad_request(
                "area or position + optional radius needed",
            )),
        },
    }
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
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
        (status = 200, body = Vec<FactorioEntity>),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn find_entities(
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<FindEntitiesParams>,
) -> ApiResult<Vec<FactorioEntity>> {
    let area_filter = area_filter_from(
        params.area.as_deref(),
        params.position.as_deref(),
        params.radius,
    )?;

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

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PlanPathParams {
    pub entity_name: String,
    pub entity_type: String,
    pub underground_entity_name: String,
    pub underground_entity_type: String,
    pub underground_max: u8,
    pub from_position: String,
    pub to_position: String,
    pub to_direction: u8,
}

/// Plan path from one position to another
#[utoipa::path(
    get,
    path = "/api/v1/game/plan-path",
    tag = "Query",
    params(PlanPathParams),
    responses(
        (status = 200, body = Vec<FactorioEntity>),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn plan_path(
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<PlanPathParams>,
) -> ApiResult<Vec<FactorioEntity>> {
    let from_position: Position = params.from_position.parse().map_err(|_| {
        ErrorResponse::bad_request(format!("invalid from_position: {}", params.from_position))
    })?;
    let to_position: Position = params.to_position.parse().map_err(|_| {
        ErrorResponse::bad_request(format!("invalid to_position: {}", params.to_position))
    })?;
    let to_direction = Direction::from_u8(params.to_direction).ok_or_else(|| {
        ErrorResponse::bad_request(format!("invalid to_direction: {}", params.to_direction))
    })?;

    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_world(instance)?;
    let entities = instance
        .rcon
        .plan_path(
            world,
            &params.entity_name,
            &params.entity_type,
            &params.underground_entity_name,
            &params.underground_entity_type,
            params.underground_max,
            &from_position,
            &to_position,
            to_direction,
        )
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(entities))
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(default)]
pub struct FindTilesParams {
    pub area: Option<String>,
    pub position: Option<String>,
    pub radius: Option<f64>,
    pub name: Option<String>,
}

/// Finds tiles in given area/radius
#[utoipa::path(
    get,
    path = "/api/v1/game/find-tiles",
    tag = "Query",
    params(FindTilesParams),
    responses(
        (status = 200, body = Vec<FactorioTile>),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn find_tiles(
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<FindTilesParams>,
) -> ApiResult<Vec<FactorioTile>> {
    let area_filter = area_filter_from(
        params.area.as_deref(),
        params.position.as_deref(),
        params.radius,
    )?;

    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let tiles = instance
        .rcon
        .find_tiles_filtered(&area_filter, params.name.clone())
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(tiles))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InventoryContentsAtParams {
    pub query: String,
}

/// List inventory contents at position
#[utoipa::path(
    get,
    path = "/api/v1/game/inventory-contents-at",
    tag = "Query",
    params(InventoryContentsAtParams),
    responses(
        (status = 200, body = Vec<Option<InventoryResponse>>),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn inventory_contents_at(
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<InventoryContentsAtParams>,
) -> ApiResult<Vec<Option<InventoryResponse>>> {
    let entities: Vec<RequestEntity> = params
        .query
        .split(';')
        .map(|part| {
            let mut parts = part.split('@');
            let name = parts.next().unwrap_or_default().to_owned();
            let position = parts
                .next()
                .ok_or_else(|| ErrorResponse::bad_request("expected <name>@<position>"))?;
            let position = position
                .parse()
                .map_err(|_| ErrorResponse::bad_request(format!("invalid position: {position}")))?;
            Ok(RequestEntity { name, position })
        })
        .collect::<Result<Vec<RequestEntity>, ErrorResponse>>()?;

    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let contents = instance
        .rcon
        .inventory_contents_at(entities)
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(contents))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PlayerInfoParams {
    pub player_id: PlayerId,
}

/// Player Information
#[utoipa::path(
    get,
    path = "/api/v1/game/player-info",
    tag = "Query",
    params(PlayerInfoParams),
    responses(
        (status = 200, body = FactorioPlayer),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn player_info(
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<PlayerInfoParams>,
) -> ApiResult<FactorioPlayer> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_world(instance)?;
    let player = require_player(world, params.player_id)?;
    Ok(Json(player))
}

/// List all connected Players
#[utoipa::path(
    get,
    path = "/api/v1/game/all-players",
    tag = "Query",
    responses(
        (status = 200, body = Vec<FactorioPlayer>),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn all_players(State(state): State<AppState>) -> ApiResult<Vec<FactorioPlayer>> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_world(instance)?;
    let mut all_players: Vec<FactorioPlayer> = Vec::new();
    for player in world.players.iter() {
        all_players.push(player.clone());
    }
    Ok(Json(all_players))
}

/// List all ItemPrototypes
#[utoipa::path(
    get,
    path = "/api/v1/game/item-prototypes",
    tag = "Query",
    responses(
        (status = 200, body = HashMap<String, FactorioItemPrototype>),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn item_prototypes(
    State(state): State<AppState>,
) -> ApiResult<HashMap<String, FactorioItemPrototype>> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_world(instance)?;
    let mut data: HashMap<String, FactorioItemPrototype> = HashMap::new();
    for item_prototype in world.item_prototypes.iter() {
        data.insert(item_prototype.name.clone(), item_prototype.clone());
    }
    Ok(Json(data))
}

/// List all EntityPrototypes
#[utoipa::path(
    get,
    path = "/api/v1/game/entity-prototypes",
    tag = "Query",
    responses(
        (status = 200, body = HashMap<String, FactorioEntityPrototype>),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn entity_prototypes(
    State(state): State<AppState>,
) -> ApiResult<HashMap<String, FactorioEntityPrototype>> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_world(instance)?;
    let mut data: HashMap<String, FactorioEntityPrototype> = HashMap::new();
    for prototype in world.entity_prototypes.iter() {
        data.insert(prototype.name.clone(), prototype.clone());
    }
    Ok(Json(data))
}
