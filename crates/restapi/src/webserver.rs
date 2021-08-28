use async_std::sync::{Arc, RwLock};
use factorio_bot_core::factorio::process_control::InstanceState;
use factorio_bot_core::settings::AppSettings;
use rocket::data::{Limits, ToByteUnit};
use rocket::http::Status;
use rocket::response::status;
use rocket::Request;
use rocket_okapi::swagger_ui::*;

#[catch(default)]
fn default_catcher(status: Status, req: &Request<'_>) -> status::Custom<String> {
    let msg = format!("{} ({})", status, req.uri());
    status::Custom(status, msg)
}

#[catch(404)]
fn general_not_found() -> String {
    "Not found".into()
}

pub async fn start(
    app_settings: Arc<RwLock<AppSettings>>,
    instance_state: Arc<RwLock<Option<InstanceState>>>,
) -> anyhow::Result<()> {
    let port = app_settings.read().await.restapi_port;
    let figment = rocket::Config::figment()
        .merge(("port", port))
        .merge(("limits", Limits::new().limit("json", 2.mebibytes())));
    rocket::custom(figment)
        .manage(app_settings)
        .manage(instance_state)
        .mount(
            "/",
            routes_with_openapi![crate::rest_api::find_entities, crate::rest_api::test],
        )
        .register("/", catchers![general_not_found, default_catcher])
        .mount(
            "/swagger-ui/",
            make_swagger_ui(&SwaggerUIConfig {
                url: "../openapi.json".to_owned(),
                ..SwaggerUIConfig::default()
            }),
        )
        .launch()
        .await
        .map_err(anyhow::Error::from)?;
    Ok(())
}
