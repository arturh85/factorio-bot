pub mod control;
pub mod query;

use crate::error::ErrorResponse;
use crate::state::AppState;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::process::process_control::FactorioInstance;
use factorio_bot_core::types::{FactorioPlayer, PlayerId};
use std::sync::Arc;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Returns the world of a running instance, or the standard error when the
/// instance is up but its world has not been populated yet.
pub fn require_world(instance: &FactorioInstance) -> Result<&Arc<FactorioWorld>, ErrorResponse> {
    instance
        .world
        .as_ref()
        .ok_or_else(|| ErrorResponse::new("world not initialized".into(), 2))
}

/// Looks up a player by id, or returns the standard not-found error.
///
/// Mutating handlers call this *before* their rcon call as well: the core
/// pathfinding fallback indexes `world.players` directly and would abort the
/// process for an unknown id (and we do not want to mutate on behalf of a
/// player that does not exist).
pub fn require_player(
    world: &FactorioWorld,
    player_id: PlayerId,
) -> Result<FactorioPlayer, ErrorResponse> {
    world
        .players
        .get(&player_id)
        .map(|player| player.clone())
        .ok_or_else(|| ErrorResponse::new("player not found".into(), 2))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(query::find_entities))
        .routes(routes!(query::find_tiles))
        .routes(routes!(query::inventory_contents_at))
        .routes(routes!(query::player_info))
        .routes(routes!(query::all_players))
        .routes(routes!(query::item_prototypes))
        .routes(routes!(query::entity_prototypes))
        .routes(routes!(query::plan_path))
        .routes(routes!(control::move_player))
        .routes(routes!(control::place_entity))
        .routes(routes!(control::cheat_item))
        .routes(routes!(control::cheat_technology))
        .routes(routes!(control::cheat_all_technologies))
        .routes(routes!(control::insert_to_inventory))
        .routes(routes!(control::remove_from_inventory))
        .routes(routes!(control::server_save))
        .routes(routes!(control::add_research))
}
