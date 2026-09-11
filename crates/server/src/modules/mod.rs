//! Module design library API.

use axum::Json;
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// List all available module designs.
#[utoipa::path(get, path = "/api/v1/modules/designs", responses((status = 200, description = "Module designs")))]
async fn list_designs() -> Json<Value> {
    let designs = vec![
        ore_to_plate_design("iron-plate", "iron-ore"),
        ore_to_plate_design("copper-plate", "copper-ore"),
    ];
    Json(serde_json::json!({ "designs": designs }))
}

fn ore_to_plate_design(item: &str, ore: &str) -> Value {
    serde_json::json!({
        "schema": 1,
        "id": format!("ore-to-plate-{}", item.split('-').next().unwrap_or(item)),
        "family": "OreToPlate",
        "parameters": { "item": item, "with_pole": false, "labs": 0 },
        "parts": [
            { "role": "drill", "entity": "burner-mining-drill",
              "offset": { "half_x": 0, "half_y": 0 }, "direction": 0, "recipe": null, "half_size": { "half_x": 2, "half_y": 2 } },
            { "role": "furnace", "entity": "stone-furnace",
              "offset": { "half_x": 0, "half_y": 4 }, "direction": 0, "recipe": item, "half_size": { "half_x": 2, "half_y": 2 } }
        ],
        "ports": [
            { "id": "belt-input", "mode": "BeltInput", "item": ore,
              "offset": { "half_x": -2, "half_y": 0 }, "direction": 12 },
            { "id": "inventory-output", "mode": "InventoryOutput", "item": item,
              "offset": { "half_x": 0, "half_y": 6 }, "direction": 0 }
        ],
        "bill": { "burner-mining-drill": 1, "stone-furnace": 1 },
        "operation": {
            "inputs": { ore: { "numerator": 1, "ticks": 600 } },
            "outputs": { item: { "numerator": 1, "ticks": 600 } },
            "power_watts": 0,
            "fuel_per_tick": { "coal": { "numerator": 1, "ticks": 4800 } },
            "startup_latency_ticks": 4800,
            "startup_items": { "coal": 10, ore: 5 },
            "required_research": [], "required_surface": "nauvis"
        }
    })
}

pub fn router() -> OpenApiRouter<crate::state::AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_designs))
}
