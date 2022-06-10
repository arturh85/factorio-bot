#[derive(
    Debug, Clone, typescript_definitions::TypeScriptify, serde::Serialize, serde::Deserialize,
)]
pub struct RestApiSettings {
    pub port: i64,
}

impl Default for RestApiSettings {
    fn default() -> Self {
        RestApiSettings { port: 7492 }
    }
}
