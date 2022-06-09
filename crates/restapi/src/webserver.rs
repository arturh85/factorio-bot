use crate::settings::RestApiSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use miette::{IntoDiagnostic, Result};
use rocket::data::{Limits, ToByteUnit};
use rocket::http::Status;
use rocket::response::status;
use rocket::Request;
use rocket_okapi::rapidoc::{make_rapidoc, GeneralConfig, RapiDocConfig};
use rocket_okapi::settings::UrlObject;
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

// #[get("/")]
// fn index() -> RawHtml<&'static str> {
// use rocket::response::content::RawHtml;
//     RawHtml(include_str!("rapidoc.html"))
// }

pub async fn start(
    settings: RestApiSettings,
    instance_state: SharedFactorioInstance,
) -> Result<()> {
    let port = settings.port;
    let figment = rocket::Config::figment()
        .merge(("port", port))
        .merge(("limits", Limits::new().limit("json", 2.mebibytes())));
    let _rocket = rocket::custom(figment)
        .manage(Arc::new(RwLock::new(settings)))
        .manage(instance_state)
        // .mount("/", rocket::routes![index])
        .mount(
            "/",
            rocket_okapi::openapi_get_routes![
                crate::restapi::find_entities,
                crate::restapi::plan_path
            ],
        )
        .mount(
            "/swagger-ui/",
            make_swagger_ui(&SwaggerUIConfig {
                url: "../openapi.json".to_owned(),
                ..SwaggerUIConfig::default()
            }),
        )
        .mount(
            "/",
            make_rapidoc(&RapiDocConfig {
                general: GeneralConfig {
                    spec_urls: vec![UrlObject::new("myapi", "/openapi.json")],
                    ..GeneralConfig::default()
                },
                ..RapiDocConfig::default()
            }),
        )
        .register("/", catchers![general_not_found, default_catcher])
        .launch()
        .await
        .into_diagnostic()?;
    // .map_err(anyhow::Error::from)?;
    Ok(())
}
