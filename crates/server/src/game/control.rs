use crate::error::{ApiResult, ErrorResponse};
use crate::extract::ApiJson;
use crate::game::{require_player, require_world};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::types::{Direction, FactorioPlayer, PlaceEntityResult, PlayerId, Position};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub success: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MovePlayerBody {
    pub player_id: PlayerId,
    pub goal: String,
    pub radius: Option<f64>,
}

/// Move player to position
#[utoipa::path(
    post,
    path = "/api/v1/game/move-player",
    tag = "Control",
    request_body = MovePlayerBody,
    responses(
        (status = 200, body = FactorioPlayer),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn move_player(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<MovePlayerBody>,
) -> ApiResult<FactorioPlayer> {
    let goal: Position = body
        .goal
        .parse()
        .map_err(|_| ErrorResponse::bad_request(format!("invalid goal: {}", body.goal)))?;

    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_world(instance)?;
    // Validate the player id before mutating anything, see `require_player`.
    require_player(world, body.player_id)?;
    instance
        .rcon
        .move_player(world, body.player_id, &goal, body.radius)
        .await
        .map_err(ErrorResponse::from)?;

    let player = require_player(world, body.player_id)?;
    Ok(Json(player))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PlaceEntityBody {
    pub player_id: PlayerId,
    pub item: String,
    pub position: String,
    pub direction: u8,
}

/// Place entity by given player
#[utoipa::path(
    post,
    path = "/api/v1/game/place-entity",
    tag = "Control",
    request_body = PlaceEntityBody,
    responses(
        (status = 200, body = PlaceEntityResult),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn place_entity(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<PlaceEntityBody>,
) -> ApiResult<PlaceEntityResult> {
    let position: Position = body
        .position
        .parse()
        .map_err(|_| ErrorResponse::bad_request(format!("invalid position: {}", body.position)))?;
    Direction::from_u8(body.direction).ok_or_else(|| {
        ErrorResponse::bad_request(format!("invalid direction: {}", body.direction))
    })?;

    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_world(instance)?;
    // Validate the player id before mutating anything, see `require_player`.
    require_player(world, body.player_id)?;
    // `None`: this HTTP endpoint has no request field for an underground
    // half yet -- see `FactorioEntity::underground_half`.
    let entity = instance
        .rcon
        .place_entity(
            body.player_id,
            body.item.clone(),
            position,
            body.direction,
            None,
            world,
        )
        .await
        .map_err(ErrorResponse::from)?;
    sleep(Duration::from_millis(50)).await;
    let player = require_player(world, body.player_id)?;
    Ok(Json(PlaceEntityResult { entity, player }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheatItemBody {
    pub name: String,
    pub count: u32,
    pub player_id: PlayerId,
}

/// Cheat items and give them to player
#[utoipa::path(
    post,
    path = "/api/v1/game/cheat-item",
    tag = "Control",
    request_body = CheatItemBody,
    responses(
        (status = 200, body = FactorioPlayer),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn cheat_item(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CheatItemBody>,
) -> ApiResult<FactorioPlayer> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_world(instance)?;
    // Validate the player id before mutating anything, see `require_player`.
    require_player(world, body.player_id)?;
    instance
        .rcon
        .cheat_item(body.player_id, &body.name, body.count)
        .await
        .map_err(ErrorResponse::from)?;
    sleep(Duration::from_millis(50)).await;
    let player = require_player(world, body.player_id)?;
    Ok(Json(player))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheatTechnologyBody {
    pub tech: String,
}

/// Cheat Technology
#[utoipa::path(
    post,
    path = "/api/v1/game/cheat-technology",
    tag = "Control",
    request_body = CheatTechnologyBody,
    responses(
        (status = 200, body = OperationResult),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn cheat_technology(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CheatTechnologyBody>,
) -> ApiResult<OperationResult> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    instance
        .rcon
        .cheat_technology(&body.tech)
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(OperationResult { success: true }))
}

/// Cheat all Technologies
#[utoipa::path(
    post,
    path = "/api/v1/game/cheat-all-technologies",
    tag = "Control",
    responses(
        (status = 200, body = OperationResult),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn cheat_all_technologies(State(state): State<AppState>) -> ApiResult<OperationResult> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    instance
        .rcon
        .cheat_all_technologies()
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(OperationResult { success: true }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct InsertToInventoryBody {
    pub player_id: PlayerId,
    pub entity_name: String,
    pub entity_position: String,
    pub inventory_type: u32,
    pub item_name: String,
    pub item_count: u32,
}

/// Insert items into inventory
#[utoipa::path(
    post,
    path = "/api/v1/game/insert-to-inventory",
    tag = "Control",
    request_body = InsertToInventoryBody,
    responses(
        (status = 200, body = FactorioPlayer),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn insert_to_inventory(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<InsertToInventoryBody>,
) -> ApiResult<FactorioPlayer> {
    let entity_position: Position = body.entity_position.parse().map_err(|_| {
        ErrorResponse::bad_request(format!("invalid entity_position: {}", body.entity_position))
    })?;

    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_world(instance)?;
    // Validate the player id before mutating anything, see `require_player`.
    require_player(world, body.player_id)?;
    instance
        .rcon
        .insert_to_inventory(
            body.player_id,
            body.entity_name.clone(),
            entity_position,
            body.inventory_type,
            body.item_name.clone(),
            body.item_count,
            world,
        )
        .await
        .map_err(ErrorResponse::from)?;
    sleep(Duration::from_millis(50)).await;
    let player = require_player(world, body.player_id)?;
    Ok(Json(player))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RemoveFromInventoryBody {
    pub player_id: PlayerId,
    pub entity_name: String,
    pub entity_position: String,
    pub inventory_type: u32,
    pub item_name: String,
    pub item_count: u32,
}

/// Remove items from inventory
#[utoipa::path(
    post,
    path = "/api/v1/game/remove-from-inventory",
    tag = "Control",
    request_body = RemoveFromInventoryBody,
    responses(
        (status = 200, body = FactorioPlayer),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn remove_from_inventory(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<RemoveFromInventoryBody>,
) -> ApiResult<FactorioPlayer> {
    let entity_position: Position = body.entity_position.parse().map_err(|_| {
        ErrorResponse::bad_request(format!("invalid entity_position: {}", body.entity_position))
    })?;

    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    let world = require_world(instance)?;
    // Validate the player id before mutating anything, see `require_player`.
    require_player(world, body.player_id)?;
    instance
        .rcon
        .remove_from_inventory(
            body.player_id,
            body.entity_name.clone(),
            entity_position,
            body.inventory_type,
            body.item_name.clone(),
            body.item_count,
            world,
        )
        .await
        .map_err(ErrorResponse::from)?;
    sleep(Duration::from_millis(50)).await;
    let player = require_player(world, body.player_id)?;
    Ok(Json(player))
}

/// Server Save
#[utoipa::path(
    post,
    path = "/api/v1/game/server-save",
    tag = "Control",
    responses(
        (status = 200, body = OperationResult),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn server_save(State(state): State<AppState>) -> ApiResult<OperationResult> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    instance
        .rcon
        .server_save()
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(OperationResult { success: true }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddResearchBody {
    pub tech: String,
}

/// Add Research to Queue
#[utoipa::path(
    post,
    path = "/api/v1/game/add-research",
    tag = "Control",
    request_body = AddResearchBody,
    responses(
        (status = 200, body = OperationResult),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn add_research(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<AddResearchBody>,
) -> ApiResult<OperationResult> {
    let instance = state.instance.read().await;
    let instance = instance.as_ref().ok_or_else(ErrorResponse::not_started)?;
    instance
        .rcon
        .add_research(&body.tech)
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(OperationResult { success: true }))
}
