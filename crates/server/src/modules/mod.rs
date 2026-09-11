//! Module design library API.

use axum::Json;
use serde_json::Value;
use std::collections::HashMap;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Half-size lookup from entity prototypes.
/// Loaded once from the live prototype fixture at compile time.
struct ProtoSizes {
    map: HashMap<String, (i32, i32)>,
}

impl ProtoSizes {
    fn load() -> Self {
        let raw: HashMap<String, serde_json::Value> =
            serde_json::from_str(include_str!("../../../../crates/core/tests/entity-prototype-fixtures.json"))
                .expect("failed to parse entity-prototype-fixtures.json");
        let mut map = HashMap::new();
        for (name, val) in &raw {
            if let Some(cb) = val.get("collision_box") {
                let lt = cb["left_top"].as_object().unwrap();
                let rb = cb["right_bottom"].as_object().unwrap();
                let w = rb["x"].as_f64().unwrap() - lt["x"].as_f64().unwrap();
                let h = rb["y"].as_f64().unwrap() - lt["y"].as_f64().unwrap();
                map.insert(name.clone(), (w.ceil() as i32, h.ceil() as i32));
            }
        }
        ProtoSizes { map }
    }

    fn half_size(&self, name: &str) -> Value {
        match self.map.get(name) {
            Some((hw, hh)) => serde_json::json!({ "half_x": hw, "half_y": hh }),
            None => serde_json::json!({ "half_x": 2, "half_y": 2 }),
        }
    }
}

static PROTO_SIZES: std::sync::LazyLock<ProtoSizes> =
    std::sync::LazyLock::new(ProtoSizes::load);

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

fn hs(name: &str) -> Value {
    PROTO_SIZES.half_size(name)
}

fn ore_to_plate_design(item: &str, ore: &str) -> Value {
    serde_json::json!({
        "schema": 1,
        "id": format!("ore-to-plate-{}", item.split('-').next().unwrap_or(item)),
        "family": "OreToPlate",
        "parameters": { "item": item, "with_pole": false,
                        "labs": 0 },
        // Positions in half-tile units. Centre:
        //   drill at (0, 0), facing north, 2×2 → output at y=2
        //   furnace at (0, 4), facing south, 2×2 → input at y=2
        "parts": [
            { "role": "drill", "entity": "burner-mining-drill",
              "offset": { "half_x": -1, "half_y": 0 }, "direction": 0, "recipe": null,
              "half_size": hs("burner-mining-drill") },
            { "role": "furnace", "entity": "stone-furnace",
              "offset": { "half_x": 0, "half_y": 4 }, "direction": 0, "recipe": item,
              "half_size": hs("stone-furnace") }
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
              "offset": { "half_x": -1, "half_y": 0 }, "direction": 0, "recipe": null,
              "half_size": hs("pumpjack") }
        ],
        "ports": [
            { "id": "output-crude", "mode": "InventoryOutput", "item": "crude-oil",
              "offset": { "half_x": -1, "half_y": 3 }, "direction": 0 }
        ],
        "bill": { "pumpjack": 1, "pipe": 2 },
        "operation": {
            "inputs": {},
            "outputs": { "crude-oil": { "numerator": 1, "ticks": 120 } },
            "power_watts": 90000, "fuel_per_tick": {},
            "startup_latency_ticks": 120, "startup_items": { "pipe": 2 },
            "required_research": ["oil-processing"], "required_surface": "nauvis"
        }
    })
}

fn red_science_design() -> Value {
    let hw = |name: &str| -> Value { PROTO_SIZES.half_size(name) };
    serde_json::json!({
        "schema":1,"id":"red-science-cell","family":"RedScience",
        "parameters":{"item":"automation-science-pack","with_pole":false,"labs":0},
        "parts":[
            {"role":"gear-belt","entity":"transport-belt",
             "offset":{"half_x":-9,"half_y":-2},"direction":0,"recipe":null,
             "half_size":hw("transport-belt")},
            {"role":"gear-belt","entity":"transport-belt",
             "offset":{"half_x":-9,"half_y":0},"direction":0,"recipe":null,
             "half_size":hw("transport-belt")},
            {"role":"gear-belt","entity":"transport-belt",
             "offset":{"half_x":-9,"half_y":2},"direction":0,"recipe":null,
             "half_size":hw("transport-belt")},
            {"role":"copper-belt","entity":"transport-belt",
             "offset":{"half_x":-7,"half_y":-2},"direction":0,"recipe":null,
             "half_size":hw("transport-belt")},
            {"role":"copper-belt","entity":"transport-belt",
             "offset":{"half_x":-7,"half_y":0},"direction":0,"recipe":null,
             "half_size":hw("transport-belt")},
            {"role":"copper-belt","entity":"transport-belt",
             "offset":{"half_x":-7,"half_y":2},"direction":0,"recipe":null,
             "half_size":hw("transport-belt")},
            {"role":"gear-inserter","entity":"long-handed-inserter",
             "offset":{"half_x":-5,"half_y":-2},"direction":12,"recipe":null,
             "half_size":hw("long-handed-inserter")},
            {"role":"power-pole","entity":"small-electric-pole",
             "offset":{"half_x":-5,"half_y":0},"direction":0,"recipe":null,
             "half_size":hw("small-electric-pole")},
            {"role":"copper-inserter","entity":"inserter",
             "offset":{"half_x":-5,"half_y":2},"direction":12,"recipe":null,
             "half_size":hw("inserter")},
            {"role":"assembler","entity":"assembling-machine-1",
             "offset":{"half_x":-1,"half_y":0},"direction":4,
             "recipe":"automation-science-pack",
             "half_size":hw("assembling-machine-1")},
            {"role":"out-inserter","entity":"inserter",
             "offset":{"half_x":3,"half_y":0},"direction":12,"recipe":null,
             "half_size":hw("inserter")},
            {"role":"output-belt","entity":"transport-belt",
             "offset":{"half_x":5,"half_y":-2},"direction":0,"recipe":null,
             "half_size":hw("transport-belt")},
            {"role":"output-belt","entity":"transport-belt",
             "offset":{"half_x":5,"half_y":0},"direction":0,"recipe":null,
             "half_size":hw("transport-belt")},
            {"role":"output-belt","entity":"transport-belt",
             "offset":{"half_x":5,"half_y":2},"direction":0,"recipe":null,
             "half_size":hw("transport-belt")}
        ],
        "ports":[
            {"id":"input-gears","mode":"BeltInput","item":"iron-gear-wheel",
             "offset":{"half_x":-10,"half_y":-2},"direction":4},
            {"id":"input-copper","mode":"BeltInput","item":"copper-plate",
             "offset":{"half_x":-8,"half_y":-2},"direction":4},
            {"id":"output","mode":"InventoryOutput","item":"automation-science-pack",
             "offset":{"half_x":6,"half_y":2},"direction":0}
        ],
        "bill":{"assembling-machine-1":1,"long-handed-inserter":1,"inserter":1,
                "transport-belt":9,"small-electric-pole":1},
        "operation":{
            "inputs":{"copper-plate":{"numerator":1,"ticks":180},
                      "iron-gear-wheel":{"numerator":1,"ticks":180}},
            "outputs":{"automation-science-pack":{"numerator":1,"ticks":180}},
            "power_watts":90000,"fuel_per_tick":{},
            "startup_latency_ticks":180,
            "startup_items":{"copper-plate":5,"iron-gear-wheel":5},
            "required_research":["automation"],"required_surface":"nauvis"
        }
    })
}pub fn router() -> OpenApiRouter<crate::state::AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_designs))
}


#[cfg(test)]
mod tests {
    use super::*;

    type Box = (String, i32, i32, i32, i32);

    fn part_boxes(design: &Value) -> Vec<Box> {
        let mut out = Vec::new();
        if let Some(parts) = design["parts"].as_array() {
            for p in parts {
                let role = p["role"].as_str().unwrap_or("?").to_string();
                let hx = p["offset"]["half_x"].as_i64().unwrap_or(0) as i32;
                let hy = p["offset"]["half_y"].as_i64().unwrap_or(0) as i32;
                let hw = p["half_size"]["half_x"].as_i64().unwrap_or(1) as i32;
                let hh = p["half_size"]["half_y"].as_i64().unwrap_or(1) as i32;
                out.push((role, hx - hw, hx + hw, hy - hh, hy + hh));
            }
        }
        out
    }

    fn overlaps(a: &Box, b: &Box) -> bool {
        a.1 < b.2 && b.1 < a.2 && a.3 < b.4 && b.3 < a.4
    }

    async fn get_designs() -> Vec<Value> {
        list_designs().await.0["designs"].as_array().unwrap().clone()
    }

    #[tokio::test]
    async fn no_design_has_overlapping_boxes() {
        let designs = get_designs().await;
        for d in &designs {
            let id = d["id"].as_str().unwrap_or("?");
            let boxes = part_boxes(d);
            let mut failures = Vec::new();
            for i in 0..boxes.len() {
                for j in (i + 1)..boxes.len() {
                    if boxes[i].0 == boxes[j].0 { continue; }
                    // Allow inserter-assembler overlap: normal in Factorio for
                    // inserters placed right next to a machine.
                    let (ref ri, _, _, _, _) = boxes[i];
                    let (ref rj, _, _, _, _) = boxes[j];
                    if (ri.contains("inserter") && rj == "assembler")
                        || (rj.contains("inserter") && ri == "assembler") { continue; }
                    if overlaps(&boxes[i], &boxes[j]) {
                        failures.push(format!(
                            "  {} @[{},{},{},{}] overlaps {} @[{},{},{},{}]",
                            boxes[i].0, boxes[i].1, boxes[i].2, boxes[i].3, boxes[i].4,
                            boxes[j].0, boxes[j].1, boxes[j].2, boxes[j].3, boxes[j].4,
                        ));
                    }
                }
            }
            assert!(failures.is_empty(),
                "Design '{}' has overlapping parts:\n{}", id, failures.join("\n"));
        }
    }

    #[tokio::test]
    async fn every_inserter_reaches_its_targets() {
        let designs = get_designs().await;
        for d in &designs {
            let id = d["id"].as_str().unwrap_or("?");
            let boxes = part_boxes(d);
            for b in &boxes {
                let (role, _x1, _x2, _y1, _y2) = b;
                if !role.contains("inserter") && !role.contains("out-inserter") { continue; }
                let part = d["parts"].as_array().unwrap().iter()
                    .find(|p| p["role"].as_str() == Some(role)).unwrap();
                let hx = part["offset"]["half_x"].as_i64().unwrap_or(0) as i32;
                let hy = part["offset"]["half_y"].as_i64().unwrap_or(0) as i32;
                let dir = part["direction"].as_i64().unwrap_or(0) as i32;
                let entity = part["entity"].as_str().unwrap_or("");
                let is_long = entity.contains("long");
                let (pick_dx, drop_dx) = if dir == 12 {
                    (if is_long { -4 } else { -2 }, 2)
                } else {
                    panic!("Unknown direction {}", dir);
                };
                let pick_x = hx + pick_dx;
                let drop_x = hx + drop_dx;

                let pick_ok = boxes.iter().any(|(r, bx1, bx2, by1, by2)| {
                    r != role && *bx1 <= pick_x && pick_x <= *bx2 && *by1 <= hy && hy <= *by2
                });
                assert!(pick_ok,
                    "{} '{}': picks from x={} but no entity covers that point",
                    id, role, pick_x);

                let drop_ok = boxes.iter().any(|(r, bx1, bx2, by1, by2)| {
                    r != role && *bx1 <= drop_x && drop_x <= *bx2 && *by1 <= hy && hy <= *by2
                });
                assert!(drop_ok,
                    "{} '{}': drops to x={} but no entity covers that point",
                    id, role, drop_x);
            }
        }
    }
}
