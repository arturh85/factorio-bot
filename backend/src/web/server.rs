use std::env;
use std::path::Path;
use std::sync::Arc;

use actix::Addr;
use actix_cors::Cors;
use actix_files as fs;
use actix_web::{
    http, middleware, web, App, Error, HttpRequest, HttpResponse, HttpServer, Responder,
};
use actix_web_actors::ws;
use dashmap::lock::RwLock;

use crate::factorio::planner::Planner;
use crate::factorio::rcon::FactorioRcon;
use crate::factorio::world::FactorioWorld;
use crate::factorio::ws::{FactorioWebSocketClient, FactorioWebSocketServer, RegisterWSClient};

async fn status() -> impl Responder {
    HttpResponse::Ok().body("ok")
}

async fn ws_index(
    r: HttpRequest,
    stream: web::Payload,
    data: web::Data<Addr<FactorioWebSocketServer>>,
) -> Result<HttpResponse, Error> {
    let (addr, res) = ws::start_with_addr(FactorioWebSocketClient {}, &r, stream)?;
    data.get_ref().do_send(RegisterWSClient { addr });
    Ok(res)
}

pub async fn start_webserver(
    rcon: Arc<FactorioRcon>,
    websocket_server: Addr<FactorioWebSocketServer>,
    open_browser: bool,
    world: Arc<FactorioWorld>,
) {
    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "7123".into())
        .parse()
        .expect("invalid PORT env");

    let url = format!("http://localhost:{}", port);
    info!("🚀 Webserver ready at: <yellow><underline>{}</>", url);
    if open_browser {
        webbrowser::open(&url).expect("failed to open browser");
    }
    let frontend_path = if Path::new("public/").exists() {
        "public/"
    } else {
        "frontend/dist/"
    };
    let planner = Arc::new(RwLock::new(Planner::new(world.clone(), Some(rcon.clone()))));
    HttpServer::new(move || {
        App::new()
            .data(world.clone())
            .data(rcon.clone())
            .data(planner.clone())
            .data(websocket_server.clone())
            .wrap(
                Cors::new()
                    .allowed_methods(vec!["GET", "POST"])
                    .allowed_header(http::header::ACCEPT)
                    .allowed_header(http::header::CONTENT_TYPE)
                    .allowed_header(http::header::CACHE_CONTROL)
                    .finish(),
            )
            .wrap(middleware::Logger::default())
            // websocket route
            .service(web::resource("/ws/").route(web::get().to(ws_index)))
            .service(
                web::resource("/api/tiles/{tile_z}/{tile_x}/{tile_y}/map_tile.png")
                    .route(web::get().to(crate::web::map_tiles::map_tiles)),
            )
            .service(
                web::resource("/api/tiles/{tile_z}/{tile_x}/{tile_y}/schematic_tile.png")
                    .route(web::get().to(crate::web::graph_tiles::map_tiles)),
            )
            .service(
                web::resource("/api/tiles/{tile_z}/{tile_x}/{tile_y}/blocked_tile.png")
                    .route(web::get().to(crate::web::graph_tiles::blocked_tiles)),
            )
            .service(
                web::resource("/api/tiles/{tile_z}/{tile_x}/{tile_y}/resource_tile.png")
                    .route(web::get().to(crate::web::graph_tiles::resource_tiles)),
            )
            .service(
                web::resource("/api/tiles/{tile_z}/{tile_x}/{tile_y}/entity_graph_tile.png")
                    .route(web::get().to(crate::web::graph_tiles::entity_graph_tiles)),
            )
            .service(
                web::resource("/api/tiles/{tile_z}/{tile_x}/{tile_y}/flow_graph_tile.png")
                    .route(web::get().to(crate::web::graph_tiles::flow_graph_tiles)),
            )
            .service(
                web::resource("/api/findEntities")
                    .route(web::get().to(crate::web::rest_api::find_entities)),
            )
            .service(
                web::resource("/api/runPlan").route(web::get().to(crate::web::rest_api::run_plan)),
            )
            .service(web::resource("/api/plans").route(web::get().to(crate::web::rest_api::plans)))
            .service(
                web::resource("/api/dotEntityGraph")
                    .route(web::get().to(crate::web::rest_api::web_entity_graph)),
            )
            .service(
                web::resource("/api/dotTaskGraph")
                    .route(web::get().to(crate::web::rest_api::web_task_graph)),
            )
            .service(
                web::resource("/api/dotFlowGraph")
                    .route(web::get().to(crate::web::rest_api::web_flow_graph)),
            )
            .service(
                web::resource("/api/initiatePlan")
                    .route(web::get().to(crate::web::rest_api::execute_taskgraph)),
            )
            .service(
                web::resource("/api/findTiles")
                    .route(web::get().to(crate::web::rest_api::find_tiles)),
            )
            .service(
                web::resource("/api/planPath")
                    .route(web::get().to(crate::web::rest_api::plan_path)),
            )
            .service(
                web::resource("/api/findOffshorePumpPlacementOptions").route(
                    web::get().to(crate::web::rest_api::find_offshore_pump_placement_options),
                ),
            )
            .service(
                web::resource("/api/inventoryContentsAt")
                    .route(web::get().to(crate::web::rest_api::inventory_contents_at)),
            )
            .service(
                web::resource("/api/{player_id}/move")
                    .route(web::get().to(crate::web::rest_api::move_player)),
            )
            .service(
                web::resource("/api/{player_id}/playerInfo")
                    .route(web::get().to(crate::web::rest_api::player_info)),
            )
            .service(
                web::resource("/api/{player_id}/placeEntity")
                    .route(web::get().to(crate::web::rest_api::place_entity)),
            )
            .service(
                web::resource("/api/{player_id}/cheatItem")
                    .route(web::get().to(crate::web::rest_api::cheat_item)),
            )
            .service(
                web::resource("/api/cheatTechnology")
                    .route(web::get().to(crate::web::rest_api::cheat_technology)),
            )
            .service(
                web::resource("/api/cheatAllTechnologies")
                    .route(web::get().to(crate::web::rest_api::cheat_all_technologies)),
            )
            .service(
                web::resource("/api/{player_id}/insertToInventory")
                    .route(web::get().to(crate::web::rest_api::insert_to_inventory)),
            )
            .service(
                web::resource("/api/{player_id}/removeFromInventory")
                    .route(web::get().to(crate::web::rest_api::remove_from_inventory)),
            )
            .service(
                web::resource("/api/players")
                    .route(web::get().to(crate::web::rest_api::all_players)),
            )
            .service(
                web::resource("/api/itemPrototypes")
                    .route(web::get().to(crate::web::rest_api::item_prototypes)),
            )
            .service(
                web::resource("/api/entityPrototypes")
                    .route(web::get().to(crate::web::rest_api::entity_prototypes)),
            )
            .service(
                web::resource("/api/serverSave")
                    .route(web::get().to(crate::web::rest_api::server_save)),
            )
            .service(
                web::resource("/api/addResearch")
                    .route(web::get().to(crate::web::rest_api::add_research)),
            )
            .service(
                web::resource("/api/storeMapData")
                    .route(web::post().to(crate::web::rest_api::store_map_data)),
            )
            .service(
                web::resource("/api/retrieveMapData")
                    .route(web::get().to(crate::web::rest_api::retrieve_map_data)),
            )
            .service(
                web::resource("/api/{player_id}/placeBlueprint")
                    .route(web::get().to(crate::web::rest_api::place_blueprint)),
            )
            .service(
                web::resource("/api/{player_id}/reviveGhost")
                    .route(web::get().to(crate::web::rest_api::revive_ghost)),
            )
            .service(
                web::resource("/api/{player_id}/cheatBlueprint")
                    .route(web::get().to(crate::web::rest_api::cheat_blueprint)),
            )
            .service(
                web::resource("/api/parseBlueprint")
                    .route(web::get().to(crate::web::rest_api::parse_blueprint)),
            )
            .service(
                web::resource("/api/recipes")
                    .route(web::get().to(crate::web::rest_api::all_recipes)),
            )
            .service(
                web::resource("/api/playerForce")
                    .route(web::get().to(crate::web::rest_api::player_force)),
            )
            .service(
                web::resource("/api/allForces")
                    .route(web::get().to(crate::web::rest_api::all_forces)),
            )
            .service(
                web::resource("/api/{player_id}/mine")
                    .route(web::get().to(crate::web::rest_api::mine)),
            )
            .service(
                web::resource("/api/{player_id}/craft")
                    .route(web::get().to(crate::web::rest_api::craft)),
            )
            .service(
                web::resource("/api/{player_id}/craft")
                    .route(web::get().to(crate::web::rest_api::craft)),
            )
            // .service(crate::web::resource("/graphiql").route(web::get().to(graphiql)))
            .service(web::resource("/status").route(web::get().to(status)))
            // .service(crate::web::resource("/playground").route(web::get().to(playground)))
            // .service(crate::web::resource("/types.d.ts").route(web::get().to(type_d_ts)))
            // .service(crate::web::resource("/schema.graphql").route(web::get().to(schema_graphql)))
            .service(fs::Files::new("/", frontend_path).index_file("index.html"))
    })
    .bind(format!("0.0.0.0:{}", port))
    .expect("failed to bind")
    .run()
    .await
    .unwrap();
}
