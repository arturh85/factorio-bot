use crate::run_lua;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::scripts::{resolve_script_path, ScriptPathError};
use factorio_bot_scripting::OutputSink;
use miette::{miette, IntoDiagnostic, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Why a script could not be run.
///
/// The path cases are kept distinct from everything else because the HTTP
/// layer answers them differently -- a missing script is the caller's typo
/// (404), a script outside the root is a refused traversal (400), and a Lua
/// failure is neither. Flattening these into a string forced the server to
/// re-derive them by matching on message text, which breaks silently the
/// first time someone rewords an error.
#[derive(Debug, thiserror::Error)]
pub enum RunScriptError {
    #[error(transparent)]
    Path(#[from] ScriptPathError),
    #[error("unknown scripting file extension: {0}")]
    UnknownExtension(String),
    #[error("path is not a file: {0}")]
    NotAFile(String),
    /// Deliberately not `#[error(transparent)] Run(#[from] miette::Report)` as
    /// first sketched: `miette::Report` (like `anyhow::Error`) does not
    /// implement `std::error::Error`, so thiserror cannot make it a `source`
    /// and that form does not compile. `Display` is forwarded by hand and the
    /// `From` conversion is written out below instead.
    #[error("{0}")]
    Run(miette::Report),
}

impl From<miette::Report> for RunScriptError {
    fn from(report: miette::Report) -> Self {
        RunScriptError::Run(report)
    }
}

impl RunScriptError {
    /// Collapses back into a `miette::Report` for the callers that only
    /// display the error -- the GUI command, the CLI and the REPL. The `Run`
    /// arm is unwrapped rather than re-wrapped so a Lua failure keeps its
    /// source snippet and diagnostic code; only the path/extension arms, which
    /// carry no diagnostic of their own, become plain messages.
    pub fn into_report(self) -> miette::Report {
        match self {
            RunScriptError::Run(report) => report,
            other => miette!("{other}"),
        }
    }
}

/// A script name resolved to a file this crate knows how to run.
#[derive(Debug, Clone)]
pub struct ResolvedScript {
    /// Canonical path, guaranteed to be inside the scripts root.
    pub path: PathBuf,
    /// The scripting language chosen by the file's extension.
    pub language: &'static str,
}

/// Turns a client-supplied script name into a runnable file, or says exactly
/// why it is not one.
///
/// Split out of [`run_script_file`] because the HTTP server has to answer the
/// "is this a script I can run" question *before* it accepts the request: it
/// returns `202` and runs the script on a detached task, so by the time
/// `run_script_file` fails there is no status code left to put the failure in.
/// Sharing this function is what keeps the pre-flight check and the run from
/// drifting apart -- the alternative, re-implementing the checks in the
/// handler, is how the two would come to disagree about which names are
/// runnable.
pub fn resolve_script(
    scripts_root: &Path,
    requested: &str,
) -> std::result::Result<ResolvedScript, RunScriptError> {
    let path = resolve_script_path(scripts_root, requested)?;
    if !path.is_file() {
        return Err(RunScriptError::NotAFile(requested.to_owned()));
    }
    let language = language_by_filename(requested)
        .ok_or_else(|| RunScriptError::UnknownExtension(requested.to_owned()))?;
    Ok(ResolvedScript { path, language })
}

/// Maps a filename to the scripting language that runs it.
pub fn language_by_filename(filename: &str) -> Option<&'static str> {
    match Path::new(filename).extension()?.to_str()? {
        "lua" => Some("lua"),
        // "rhai" => Some("rhai"),
        // "rn" => Some("rune"),
        _ => None,
    }
}

/// Runs a script named relative to `scripts_root`.
///
/// `scripts_root` is passed in rather than looked up: the old version read
/// global settings and then consulted `factorio_bot_core::scripts::scripts_dir`,
/// which prefers `./scripts` relative to the process's working directory. That
/// made the same name mean different files to the editor (which writes through
/// the workspace-only `scripts_root`) and to the executor (plan 3, finding I3).
/// There is now exactly one resolution, and its root is the caller's to state.
///
/// The bound is enforced by [`resolve_script_path`], which canonicalises and
/// then checks `starts_with`, replacing a `path.contains("..")` substring test
/// that both rejected legitimate names like `my..script.lua` and let symlinks
/// out of the root.
///
/// `sink`, when present, receives the script's output line by line while it
/// runs; the full transcript is returned either way.
pub async fn run_script_file(
    planner: &mut Planner,
    scripts_root: &Path,
    requested: &str,
    bot_count: u8,
    sink: Option<Arc<dyn OutputSink>>,
) -> std::result::Result<(String, String), RunScriptError> {
    let ResolvedScript { path, language } = resolve_script(scripts_root, requested)?;
    let code = std::fs::read_to_string(&path).into_diagnostic()?;
    // The resolved absolute path, not the request: `include` resolves relative
    // to the script's own directory, and errors should name the real file.
    let filename = path.to_string_lossy().into_owned();
    match language {
        "lua" => Ok(run_lua(
            planner,
            &code,
            Some(&filename),
            scripts_root,
            bot_count,
            sink,
        )
        .await
        .map(|outcome| outcome.1)?),
        other => Err(RunScriptError::Run(miette!(
            "unknown language: \"{other}\""
        ))),
    }
}

/// Runs code that has no file behind it (the editor's "run selection").
///
/// `scripts_root` is still required: it is the sandbox boundary handed to the
/// interpreter, so inline code reaches exactly the same files a saved script
/// would.
pub async fn run_script(
    planner: &mut Planner,
    language: &str,
    code: &str,
    scripts_root: &Path,
    bot_count: u8,
    sink: Option<Arc<dyn OutputSink>>,
) -> Result<(String, String)> {
    match language {
        "lua" => run_lua(planner, code, None, scripts_root, bot_count, sink)
            .await
            .map(|outcome| outcome.1),
        other => Err(miette!("unknown language: \"{other}\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::test_utils::fixture_world;
    use std::path::PathBuf;

    /// A scripts root with a real file *outside* it, as a sibling of the root.
    /// Escape attempts below resolve to that file, so `canonicalize` succeeds
    /// and the traversal guard is what rejects them — not a coincidental
    /// "file not found", which would let a broken guard pass.
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("scripts");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(dir.path().join("outside.lua"), "print(\"outside\")").expect("write");
        let root = std::fs::canonicalize(&root).expect("canonicalize");
        (dir, root)
    }

    #[tokio::test]
    async fn a_script_is_read_from_the_given_root_not_from_the_working_directory() {
        // The regression this file exists to prevent: `scripts_dir` preferred
        // `./scripts` (and `../../scripts`) relative to the process CWD over
        // the workspace, so the editor and the executor resolved the same name
        // to different files.
        let (_dir, root) = fixture();
        std::fs::write(root.join("hello.lua"), "print(\"from the root\")").expect("write");

        // A decoy with the same name under the process's working directory. If
        // resolution ever consults the CWD again, this is what would run.
        let cwd_scripts = std::env::current_dir().expect("cwd").join("scripts");
        assert!(
            !cwd_scripts.join("hello.lua").exists(),
            "test precondition: no ./scripts/hello.lua in the checkout"
        );

        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (stdout, _stderr) = run_script_file(&mut planner, &root, "/hello.lua", 1, None)
            .await
            .expect("run_script_file failed");
        assert!(stdout.contains("from the root"), "stdout was {stdout:?}");
    }

    #[tokio::test]
    async fn a_script_outside_the_root_is_refused() {
        let (dir, root) = fixture();
        // Sanity check the fixture: the escape attempt must land on a real
        // file that is genuinely outside the root.
        let outside = std::fs::canonicalize(dir.path().join("outside.lua")).expect("canonicalize");
        assert!(
            !outside.starts_with(&root),
            "fixture bug: outside.lua ended up under root"
        );

        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let err = run_script_file(&mut planner, &root, "../outside.lua", 1, None)
            .await
            .expect_err("should be refused");
        assert!(format!("{err}").contains("escapes"), "error was {err}");
    }

    /// A substring check on ".." rejects this legitimate name; the
    /// canonicalising check accepts it. Without this, "refuse everything with
    /// two dots in it" would pass the escape test above.
    #[tokio::test]
    async fn a_script_whose_name_contains_two_dots_still_runs() {
        let (_dir, root) = fixture();
        std::fs::write(root.join("my..script.lua"), "print(\"dotted\")").expect("write");

        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (stdout, _stderr) = run_script_file(&mut planner, &root, "/my..script.lua", 1, None)
            .await
            .expect("run_script_file failed");
        assert!(stdout.contains("dotted"), "stdout was {stdout:?}");
    }

    #[tokio::test]
    async fn a_non_script_extension_is_refused() {
        let (_dir, root) = fixture();
        std::fs::write(root.join("notes.txt"), "hello").expect("write");

        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let err = run_script_file(&mut planner, &root, "notes.txt", 1, None)
            .await
            .expect_err("should be refused");
        assert!(format!("{err}").contains("extension"), "error was {err}");
    }

    /// `resolve_script_path` deliberately resolves `""` and `"/"` to the root
    /// itself and does not care whether the result is a file, so the caller
    /// has to. Without the check a directory reaches `read_to_string`, whose
    /// "Is a directory" is not an answer a script author can act on.
    #[tokio::test]
    async fn a_directory_is_refused() {
        let (_dir, root) = fixture();
        std::fs::create_dir_all(root.join("sub.lua")).expect("mkdir");

        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let err = run_script_file(&mut planner, &root, "/sub.lua", 1, None)
            .await
            .expect_err("should be refused");
        assert!(format!("{err}").contains("not a file"), "error was {err}");
    }

    /// The distinction the HTTP layer turns into 404 (a typo) versus 400 (a
    /// refused traversal).
    ///
    /// Asserted on the *variant*, never on the message. Before this type
    /// existed `run_script_file` flattened both into a string with
    /// `miette!("{err}")`, and the only way for a caller to recover the split
    /// was to match on message text -- which turns rewording an error into a
    /// silent status-code regression that nothing fails on. A test that
    /// asserted the text would pass just as happily against the flattened
    /// version, which is exactly why this one does not.
    #[test]
    fn a_missing_script_and_an_escaping_script_are_distinguishable_arms() {
        let (dir, root) = fixture();
        // Same fixture requirement as `a_script_outside_the_root_is_refused`:
        // the escape attempt must land on a file that really exists outside
        // the root, or `NotFound` answers it and the split is never exercised.
        let outside = std::fs::canonicalize(dir.path().join("outside.lua")).expect("canonicalize");
        assert!(
            !outside.starts_with(&root),
            "fixture bug: outside.lua ended up under root"
        );

        assert!(
            matches!(
                resolve_script(&root, "/nope.lua"),
                Err(RunScriptError::Path(ScriptPathError::NotFound { .. }))
            ),
            "a missing script must stay distinguishable as NotFound"
        );
        assert!(
            matches!(
                resolve_script(&root, "../outside.lua"),
                Err(RunScriptError::Path(ScriptPathError::EscapesRoot { .. }))
            ),
            "a traversal must stay distinguishable as EscapesRoot"
        );
    }

    /// The non-path arms, so a caller matching on `Path` cannot accidentally
    /// be handed one of these instead.
    #[test]
    fn a_directory_and_an_unknown_extension_are_their_own_arms() {
        let (_dir, root) = fixture();
        std::fs::create_dir_all(root.join("sub.lua")).expect("mkdir");
        std::fs::write(root.join("notes.txt"), "hello").expect("write");

        assert!(
            matches!(
                resolve_script(&root, "/sub.lua"),
                Err(RunScriptError::NotAFile(_))
            ),
            "a directory is not a missing file"
        );
        assert!(
            matches!(
                resolve_script(&root, "/notes.txt"),
                Err(RunScriptError::UnknownExtension(_))
            ),
            "a file with no interpreter is not a missing file"
        );
    }

    #[test]
    fn language_by_filename_knows_lua_and_nothing_else() {
        assert_eq!(language_by_filename("a.lua"), Some("lua"));
        assert_eq!(language_by_filename("a.txt"), None);
        assert_eq!(language_by_filename("noextension"), None);
    }
}
