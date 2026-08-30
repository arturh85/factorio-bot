use crate::constants::WORKSPACE_FOLDERNAME;
use std::path::PathBuf;

pub const APP_SETTINGS_FILENAME: &str = "AppSettings.toml";

/// Name of the shipped binary. Hardcoded rather than taken from
/// `env!("CARGO_PKG_NAME")`, which would resolve to this crate's name and
/// rename the user-visible data directory.
const APP_NAME: &str = "factorio-bot";

pub fn data_local_dir() -> PathBuf {
    dirs_next::data_local_dir()
        .expect("no local data directory available")
        .join(format!(
            "{}{}",
            APP_NAME,
            if cfg!(debug_assertions) { "-dev" } else { "" }
        ))
}

pub fn settings_file() -> PathBuf {
    data_local_dir().join(APP_SETTINGS_FILENAME)
}

pub fn workspace_dir() -> PathBuf {
    data_local_dir().join(WORKSPACE_FOLDERNAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The data directory is named after the binary, not the crate this code
    /// lives in. Moving this module must not rename it: doing so orphans every
    /// existing user's AppSettings.toml and workspace.
    #[test]
    fn data_local_dir_is_named_after_the_binary() {
        let dir = data_local_dir();
        let name = dir
            .file_name()
            .expect("data dir has a final component")
            .to_str()
            .expect("data dir name is utf-8");
        let expected = if cfg!(debug_assertions) {
            "factorio-bot-dev"
        } else {
            "factorio-bot"
        };
        assert_eq!(name, expected);
    }

    #[test]
    fn settings_file_lives_in_the_data_dir() {
        assert_eq!(
            settings_file().parent().expect("has a parent"),
            data_local_dir()
        );
        assert_eq!(
            settings_file()
                .file_name()
                .expect("has a name")
                .to_str()
                .expect("utf-8"),
            APP_SETTINGS_FILENAME
        );
    }
}
