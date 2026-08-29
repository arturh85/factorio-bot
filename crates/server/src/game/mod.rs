pub mod query;

use crate::state::AppState;
use axum::routing::get;
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
}
