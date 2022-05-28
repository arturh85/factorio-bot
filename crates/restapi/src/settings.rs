#[derive(
    Debug, Clone, typescript_definitions::TypeScriptify, serde::Serialize, serde::Deserialize,
)]
pub struct RestApiSettings {
    pub port: i64,
}

pub const RESTAPI_SETTINGS_DEFAULT: RestApiSettings = RestApiSettings { port: 7492 };
