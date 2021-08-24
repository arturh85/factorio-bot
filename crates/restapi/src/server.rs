use async_std::sync::{Arc, RwLock};
use factorio_bot_core::factorio::process_control::InstanceState;
use factorio_bot_core::settings::AppSettings;
use rocket::data::{Limits, ToByteUnit};
use rocket_okapi::swagger_ui::*;

pub async fn start_webserver(
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
        .mount("/", routes_with_openapi![crate::rest_api::find_entities])
        .mount(
            "/swagger-ui/",
            make_swagger_ui(&SwaggerUIConfig {
                url: "../openapi.json".to_owned(),
                ..SwaggerUIConfig::default()
            }),
        )
        .launch()
        .await
        .map_err(|err| anyhow::Error::from(err))?;
    // rocket::build()
    // .mount("/", routes_with_openapi![find_entities])
    // .launch()
    // .await?;
    info!("build done");
    Ok(())
}
