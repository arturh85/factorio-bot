#[derive(
    Debug, Clone, typescript_definitions::TypeScriptify, serde::Serialize, serde::Deserialize,
)]
pub struct RestApiSettings {
    pub port: i64,
    /// Directory containing the built SPA. Relative paths resolve against the
    /// current working directory. When the directory does not exist, the server
    /// still starts and serves the API only.
    #[serde(default)]
    pub web_root: Option<String>,
}

impl Default for RestApiSettings {
    fn default() -> Self {
        RestApiSettings {
            port: 7492,
            web_root: None,
        }
    }
}
