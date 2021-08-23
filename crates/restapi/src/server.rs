use rocket::data::{Limits, ToByteUnit};
// use rocket_okapi::swagger_ui::*;

pub async fn start_webserver() -> anyhow::Result<()> {
    let figment = rocket::Config::figment()
        .merge(("port", 1111))
        .merge(("limits", Limits::new().limit("json", 2.mebibytes())));
    rocket::custom(figment)
        // .mount("/", routes_with_openapi![find_entities])
        .launch()
        .await
        .map_err(|err| anyhow::Error::from(err));
    // rocket::build()
    // .mount("/", routes_with_openapi![find_entities])
    // .mount(
    //     "/swagger-ui/",
    //     make_swagger_ui(&SwaggerUIConfig {
    //         url: "../openapi.json".to_owned(),
    //         ..SwaggerUIConfig::default()
    //     }),
    // )
    // .launch()
    // .await?;
    info!("build done");
    Ok(())
}
