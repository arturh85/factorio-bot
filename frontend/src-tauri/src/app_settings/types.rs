#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(dead_code)]

use std::borrow::Cow;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(non_camel_case_types)]
pub struct AppSettings {
    pub client_count: i64,
    pub factorio_version: Cow<'static, str>,
    pub workspace_path: Cow<'static, str>,
}

pub const APP_SETTINGS_DEFAULT: AppSettings = AppSettings {
    client_count: 2,
    factorio_version: Cow::Borrowed("1.1.36"),
    workspace_path: Cow::Borrowed(""),
};
