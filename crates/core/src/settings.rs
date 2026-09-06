use std::borrow::Cow;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[allow(non_camel_case_types)]
pub struct FactorioSettings {
    pub client_count: u8,
    pub factorio_archive_path: Cow<'static, str>,
    // A map-exchange string, which encodes a seed **and** the map-gen and map
    // settings together. Empty means "Factorio's defaults for this version",
    // and that is the default on purpose.
    //
    // A seed reproduces a map only *under fixed settings*, so this field is
    // what can break that condition -- and it is not inert. On the paths that
    // read settings at all (`crates/server/src/manage/instance.rs`, the REPL's
    // `start`) a non-empty value is handed to `setup_factorio_instance`, which
    // parses it into `map-gen-settings.json` / `map-settings.json` and passes
    // both to `factorio --create`. The same seed then generates a *different
    // map*.
    //
    // It used to default to a 719-character string carried in from the
    // project's first commit (`0bb136c6`, Feb 2021, Factorio 1.x). No run in
    // this project ever applied it: `factorio-bot lua` and `start` take the
    // string only from `--map`, never from here, and both live workspaces
    // carry `map_exchange_string = ""`. So it was reachable only by a fresh
    // setup through the REPL or the REST API, which would have been silently
    // switched off defaults and off every number in the record. Emptied
    // 2026-09-06; the string is in this file's git history if anybody wants
    // that particular map back.
    //
    // A plain comment, not a doc comment, deliberately: `FactorioSettings`
    // derives `utoipa::ToSchema`, so a `///` here becomes a `description` in
    // the published OpenAPI spec and moves `app/src/api/openapi.snapshot.json`.
    // Nothing about the wire shape changed, so nothing should move.
    pub map_exchange_string: Cow<'static, str>,
    pub rcon_pass: Cow<'static, str>,
    pub rcon_port: u16,
    /// The game port the server listens on; `None` means Factorio's default
    /// 34197. Set it, with `rcon_port` and `workspace_path`, to run a second
    /// instance beside a live one.
    #[serde(default)]
    pub factorio_port: Option<u16>,
    pub recreate: bool,
    pub seed: Cow<'static, str>,
    pub workspace_path: Cow<'static, str>,
}

impl Default for FactorioSettings {
    fn default() -> Self {
        FactorioSettings {
            client_count: 2,
            factorio_archive_path: Cow::Borrowed(""),
            // Empty: Factorio's defaults for the installed version, which is what
            // every archived number in this project was measured on. See the field's
            // doc comment above -- a non-empty value here silently generates a
            // different map on the settings-reading start paths.
            map_exchange_string: Cow::Borrowed(""),
            rcon_pass: Cow::Borrowed("foobar"),
            rcon_port: 4321,
            factorio_port: None,
            recreate: false,
            seed: Cow::Borrowed(""),
            workspace_path: Cow::Borrowed(""),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
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

#[cfg(test)]
mod restapi_settings_tests {
    use super::RestApiSettings;

    #[test]
    fn defaults_match_the_shipped_configuration() {
        let settings = RestApiSettings::default();
        assert_eq!(settings.port, 7492);
        assert_eq!(settings.web_root, None);
    }
}
