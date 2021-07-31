#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(dead_code)]

use std::borrow::Cow;

#[derive(Debug, Clone, typescript_definitions::TypeScriptify, serde::Serialize, serde::Deserialize)]
#[allow(non_camel_case_types)]
pub struct AppSettings {
    pub client_count: i64,
    pub factorio_version: Cow<'static, str>,
    pub rcon_pass: Cow<'static, str>,
    pub rcon_port: i64,
    pub workspace_path: Cow<'static, str>,
}

pub const APP_SETTINGS_DEFAULT: AppSettings = AppSettings {
    client_count: 2,
    factorio_version: Cow::Borrowed("1.1.36"),
    rcon_pass: Cow::Borrowed("foobar"),
    rcon_port: 4321,
    workspace_path: Cow::Borrowed(""),
};
