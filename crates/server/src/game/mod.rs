pub mod control;
pub mod query;

use crate::state::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

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
