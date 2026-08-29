pub mod query;

use crate::state::AppState;
use axum::routing::get;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new().route("/find-entities", get(query::find_entities))
}
