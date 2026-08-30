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

/// A configured `workspace_path` that is neither empty nor absolute.
///
/// Joining a relative workspace against the process's working directory is
/// how the same configured value came to mean different directories to the
/// desktop app, the CLI and the HTTP server, so it is refused rather than
/// resolved.
#[allow(unused_assignments)] // false positive: the field is used by the derive
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("workspace_path must be absolute, got: {path}")]
#[diagnostic(
    code(factorio::workspace::relative),
    help("set settings.factorio.workspace_path to an absolute path, or leave it empty to use the data-local workspace")
)]
pub struct RelativeWorkspacePath {
    pub path: String,
}

/// Resolves a configured `workspace_path` to the directory a run must use:
/// empty means [`workspace_dir`], absolute means itself, and relative is an
/// error.
///
/// The rule itself is old; what is new is that there is one copy of it. It was
/// written out separately in `load_app_settings`, in the `serve` command's
/// scripts bootstrap and in the server's `scripts_root_path`, and the copy
/// that did *not* exist was the one `POST /api/v1/instance/start` needed: it
/// handed the raw settings string to `setup_factorio_instance`, which rejects
/// an empty one outright, so a browser could list and edit scripts under the
/// data-local workspace and then fail to start Factorio in it.
///
/// Deliberately does not create the directory. Whether a missing workspace is
/// an error or something to bootstrap differs per caller, and the callers that
/// create it (`Context::new`, `ensure_scripts_dir`) already do so knowingly.
pub fn resolve_workspace(configured: &str) -> Result<PathBuf, RelativeWorkspacePath> {
    let resolved = if configured.is_empty() {
        workspace_dir()
    } else {
        PathBuf::from(configured)
    };
    if resolved.is_relative() {
        return Err(RelativeWorkspacePath {
            path: resolved.display().to_string(),
        });
    }
    Ok(resolved)
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

    /// The three properties every caller of `resolve_workspace` depends on.
    /// An unconfigured install (`workspace_path = ""`) is the interesting one:
    /// it is the default, and the start route used to reject it while the
    /// scripts routes resolved it here.
    #[test]
    fn an_unconfigured_workspace_resolves_to_the_data_local_one() {
        assert_eq!(
            resolve_workspace("").expect("empty resolves"),
            workspace_dir()
        );
    }

    #[test]
    fn an_absolute_workspace_is_taken_as_given() {
        assert_eq!(
            resolve_workspace("/srv/factorio").expect("absolute resolves"),
            PathBuf::from("/srv/factorio")
        );
    }

    /// Never joined against the process's working directory: that is how one
    /// configured value came to mean different directories to different
    /// callers.
    #[test]
    fn a_relative_workspace_is_refused_rather_than_joined_to_the_cwd() {
        let err = resolve_workspace("./configured/workspace")
            .expect_err("a relative workspace_path must be refused");
        assert!(
            err.to_string().contains("must be absolute"),
            "unexpected error: {err}"
        );
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
