use crate::constants::WORKSPACE_FOLDERNAME;
use std::path::{Path, PathBuf};

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
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("workspace_path must be absolute, got: {path}")]
#[diagnostic(
    code(factorio::workspace::relative),
    help("set settings.factorio.workspace_path to an absolute path, or leave it empty to use the data-local workspace")
)]
pub struct RelativeWorkspacePath {
    pub path: String,
}

/// A workspace path that has been through [`resolve_workspace`]: absolute,
/// and therefore independent of the process's working directory.
///
/// The field is private, so `resolve_workspace` is the only way to obtain
/// one -- filesystem entry points that must never see an unresolved
/// `workspace_path` (such as [`crate::scripts::ensure_scripts_dir`]) demand
/// this type instead of a bare `&Path`, which makes the resolve-before-use
/// rule a compile error to skip rather than a convention to remember.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkspace(PathBuf);

impl ResolvedWorkspace {
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// A workspace directory whose name is not valid UTF-8.
///
/// `FactorioSettings::workspace_path` is a `str`, so such a path cannot be
/// carried in the settings at all. The only way to reach one is the
/// empty-`workspace_path` default resolving to a non-UTF-8 data-local
/// directory. Refused rather than run through `to_string_lossy`, which would
/// silently name a *different* directory than the one the default points at --
/// the policy `POST /api/v1/instance/start` already applied, now applied by
/// everything that turns a resolved workspace back into settings.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("workspace path is not valid utf-8: {path}")]
#[diagnostic(
    code(factorio::workspace::not_utf8),
    help("set settings.factorio.workspace_path to an absolute, utf-8 path")
)]
pub struct NonUtf8WorkspacePath {
    pub path: String,
}

/// Fills in the default for an unconfigured `workspace_path`: empty means
/// [`workspace_dir`], anything else is taken as written.
///
/// The one place that decides what an *unset* workspace means. Deliberately
/// separate from [`resolve_workspace`], which decides the other question --
/// whether a configured path is usable at all. Settings **load** may only ask
/// the first: rejecting at load put the refusal in front of `config show` and
/// `config init --force`, i.e. in front of every way a user has of repairing
/// the very setting being refused.
#[must_use]
pub fn fill_workspace_default(configured: &str) -> PathBuf {
    if configured.is_empty() {
        workspace_dir()
    } else {
        PathBuf::from(configured)
    }
}

/// Turns a resolved workspace back into the `str` the settings carry, refusing
/// a name that is not UTF-8 rather than mangling it. See
/// [`NonUtf8WorkspacePath`].
pub fn workspace_to_string(path: PathBuf) -> Result<String, NonUtf8WorkspacePath> {
    path.into_os_string()
        .into_string()
        .map_err(|raw| NonUtf8WorkspacePath {
            path: PathBuf::from(raw).display().to_string(),
        })
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
/// Called at the point of **use**, not at settings load -- asking at load turned
/// every subcommand into a startup gate and took `config init --force`, the
/// documented repair path, down with it.
///
/// Returns [`ResolvedWorkspace`] rather than a bare `PathBuf` so that
/// "resolved" is a fact the type carries, not a convention a caller might
/// skip. An earlier version of this comment enumerated the call sites that
/// ask versus the ones that should -- that list was wrong twice, because a
/// list decays silently. `scripts::ensure_scripts_dir` now demands
/// `&ResolvedWorkspace`, so a caller that never resolved simply cannot reach
/// it; there is no enumeration left to go stale.
///
/// Deliberately does not create the directory. Whether a missing workspace is
/// an error or something to bootstrap differs per caller, and the callers that
/// create it (`Context::new`, `ensure_scripts_dir`) already do so knowingly.
pub fn resolve_workspace(configured: &str) -> Result<ResolvedWorkspace, RelativeWorkspacePath> {
    let resolved = fill_workspace_default(configured);
    if resolved.is_relative() {
        return Err(RelativeWorkspacePath {
            path: resolved.display().to_string(),
        });
    }
    Ok(ResolvedWorkspace(resolved))
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
            resolve_workspace("").expect("empty resolves").as_path(),
            workspace_dir()
        );
    }

    #[test]
    fn an_absolute_workspace_is_taken_as_given() {
        assert_eq!(
            resolve_workspace("/srv/factorio")
                .expect("absolute resolves")
                .as_path(),
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

    /// `fill_workspace_default` answers the *other* question, and must answer
    /// it for a relative path too: settings load calls this and only this, so a
    /// user with a relative `workspace_path` can still run `config show` and
    /// `config init --force` to see and repair it. Rejecting here is the
    /// regression this split exists to prevent.
    #[test]
    fn filling_the_default_does_not_judge_a_relative_path() {
        assert_eq!(
            fill_workspace_default("relative-ws"),
            PathBuf::from("relative-ws")
        );
        assert_eq!(fill_workspace_default(""), workspace_dir());
        assert_eq!(
            fill_workspace_default("/srv/factorio"),
            PathBuf::from("/srv/factorio")
        );
    }

    /// The refusal is only useful if the sentence saying how to fix it survives
    /// the trip into a `miette::Report`. Converting with `miette!("{err}")` or
    /// `.into_diagnostic()` drops `help` and `code`, leaving the user with
    /// "must be absolute" and nothing else -- so callers use a bare `?`, which
    /// goes through `From<E: Diagnostic> for Report`.
    #[test]
    fn the_relative_refusal_keeps_its_fix_instruction_as_a_report() {
        let err = resolve_workspace("relative-ws").expect_err("relative is refused");
        let report = miette::Report::new(err);
        let diagnostic: &dyn miette::Diagnostic = report.as_ref();
        let help = diagnostic
            .help()
            .map(|help| help.to_string())
            .expect("the report must carry the help text, not just the message");
        assert!(
            help.contains("absolute path"),
            "unexpected help text: {help}"
        );
        let code = diagnostic
            .code()
            .map(|code| code.to_string())
            .expect("the report must carry the diagnostic code");
        assert_eq!(code, "factorio::workspace::relative");
    }

    /// One policy for a non-UTF-8 workspace, shared by every caller that turns
    /// a resolved path back into settings: refuse it. `to_string_lossy` would
    /// substitute replacement characters and silently name a *different*
    /// directory than the one configured -- which is exactly what the start
    /// route already refused to do, three files away, while the two settings
    /// loaders did it happily.
    #[cfg(unix)]
    #[test]
    fn a_non_utf8_workspace_is_refused_rather_than_mangled() {
        use std::os::unix::ffi::OsStringExt;
        let raw = PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', b'w', 0xff, b's']));
        let lossy = raw.to_string_lossy().into_owned();
        let err = workspace_to_string(raw).expect_err("a non-utf-8 workspace must be refused");
        assert!(
            err.to_string().contains("not valid utf-8"),
            "unexpected error: {err}"
        );
        assert!(
            !lossy.as_bytes().contains(&0xff),
            "the lossy form really does differ from the bytes, so this test has teeth"
        );
    }

    #[test]
    fn a_utf8_workspace_survives_the_round_trip() {
        assert_eq!(
            workspace_to_string(PathBuf::from("/srv/factorio")).expect("utf-8 converts"),
            "/srv/factorio"
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
