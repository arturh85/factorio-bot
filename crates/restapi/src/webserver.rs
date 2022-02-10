use factorio_bot_core::process::process_control::InstanceState;
use factorio_bot_core::settings::AppSettings;
use miette::Result;
use rocket::data::{Limits, ToByteUnit};
use rocket::http::Status;
use rocket::response::content::Html;
use rocket::response::status;
use rocket::Request;
use rocket_okapi::swagger_ui::*;
use std::sync::Arc;
use tokio::sync::RwLock;

#[catch(default)]
fn default_catcher(status: Status, req: &Request<'_>) -> status::Custom<String> {
    let msg = format!("{} ({})", status, req.uri());
    status::Custom(status, msg)
}

#[catch(404)]
fn general_not_found() -> String {
    "Not found".into()
}

#[get("/")]
fn index() -> Html<&'static str> {
    Html(include_str!("rapidoc.html"))
}

pub async fn start(
    app_settings: Arc<RwLock<AppSettings>>,
    instance_state: Arc<RwLock<Option<InstanceState>>>,
) -> Result<()> {
    let port = app_settings.read().await.restapi_port;
    let figment = rocket::Config::figment()
        .merge(("port", port))
        .merge(("limits", Limits::new().limit("json", 2.mebibytes())));
    rocket::custom(figment)
        .manage(app_settings)
        .manage(instance_state)
        .mount("/", rocket::routes![index])
        .mount(
            "/",
            rocket_okapi::routes_with_openapi![
                crate::rest_api::find_entities,
                crate::rest_api::plan_path
            ],
        )
        .mount(
            "/swagger-ui/",
            make_swagger_ui(&SwaggerUIConfig {
                url: "../openapi.json".to_owned(),
                ..SwaggerUIConfig::default()
            }),
        )
        .register("/", catchers![general_not_found, default_catcher])
        .launch()
        .await
        .unwrap();
    // .map_err(anyhow::Error::from)?;
    Ok(())
}
