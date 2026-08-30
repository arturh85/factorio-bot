use crate::run_lua;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::scripts::resolve_script_path;
use factorio_bot_scripting::OutputSink;
use miette::{miette, IntoDiagnostic, Result};
use std::path::Path;
use std::sync::Arc;

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
) -> Result<(String, String)> {
    let resolved = resolve_script_path(scripts_root, requested).map_err(|err| miette!("{err}"))?;
    if !resolved.is_file() {
        return Err(miette!("path is not a file: {requested}"));
    }
    let language = language_by_filename(requested)
        .ok_or_else(|| miette!("unknown scripting file extension: {requested}"))?;
    let code = std::fs::read_to_string(&resolved).into_diagnostic()?;
    // The resolved absolute path, not the request: `include` resolves relative
    // to the script's own directory, and errors should name the real file.
    let filename = resolved.to_string_lossy().into_owned();
    match language {
        "lua" => run_lua(
            planner,
            &code,
            Some(&filename),
            scripts_root,
            bot_count,
            sink,
        )
        .await
        .map(|outcome| outcome.1),
        other => Err(miette!("unknown language: \"{other}\"")),
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

    #[test]
    fn language_by_filename_knows_lua_and_nothing_else() {
        assert_eq!(language_by_filename("a.lua"), Some("lua"));
        assert_eq!(language_by_filename("a.txt"), None);
        assert_eq!(language_by_filename("noextension"), None);
    }
}
