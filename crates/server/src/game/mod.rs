pub mod control;
pub mod query;

use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/find-entities", get(query::find_entities))
        .route("/find-tiles", get(query::find_tiles))
        .route("/inventory-contents-at", get(query::inventory_contents_at))
        .route("/player-info", get(query::player_info))
        .route("/all-players", get(query::all_players))
        .route("/item-prototypes", get(query::item_prototypes))
        .route("/entity-prototypes", get(query::entity_prototypes))
        .route("/move-player", post(control::move_player))
        .route("/place-entity", post(control::place_entity))
        .route("/cheat-item", post(control::cheat_item))
        .route("/cheat-technology", post(control::cheat_technology))
        .route(
            "/cheat-all-technologies",
            post(control::cheat_all_technologies),
        )
        .route("/insert-to-inventory", post(control::insert_to_inventory))
        .route(
            "/remove-from-inventory",
            post(control::remove_from_inventory),
        )
        .route("/server-save", post(control::server_save))
        .route("/add-research", post(control::add_research))
}
