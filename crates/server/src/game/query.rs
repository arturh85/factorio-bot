use crate::error::{ApiResult, ErrorResponse};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::Json;
use factorio_bot_core::types::{
    AreaFilter, FactorioEntity, FactorioEntityPrototype, FactorioItemPrototype, FactorioPlayer,
    FactorioTile, InventoryResponse, PlayerId, RequestEntity,
};
use serde::Deserialize;
use std::collections::HashMap;
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
        (status = 200, body = Vec<FactorioEntity>),
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

#[derive(Debug, Default, Deserialize, IntoParams)]
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
    Query(params): Query<FindTilesParams>,
) -> ApiResult<Vec<FactorioTile>> {
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
    let tiles = instance
        .rcon
        .find_tiles_filtered(&area_filter, params.name.clone())
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(tiles))
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(default)]
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
    Query(params): Query<InventoryContentsAtParams>,
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

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(default)]
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
    Query(params): Query<PlayerInfoParams>,
) -> ApiResult<FactorioPlayer> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = instance
        .world
        .as_ref()
        .ok_or_else(|| ErrorResponse::new("world not initialized".into(), 2))?;
    let player = world
        .players
        .get(&params.player_id)
        .ok_or_else(|| ErrorResponse::new("player not found".into(), 2))?;
    Ok(Json(player.clone()))
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
    let world = instance
        .world
        .as_ref()
        .ok_or_else(|| ErrorResponse::new("world not initialized".into(), 2))?;
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
    let world = instance
        .world
        .as_ref()
        .ok_or_else(|| ErrorResponse::new("world not initialized".into(), 2))?;
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
    let world = instance
        .world
        .as_ref()
        .ok_or_else(|| ErrorResponse::new("world not initialized".into(), 2))?;
    let mut data: HashMap<String, FactorioEntityPrototype> = HashMap::new();
    for prototype in world.entity_prototypes.iter() {
        data.insert(prototype.name.clone(), prototype.clone());
    }
    Ok(Json(data))
}
