use crate::error::{ErrorResponse, RestApiResult};
use async_std::sync::{Arc, RwLock};
use factorio_bot_core::process::process_control::InstanceState;
use factorio_bot_core::types::{
    AreaFilter, Direction, FactorioEntity, FactorioItemPrototype, FactorioPlayer, FactorioTile,
    InventoryResponse, PlaceEntityResult, Position, RequestEntity,
};
use num_traits::cast::FromPrimitive;
use rocket::serde::json::Json;
use rocket::State;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub success: bool,
}

/// Finds entities in given area/radius
#[openapi(tag = "Query")]
#[get("/findEntities?<area>&<position>&<radius>&<name>&<entity_type>")]
pub async fn find_entities(
    area: Option<String>,
    position: Option<String>,
    radius: Option<f64>,
    name: Option<String>,
    entity_type: Option<String>,
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
) -> RestApiResult<Vec<FactorioEntity>> {
    let area_filter = match &area {
        Some(area) => AreaFilter::Rect(area.parse().unwrap()),
        None => {
            if let Some(position) = &position {
                AreaFilter::PositionRadius((position.parse().unwrap(), radius))
            } else {
                return Err(ErrorResponse::new(
                    "area or position + optional radius needed".into(),
                    1,
                ));
            }
        }
    };

    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        Ok(Json(
            instance_state
                .rcon
                .find_entities_filtered(&area_filter, name.clone(), entity_type.clone())
                .await
                .unwrap(),
        ))
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// Plan path from one position to another
#[openapi(tag = "Query")]
#[get("/planPath?<entity_name>&<entity_type>&<underground_entity_name>&<underground_entity_type>&<underground_max>&<from_position>&<to_position>&<to_direction>")]
#[allow(clippy::too_many_arguments)]
pub async fn plan_path(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
    entity_name: String,
    entity_type: String,
    underground_entity_name: String,
    underground_entity_type: String,
    underground_max: u8,
    from_position: String,
    to_position: String,
    to_direction: u8,
) -> RestApiResult<Vec<FactorioEntity>> {
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        Ok(Json(
            instance_state
                .rcon
                .plan_path(
                    &instance_state.world.as_ref().unwrap().clone(),
                    &entity_name,
                    &entity_type,
                    &underground_entity_name,
                    &underground_entity_type,
                    underground_max,
                    &from_position.parse().unwrap(),
                    &to_position.parse().unwrap(),
                    Direction::from_u8(to_direction).unwrap(),
                )
                .await
                .unwrap(),
        ))
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// Finds tiles in given area/radius
#[openapi(tag = "Query")]
#[get("/findTiles?<area>&<position>&<radius>&<name>")]
pub async fn find_tiles(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
    area: Option<String>,
    position: Option<String>,
    radius: Option<f64>,
    name: Option<String>,
) -> RestApiResult<Vec<FactorioTile>> {
    let area_filter = match &area {
        Some(area) => AreaFilter::Rect(area.parse().unwrap()),
        None => {
            if let Some(position) = &position {
                AreaFilter::PositionRadius((position.parse().unwrap(), radius))
            } else {
                return Err(ErrorResponse::new(
                    "area or position + optional radius needed".into(),
                    2,
                ));
            }
        }
    };

    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        Ok(Json(
            instance_state
                .rcon
                .find_tiles_filtered(&area_filter, name)
                .await
                .unwrap(),
        ))
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// List inventory contents at position
#[openapi(tag = "Query")]
#[get("/inventoryContentsAt?<query>")]
pub async fn inventory_contents_at(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
    query: String,
) -> RestApiResult<Vec<Option<InventoryResponse>>> {
    let parts: Vec<&str> = query.split(';').collect();
    let entities: Vec<RequestEntity> = parts
        .iter()
        .map(|part| {
            let parts: Vec<&str> = part.split('@').collect();
            RequestEntity {
                name: String::from(parts[0]),
                position: parts[1].parse().unwrap(),
            }
        })
        .collect();

    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        Ok(Json(
            instance_state
                .rcon
                .inventory_contents_at(entities)
                .await
                .unwrap(),
        ))
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// Move player to position
#[openapi(tag = "Control")]
#[get("/movePlayer?<player_id>&<goal>&<radius>")]
pub async fn move_player(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
    player_id: u32,
    goal: String,
    radius: Option<f64>,
) -> RestApiResult<FactorioPlayer> {
    let goal: Position = goal.parse().unwrap();

    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        let world = &instance_state.world.as_ref().unwrap().clone();
        instance_state
            .rcon
            .move_player(world, player_id, &goal, radius)
            .await
            .unwrap();

        let player = world.players.get(&player_id);
        match player {
            Some(player) => Ok(Json(player.clone())),
            None => Err(ErrorResponse::new("player not found".into(), 2)),
        }
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// Player Information
#[openapi(tag = "Query")]
#[get("/playerInfo?<player_id>")]
pub async fn player_info(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
    player_id: u32,
) -> RestApiResult<FactorioPlayer> {
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        let player = instance_state
            .world
            .as_ref()
            .unwrap()
            .players
            .get(&player_id);
        match player {
            Some(player) => Ok(Json(player.clone())),
            None => Err(ErrorResponse::new("player not found".into(), 2)),
        }
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// Place entity by given player
#[openapi(tag = "Place")]
#[get("/placeEntity?<player_id>&<item>&<position>&<direction>")]
pub async fn place_entity(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
    player_id: u32,
    item: String,
    position: String,
    direction: u8,
) -> RestApiResult<PlaceEntityResult> {
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        let world = &instance_state.world.as_ref().unwrap().clone();
        let entity = instance_state
            .rcon
            .place_entity(
                player_id,
                item.clone(),
                position.parse().unwrap(),
                direction,
                world,
            )
            .await
            .unwrap();
        async_std::task::sleep(Duration::from_millis(50)).await;
        let player = world.players.get(&player_id);
        match player {
            Some(player) => Ok(Json(PlaceEntityResult {
                entity,
                player: player.clone(),
            })),
            None => Err(ErrorResponse::new("player not found".into(), 2)),
        }
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// Cheat items and give them to player
#[openapi(tag = "Cheat")]
#[get("/cheatItem?<name>&<count>&<player_id>")]
pub async fn cheat_item(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
    name: String,
    count: u32,
    player_id: u32,
) -> RestApiResult<FactorioPlayer> {
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        let world = &instance_state.world.as_ref().unwrap().clone();
        instance_state
            .rcon
            .cheat_item(player_id, &name, count)
            .await
            .unwrap();
        async_std::task::sleep(Duration::from_millis(50)).await;
        let player = world.players.get(&player_id);
        match player {
            Some(player) => Ok(Json(player.clone())),
            None => Err(ErrorResponse::new("player not found".into(), 2)),
        }
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// Cheat Technology
#[openapi(tag = "Cheat")]
#[get("/cheatTechnology?<tech>")]
pub async fn cheat_technology(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
    tech: String,
) -> RestApiResult<OperationResult> {
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        instance_state.rcon.cheat_technology(&tech).await.unwrap();
        Ok(Json(OperationResult { success: true }))
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// Cheat Technology
#[openapi(tag = "Cheat")]
#[get("/cheatAllTechnologies")]
pub async fn cheat_all_technologies(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
) -> RestApiResult<OperationResult> {
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        instance_state.rcon.cheat_all_technologies().await.unwrap();
        Ok(Json(OperationResult { success: true }))
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// Insert items into inventory
#[openapi(tag = "Inventory")]
#[get(
    "/insertToInventory?<player_id>&<entity_name>&<entity_position>&<inventory_type>&<item_name>&<item_count>"
)]
pub async fn insert_to_inventory(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
    player_id: u32,
    entity_name: String,
    entity_position: String,
    inventory_type: u32,
    item_name: String,
    item_count: u32,
) -> RestApiResult<FactorioPlayer> {
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        let world = &instance_state.world.as_ref().unwrap().clone();
        instance_state
            .rcon
            .insert_to_inventory(
                player_id,
                entity_name.clone(),
                entity_position.parse().unwrap(),
                inventory_type,
                item_name.clone(),
                item_count,
                world,
            )
            .await
            .unwrap();
        async_std::task::sleep(Duration::from_millis(50)).await;
        let player = world.players.get(&player_id);
        match player {
            Some(player) => Ok(Json(player.clone())),
            None => Err(ErrorResponse::new("player not found".into(), 2)),
        }
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// Remove items from inventory
#[openapi(tag = "Inventory")]
#[get(
"/removeToInventory?<player_id>&<entity_name>&<entity_position>&<inventory_type>&<item_name>&<item_count>"
)]
pub async fn remove_from_inventory(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
    player_id: u32,
    entity_name: String,
    entity_position: String,
    inventory_type: u32,
    item_name: String,
    item_count: u32,
) -> RestApiResult<FactorioPlayer> {
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        let world = &instance_state.world.as_ref().unwrap().clone();
        instance_state
            .rcon
            .remove_from_inventory(
                player_id,
                entity_name.clone(),
                entity_position.parse().unwrap(),
                inventory_type,
                item_name.clone(),
                item_count,
                world,
            )
            .await
            .unwrap();
        async_std::task::sleep(Duration::from_millis(50)).await;
        let player = world.players.get(&player_id);
        match player {
            Some(player) => Ok(Json(player.clone())),
            None => Err(ErrorResponse::new("player not found".into(), 2)),
        }
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// List all connected Players
#[openapi(tag = "Query")]
#[get("/allPlayers")]
pub async fn all_players(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
) -> RestApiResult<Vec<FactorioPlayer>> {
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        let mut all_players: Vec<FactorioPlayer> = Vec::new();
        let world = &instance_state.world.as_ref().unwrap().clone();
        for player in world.players.iter() {
            all_players.push(player.clone());
        }
        Ok(Json(all_players))
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}

/// List all ItemPrototypes
#[openapi(tag = "Query")]
#[get("/itemPrototypes")]
pub async fn item_prototypes(
    instance_state: &State<Arc<RwLock<Option<InstanceState>>>>,
) -> RestApiResult<HashMap<String, FactorioItemPrototype>> {
    let instance_state = instance_state.read().await;
    if let Some(instance_state) = &*instance_state {
        let world = &instance_state.world.as_ref().unwrap().clone();
        let mut data: HashMap<String, FactorioItemPrototype> = HashMap::new();
        for item_prototype in world.item_prototypes.iter() {
            data.insert(item_prototype.name.clone(), item_prototype.clone());
        }
        Ok(Json(data))
    } else {
        Err(ErrorResponse::new("not started".into(), 2))
    }
}
//
// // #[get("/entityPrototypes")]
// pub async fn entity_prototypes(
//     world: web::Data<Arc<FactorioWorld>>,
// ) -> Result<Json<HashMap<String, FactorioEntityPrototype>>, ActixAnyhowError> {
//     let mut data: HashMap<String, FactorioEntityPrototype> = HashMap::new();
//     for prototype in world.entity_prototypes.iter() {
//         data.insert(prototype.name.clone(), prototype.clone());
//     }
//     Ok(Json(data))
// }
//
// // #[get("/serverSave")]
// pub async fn server_save(
//     rcon: web::Data<Arc<FactorioRcon>>,
// ) -> Result<Json<Value>, ActixAnyhowError> {
//     rcon.server_save().await?;
//     Ok(Json(json!({"status": "ok"})))
// }
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct AddResearchQueryParams {
//     tech: String,
// }
// // #[get("/addResearch?<tech>")]
// pub async fn add_research(
//     info: actix_web::web::Query<AddResearchQueryParams>,
//     rcon: web::Data<Arc<FactorioRcon>>,
// ) -> Result<Json<Value>, ActixAnyhowError> {
//     rcon.add_research(&info.tech).await?;
//     Ok(Json(json!({"status": "ok"})))
// }
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct StoreMapDataQueryParams {
//     key: String,
// }
//
// // #[post("/storeMapData?<key>", format = "application/json", data = "<value>")]
// pub async fn store_map_data(
//     rcon: web::Data<Arc<FactorioRcon>>,
//     data: Json<Value>,
//     info: actix_web::web::Query<StoreMapDataQueryParams>,
// ) -> Result<Json<Value>, ActixAnyhowError> {
//     rcon.store_map_data(&info.key, data.into_inner()).await?;
//     Ok(Json(json!({"status": "ok"})))
// }
// // #[get("/retrieveMapData?<key>")]
// pub async fn retrieve_map_data(
//     rcon: web::Data<Arc<FactorioRcon>>,
//     info: actix_web::web::Query<StoreMapDataQueryParams>,
// ) -> Result<Json<Value>, ActixAnyhowError> {
//     let res = rcon.retrieve_map_data(&info.key).await?;
//     match res {
//         Some(result) => Ok(Json(result)),
//         None => Ok(Json(json!(null))),
//     }
// }
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct PlaceBlueprintQueryParams {
//     blueprint: String,
//     position: String,
//     direction: Option<u8>,
//     force_build: Option<bool>,
//     only_ghosts: Option<bool>,
//     inventory_player_ids: Option<String>,
// }
// // #[get("/<player_id>/placeBlueprint?<position>&<direction>&<force_build>&<blueprint>&<only_ghosts>")]
// // #[allow(clippy::too_many_arguments)]
// pub async fn place_blueprint(
//     world: web::Data<Arc<FactorioWorld>>,
//     rcon: web::Data<Arc<FactorioRcon>>,
//     path: PathInfo<u32>,
//     info: actix_web::web::Query<PlaceBlueprintQueryParams>,
// ) -> Result<Json<PlaceEntitiesResult>, ActixAnyhowError> {
//     let player_id = *path;
//     let inventory_player_ids: Vec<u32> = match info.inventory_player_ids.as_ref() {
//         Some(inventory_player_ids) => inventory_player_ids
//             .split(',')
//             .map(|id| id.parse().unwrap())
//             .collect(),
//         None => vec![],
//     };
//     let entities = rcon
//         .place_blueprint(
//             player_id,
//             info.blueprint.clone(),
//             &info.position.parse()?,
//             info.direction.unwrap_or(0),
//             info.force_build.unwrap_or(false),
//             info.only_ghosts.unwrap_or(false),
//             inventory_player_ids,
//             &world,
//         )
//         .await?;
//     async_std::task::sleep(Duration::from_millis(50)).await;
//     let player = world.players.get(&player_id);
//     match player {
//         Some(player) => Ok(Json(PlaceEntitiesResult {
//             player: player.clone(),
//             entities,
//         })),
//         None => Err(ActixAnyhowError::from(anyhow!("player not found"))),
//     }
// }
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct ReviveGhostQueryParams {
//     name: String,
//     position: String,
// }
// // #[get("/<player_id>/reviveGhost?<position>&<name>")]
// // #[allow(clippy::too_many_arguments)]
// pub async fn revive_ghost(
//     info: actix_web::web::Query<ReviveGhostQueryParams>,
//     path: PathInfo<u32>,
//     world: web::Data<Arc<FactorioWorld>>,
//     rcon: web::Data<Arc<FactorioRcon>>,
// ) -> Result<Json<PlaceEntityResult>, ActixAnyhowError> {
//     let player_id = *path;
//     let entity = rcon
//         .revive_ghost(player_id, &info.name, &info.position.parse()?, &world)
//         .await?;
//     async_std::task::sleep(Duration::from_millis(50)).await;
//     let player = world.players.get(&player_id);
//     match player {
//         Some(player) => Ok(Json(PlaceEntityResult {
//             player: player.clone(),
//             entity,
//         })),
//         None => Err(ActixAnyhowError::from(anyhow!("player not found"))),
//     }
// }
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct CheatBlueprintQueryParams {
//     blueprint: String,
//     position: String,
//     direction: Option<u8>,
//     force_build: Option<bool>,
// }
// // #[get("/<player_id>/cheatBlueprint?<position>&<direction>&<force_build>&<blueprint>")]
// pub async fn cheat_blueprint(
//     world: web::Data<Arc<FactorioWorld>>,
//     rcon: web::Data<Arc<FactorioRcon>>,
//     info: actix_web::web::Query<CheatBlueprintQueryParams>,
//     path: PathInfo<u32>,
// ) -> Result<Json<PlaceEntitiesResult>, ActixAnyhowError> {
//     let player_id = *path;
//     let entities = rcon
//         .cheat_blueprint(
//             player_id,
//             info.blueprint.clone(),
//             &info.position.parse()?,
//             info.direction.unwrap_or(0),
//             info.force_build.unwrap_or(false),
//         )
//         .await?;
//     async_std::task::sleep(Duration::from_millis(50)).await;
//     let player = world.players.get(&player_id);
//     match player {
//         Some(player) => Ok(Json(PlaceEntitiesResult {
//             player: player.clone(),
//             entities,
//         })),
//         None => Err(ActixAnyhowError::from(anyhow!("player not found"))),
//     }
// }
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct ParseBlueprintQueryParams {
//     label: String,
//     blueprint: String,
// }
//
// // #[get("/parseBlueprint?<blueprint>")]
// pub async fn parse_blueprint(
//     world: web::Data<Arc<FactorioWorld>>,
//     info: actix_web::web::Query<ParseBlueprintQueryParams>,
// ) -> Result<Json<FactorioBlueprintInfo>, ActixAnyhowError> {
//     let decoded =
//         BlueprintCodec::decode_string(&info.blueprint).expect("failed to parse blueprint");
//     let rect = blueprint_build_area(world.entity_prototypes.clone(), &info.blueprint);
//     let response = FactorioBlueprintInfo {
//         rect: rect.clone(),
//         label: info.label.clone(),
//         blueprint: info.blueprint.clone(),
//         width: rect.width() as u16,
//         height: rect.height() as u16,
//         data: serde_json::to_value(decoded).unwrap(),
//     };
//     Ok(Json(response))
// }
//
// // #[get("/recipes")]
// pub async fn all_recipes(
//     world: web::Data<Arc<FactorioWorld>>,
// ) -> Result<Json<HashMap<String, FactorioRecipe>>, ActixAnyhowError> {
//     let mut map: HashMap<String, FactorioRecipe> = HashMap::new();
//     for recipe in world.recipes.iter() {
//         map.insert(recipe.name.clone(), recipe.clone());
//     }
//     Ok(Json(map))
// }
// // #[get("/playerForce")]
// pub async fn player_force(
//     world: web::Data<Arc<FactorioWorld>>,
// ) -> Result<Json<FactorioForce>, ActixAnyhowError> {
//     Ok(Json(
//         world
//             .forces
//             .get("player")
//             .expect("player force not found")
//             .clone(),
//     ))
// }
// pub async fn all_forces(
//     world: web::Data<Arc<FactorioWorld>>,
// ) -> Result<Json<Vec<FactorioForce>>, ActixAnyhowError> {
//     let mut forces: Vec<FactorioForce> = vec![];
//     for force in world.forces.iter() {
//         forces.push(force.clone());
//     }
//     Ok(Json(forces))
// }
//
// // #[get("/<player_id>/mine?<name>&<position>&<count>")]
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct MineQueryParams {
//     name: String,
//     position: String,
//     count: u32,
// }
// pub async fn mine(
//     info: actix_web::web::Query<MineQueryParams>,
//     path: PathInfo<u32>,
//     rcon: web::Data<Arc<FactorioRcon>>,
//     world: web::Data<Arc<FactorioWorld>>,
// ) -> Result<Json<FactorioPlayer>, ActixAnyhowError> {
//     let player_id = *path;
//     rcon.player_mine(
//         &world,
//         player_id,
//         &info.name,
//         &info.position.parse()?,
//         info.count,
//     )
//     .await?;
//     async_std::task::sleep(Duration::from_millis(50)).await;
//     let player = world.players.get(&player_id);
//     match player {
//         Some(player) => Ok(Json(player.clone())),
//         None => Err(ActixAnyhowError::from(anyhow!("player not found"))),
//     }
// }
//
// // #[get("/<player_id>/craft?<recipe>&<count>")]
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct CraftQueryParams {
//     recipe: String,
//     count: u32,
// }
// pub async fn craft(
//     info: actix_web::web::Query<CraftQueryParams>,
//     path: PathInfo<u32>,
//     rcon: web::Data<Arc<FactorioRcon>>,
//     world: web::Data<Arc<FactorioWorld>>,
// ) -> Result<Json<FactorioPlayer>, ActixAnyhowError> {
//     let player_id = *path;
//     rcon.player_craft(&world, player_id, &info.recipe, info.count)
//         .await?;
//     async_std::task::sleep(Duration::from_millis(50)).await;
//     let player = world.players.get(&player_id);
//     match player {
//         Some(player) => Ok(Json(player.clone())),
//         None => Err(ActixAnyhowError::from(anyhow!("player not found"))),
//     }
// }
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct FindOffshorePumpPlacementOptionsQueryParams {
//     search_center: String,
//     pump_direction: u8,
// }
// pub async fn find_offshore_pump_placement_options(
//     info: actix_web::web::Query<FindOffshorePumpPlacementOptionsQueryParams>,
//     rcon: web::Data<Arc<FactorioRcon>>,
//     world: web::Data<Arc<FactorioWorld>>,
// ) -> Result<Json<Vec<Position>>, ActixAnyhowError> {
//     Ok(Json(
//         rcon.find_offshore_pump_placement_options(
//             &world,
//             info.search_center.parse()?,
//             Direction::from_u8(info.pump_direction).expect("invalid direction"),
//         )
//         .await?
//         .iter()
//         .map(|pos| pos.into())
//         .collect(),
//     ))
// }
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct PlanQueryParams {
//     name: String,
//     bot_count: u32,
// }
// pub async fn run_plan(
//     info: actix_web::web::Query<PlanQueryParams>,
//     planner: web::Data<Arc<RwLock<Planner>>>,
// ) -> Result<String, ActixAnyhowError> {
//     let lua_path_str = format!("plans/{}.lua", info.name);
//     let lua_path = Path::new(&lua_path_str);
//     let lua_path = std::fs::canonicalize(lua_path).unwrap();
//     if !lua_path.exists() {
//         return Err(anyhow!("plan {} not found at {}", info.name, lua_path_str));
//     }
//     let lua_code = read_to_string(lua_path).unwrap();
//     let graph = std::thread::spawn(move || {
//         let mut planner = planner.write();
//         planner.reset();
//         planner.plan(lua_code, info.bot_count).unwrap();
//         planner.graph()
//     })
//     .join()?;
//     Ok(graph.graphviz_dot())
// }
//
// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct ExecuteTaskGraphQueryParams {
//     name: String,
// }
// pub async fn execute_taskgraph(
//     info: actix_web::web::Query<ExecuteTaskGraphQueryParams>,
//     planner: web::Data<Arc<RwLock<Planner>>>,
//     world: web::Data<Arc<FactorioWorld>>,
// ) -> Result<String, ActixAnyhowError> {
//     let lua_path_str = format!("plans/{}.lua", info.name);
//     let lua_path = Path::new(&lua_path_str);
//     let lua_path = std::fs::canonicalize(lua_path).unwrap();
//     if !lua_path.exists() {
//         panic!("plan {} not found at {}", info.name, lua_path_str);
//     }
//     let lua_code = read_to_string(lua_path).unwrap();
//     let graph = std::thread::spawn(move || {
//         let mut planner = planner.write();
//         planner.plan(lua_code, world.players.len() as u32).unwrap();
//         planner.graph()
//     })
//     .join()
//     .unwrap();
//     let dot = graph.graphviz_dot();
//     Ok(dot)
// }
//
// pub async fn plans() -> Result<Json<Vec<String>>, ActixAnyhowError> {
//     let entries: Vec<String> = read_dir("plans/")
//         .unwrap()
//         .map(|res| res.map(|e| e.path()).unwrap())
//         .filter(|p| p.extension().is_some() && p.extension().unwrap() == "lua")
//         .map(|p| p.with_extension(""))
//         .map(|p| p.file_name().unwrap().to_str().unwrap().into())
//         .collect();
//     Ok(Json(entries))
// }
// pub async fn web_entity_graph(
//     world: web::Data<Arc<FactorioWorld>>,
// ) -> Result<String, ActixAnyhowError> {
//     world.entity_graph.connect()?;
//     let dot = world.entity_graph.graphviz_dot_condensed();
//     Ok(dot)
// }
// pub async fn web_task_graph(
//     planner: web::Data<Arc<RwLock<Planner>>>,
// ) -> Result<String, ActixAnyhowError> {
//     let planner = planner.read();
//     let dot = planner.graph().graphviz_dot();
//     Ok(dot)
// }
// pub async fn web_flow_graph(
//     world: web::Data<Arc<FactorioWorld>>,
// ) -> Result<String, ActixAnyhowError> {
//     world.entity_graph.connect()?;
//     world.flow_graph.update()?;
//     let dot = world.flow_graph.graphviz_dot_condensed();
//     Ok(dot)
// }
