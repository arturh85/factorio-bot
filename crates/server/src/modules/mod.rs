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
        pumpjack_design(),
        red_science_design(),
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


fn pumpjack_design() -> Value {
    serde_json::json!({
        "schema": 1,
        "id": "pumpjack",
        "family": "OreToPlate",
        "parameters": { "item": "crude-oil", "with_pole": false, "labs": 0 },
        "parts": [
            { "role": "pumpjack", "entity": "pumpjack",
              "offset": { "half_x": 0, "half_y": 0 }, "direction": 0, "recipe": null,
              "half_size": { "half_x": 3, "half_y": 3 } }
        ],
        "ports": [
            { "id": "output-crude", "mode": "InventoryOutput", "item": "crude-oil",
              "offset": { "half_x": 0, "half_y": 3 }, "direction": 0 }
        ],
        "bill": { "pumpjack": 1, "pipe": 2 },
        "operation": {
            "inputs": {},
            "outputs": { "crude-oil": { "numerator": 1, "ticks": 120 } },
            "power_watts": 90000,
            "fuel_per_tick": {},
            "startup_latency_ticks": 120,
            "startup_items": { "pipe": 2 },
            "required_research": ["oil-processing"],
            "required_surface": "nauvis"
        }
    })
}


fn red_science_design() -> Value {
    serde_json::json!({
        "schema": 1,
        "id": "red-science-cell",
        "family": "RedScience",
        "parameters": { "item": "automation-science-pack", "with_pole": false, "labs": 0 },
        "parts": [
            { "role": "gear-belt", "entity": "transport-belt",
              "offset": { "half_x": 0, "half_y": -5 }, "direction": 4, "recipe": null,
              "half_size": { "half_x": 1, "half_y": 1 } },
            { "role": "copper-belt", "entity": "transport-belt",
              "offset": { "half_x": 0, "half_y": -3 }, "direction": 4, "recipe": null,
              "half_size": { "half_x": 1, "half_y": 1 } },
            { "role": "gear-inserter", "entity": "long-handed-inserter",
              "offset": { "half_x": 0, "half_y": -2 }, "direction": 0, "recipe": null,
              "half_size": { "half_x": 1, "half_y": 1 } },
            { "role": "copper-inserter", "entity": "inserter",
              "offset": { "half_x": 0, "half_y": -1 }, "direction": 0, "recipe": null,
              "half_size": { "half_x": 1, "half_y": 1 } },
            { "role": "assembler", "entity": "assembling-machine-1",
              "offset": { "half_x": 0, "half_y": 0 }, "direction": 0,
              "recipe": "automation-science-pack",
              "half_size": { "half_x": 2, "half_y": 2 } },
            { "role": "out-inserter", "entity": "inserter",
              "offset": { "half_x": 0, "half_y": 3 }, "direction": 8, "recipe": null,
              "half_size": { "half_x": 1, "half_y": 1 } },
            { "role": "output-belt", "entity": "transport-belt",
              "offset": { "half_x": 0, "half_y": 5 }, "direction": 4, "recipe": null,
              "half_size": { "half_x": 1, "half_y": 1 } }
        ],
        "ports": [
            { "id": "input-gears", "mode": "BeltInput", "item": "iron-gear-wheel",
              "offset": { "half_x": -2, "half_y": -5 }, "direction": 12 },
            { "id": "input-copper", "mode": "BeltInput", "item": "copper-plate",
              "offset": { "half_x": -2, "half_y": -3 }, "direction": 12 },
            { "id": "output", "mode": "InventoryOutput", "item": "automation-science-pack",
              "offset": { "half_x": 2, "half_y": 5 }, "direction": 4 }
        ],
        "bill": { "assembling-machine-1": 1, "inserter": 1, "long-handed-inserter": 1,
                  "transport-belt": 3, "small-electric-pole": 1 },
        "operation": {
            "inputs": { "copper-plate": { "numerator": 1, "ticks": 180 },
                        "iron-gear-wheel": { "numerator": 1, "ticks": 180 } },
            "outputs": { "automation-science-pack": { "numerator": 1, "ticks": 180 } },
            "power_watts": 90000,
            "fuel_per_tick": {},
            "startup_latency_ticks": 180,
            "startup_items": { "copper-plate": 5, "iron-gear-wheel": 5 },
            "required_research": ["automation"],
            "required_surface": "nauvis"
        }
    })
}

pub fn router() -> OpenApiRouter<crate::state::AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_designs))
}
