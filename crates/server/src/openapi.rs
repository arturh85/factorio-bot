use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "factorio-bot", description = "factorio-bot HTTP API"),
    tags(
        (name = "Query", description = "World queries"),
        (name = "Control", description = "Bot control"),
        (name = "Place", description = "Entity placement"),
        (name = "Cheat", description = "Cheats"),
        (name = "Inventory", description = "Inventory manipulation"),
        (name = "Admin", description = "Server administration"),
        (name = "Research", description = "Research"),
    )
)]
pub struct ApiDoc;
