use crate::error::{ApiResult, ErrorResponse};
use crate::extract::{ApiJson, ApiQuery};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use factorio_bot_core::scripts::resolve_script_path;
use factorio_bot_core::types::ScriptTreeNode;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ScriptPathQuery {
    /// Slash-separated path relative to the scripts root, e.g. `/` or
    /// `/sub/example.lua`. A leading slash is optional.
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ScriptContent {
    pub code: String,
}

/// Resolves the scripts root for this request from the current settings.
///
/// Deliberately does not call `factorio_bot_core::scripts::scripts_dir`:
/// that function checks `./scripts` and `../../scripts` relative to the
/// *server process's* current working directory before ever looking at
/// `workspace_path`. That is the right convenience for a developer running
/// `cargo repl` from a checkout, but wrong for an HTTP handler — a request
/// (or a test) that relies on `workspace_path` would silently be redirected
/// to whatever `./scripts` happens to resolve to relative to the server's
/// CWD instead, which in this repository's own checkout is a real
/// `scripts/` directory. Resolving `workspace_path/scripts` directly here
/// makes the settings value the single source of truth and keeps tests
/// (which point `workspace_path` at a temp directory) isolated from the
/// repository's real scripts.
///
/// `FactorioSettings::default()` leaves `workspace_path` as an empty
/// string, which would otherwise join down to the bare relative path
/// `"scripts"` and canonicalize *that* against the server process's CWD —
/// reintroducing, on an unconfigured install, exactly the CWD-dependent
/// hazard this function exists to avoid (running the server from this
/// repository's checkout would bind every request to the repository's own
/// `scripts/` directory). That fallback, and the refusal of any *other*
/// relative `workspace_path`, now live in
/// `factorio_bot_core::paths::resolve_workspace`, which this function calls
/// rather than restates: `POST /api/v1/instance/start` needs the same rule,
/// and a second copy of it is how the two routes would come to disagree about
/// where the workspace is (they did -- the start route used to reject the
/// empty default outright).
///
/// Split out from `scripts_root` (which canonicalizes and requires the
/// result to exist) so the fallback and the absolute-path guard can be
/// unit-tested in isolation: `paths::workspace_dir()` is a real,
/// developer-specific directory that a test must not create, write into,
/// or otherwise depend on the contents of, and canonicalize would fail
/// outright if it (or its `scripts` subdirectory) happens not to exist on
/// the machine running the tests.
fn scripts_root_path(workspace_path: &str) -> Result<PathBuf, ErrorResponse> {
    let workspace_path = factorio_bot_core::paths::resolve_workspace(workspace_path)
        .map_err(|err| ErrorResponse::bad_request(err.to_string()))?;
    Ok(workspace_path.as_path().join("scripts"))
}

/// Crate-visible because `manage::execute` resolves the same root: a script
/// run over HTTP and a script read over HTTP must mean the same file, and a
/// second copy of this lookup is how they would come to disagree.
pub(crate) async fn scripts_root(state: &AppState) -> Result<PathBuf, ErrorResponse> {
    let workspace_path = state.settings.read().await.factorio.workspace_path.clone();
    let root = scripts_root_path(workspace_path.as_ref())?;
    std::fs::canonicalize(&root).map_err(|_| {
        ErrorResponse::bad_request(format!("missing scripts directory: {}", root.display()))
    })
}

/// Resolves the destination for a script that is about to be *created*.
///
/// `resolve_script_path` canonicalises, and `canonicalize` fails when the
/// target does not exist — so it cannot be used on the path of a file that
/// is not there yet. Instead this splits the requested path into a parent
/// directory and a final path component: the parent is resolved through
/// `resolve_script_path` (which is what keeps the traversal guard in
/// force, exactly as for read/write/delete), and the final component is
/// validated on its own before being joined back on.
///
/// The final component must be exactly one *ordinary* path component, and the
/// joined result is bounds-checked against `root` before it is returned.
///
/// The component check goes through `Path::components` rather than a
/// hand-written list of forbidden strings, because a list is not complete.
/// `Components` normalises `.` away to `CurDir`, yields `ParentDir` for `..`,
/// `RootDir`/`Prefix` for an absolute or drive-qualified path, and more than
/// one item for anything holding a separator — so "exactly one
/// `Component::Normal`" subsumes every case the old five-way filter
/// (`/`, `\`, `.`, `..`, empty) covered *and* rejects a drive prefix, which
/// that filter did not.
///
/// The prefix case is not hypothetical, and this check is **not inert on
/// Windows**: `PathBuf::push` (which `Path::join` uses) documents that pushing
/// a path with a prefix but no root — `C:evil.lua` — *replaces* the existing
/// path entirely rather than appending to it. Without this check, a request
/// for `C:evil.lua` would therefore discard the already-verified `parent` and
/// return a path outside the scripts root altogether. Unix is unaffected
/// (there are no prefixes), which is exactly why a Unix-only reading of the
/// old filter concluded it was dead code.
///
/// Belt and braces, the joined `target` is re-checked with
/// `starts_with(root)`. This is the one script path that cannot be
/// canonicalised before use — the file does not exist yet, and `canonicalize`
/// fails on a missing path — so it is the one place where a bounds check has
/// to be made on an unresolved path, and it is worth making twice.
fn resolve_new_script_path(
    root: &std::path::Path,
    requested: &str,
) -> Result<PathBuf, ErrorResponse> {
    let trimmed = requested.trim_end_matches('/');
    let (parent_part, component) = match trimmed.rsplit_once('/') {
        Some((parent, component)) => (parent, component),
        None => ("", trimmed),
    };

    let mut components = std::path::Path::new(component).components();
    let is_single_normal_component = matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    );
    if !is_single_normal_component {
        return Err(ErrorResponse::bad_request(format!(
            "invalid script name: {requested}"
        )));
    }

    let parent = resolve_script_path(root, parent_part).map_err(ErrorResponse::from)?;
    if !parent.is_dir() {
        return Err(ErrorResponse::bad_request(format!(
            "not a directory: {parent_part}"
        )));
    }

    let target = parent.join(component);
    if !target.starts_with(root) {
        return Err(ErrorResponse::bad_request(format!(
            "path escapes the scripts directory: {requested}"
        )));
    }
    Ok(target)
}

/// Lists a directory under the scripts root as a script tree
#[utoipa::path(
    get,
    path = "/api/v1/scripts",
    tag = "Admin",
    params(ScriptPathQuery),
    responses(
        (status = 200, body = Vec<ScriptTreeNode>),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
        (status = 500, body = crate::error::ErrorResponse),
    )
)]
pub async fn list_scripts(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ScriptPathQuery>,
) -> ApiResult<Vec<ScriptTreeNode>> {
    let root = scripts_root(&state).await?;
    let resolved = resolve_script_path(&root, &query.path).map_err(ErrorResponse::from)?;
    if !resolved.is_dir() {
        return Err(ErrorResponse::bad_request(format!(
            "not a directory: {}",
            query.path
        )));
    }

    // Build the key prefix from the *request* path (not the resolved
    // filesystem path) so the tree's keys are stable, slash-separated
    // strings a client can feed straight back into a subsequent request.
    let trimmed = query.path.trim_matches('/');
    let key_prefix = if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    };

    let read_dir = std::fs::read_dir(&resolved)
        .map_err(|err| ErrorResponse::internal(format!("failed to list directory: {err}")))?;

    let mut entries = Vec::new();
    for entry in read_dir {
        let entry =
            entry.map_err(|err| ErrorResponse::internal(format!("failed to read entry: {err}")))?;
        let file_type = entry
            .file_type()
            .map_err(|err| ErrorResponse::internal(format!("failed to read entry type: {err}")))?;
        // A filename that is not valid UTF-8 must not abort the process:
        // fall back to a lossy conversion rather than `.to_str().unwrap()`.
        let file_name = entry.file_name().to_string_lossy().into_owned();
        entries.push((file_name, file_type.is_dir()));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let nodes = entries
        .into_iter()
        .map(|(file_name, is_dir)| ScriptTreeNode {
            key: format!("{key_prefix}/{file_name}"),
            label: file_name,
            leaf: !is_dir,
            children: Vec::new(),
        })
        .collect();

    Ok(Json(nodes))
}

/// Reads the contents of a script file
#[utoipa::path(
    get,
    path = "/api/v1/scripts/file",
    tag = "Admin",
    params(ScriptPathQuery),
    responses(
        (status = 200, body = ScriptContent),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
        (status = 500, body = crate::error::ErrorResponse),
    )
)]
pub async fn read_script(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ScriptPathQuery>,
) -> ApiResult<ScriptContent> {
    let root = scripts_root(&state).await?;
    let resolved = resolve_script_path(&root, &query.path).map_err(ErrorResponse::from)?;
    if !resolved.is_file() {
        return Err(ErrorResponse::bad_request(format!(
            "not a file: {}",
            query.path
        )));
    }
    let code = std::fs::read_to_string(&resolved)
        .map_err(|err| ErrorResponse::internal(format!("failed to read script: {err}")))?;
    Ok(Json(ScriptContent { code }))
}

/// Overwrites the contents of an existing script file
///
/// Mirrors the original desktop command's behavior: this only writes to a
/// file that already exists under the scripts root. Creating new scripts is
/// not supported here; that would be a separate decision.
#[utoipa::path(
    put,
    path = "/api/v1/scripts/file",
    tag = "Admin",
    params(ScriptPathQuery),
    request_body = ScriptContent,
    responses(
        (status = 204),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
        (status = 500, body = crate::error::ErrorResponse),
    )
)]
pub async fn write_script(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ScriptPathQuery>,
    ApiJson(body): ApiJson<ScriptContent>,
) -> Result<StatusCode, ErrorResponse> {
    let root = scripts_root(&state).await?;
    let resolved = resolve_script_path(&root, &query.path).map_err(ErrorResponse::from)?;
    if !resolved.is_file() {
        return Err(ErrorResponse::bad_request(format!(
            "not a file: {}",
            query.path
        )));
    }
    std::fs::write(&resolved, body.code)
        .map_err(|err| ErrorResponse::internal(format!("failed to write script: {err}")))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Creates a new script file. Rejects the request if the target already
/// exists — use `PUT` to overwrite an existing script instead.
///
/// Deliberately does not check `target.exists()` and then call
/// `std::fs::write`: `exists()` follows symlinks and reports `false` for a
/// *dangling* one, and `std::fs::write` (via `File::create`) also follows
/// symlinks when opening. A dangling symlink planted inside the scripts
/// root pointing outside it (e.g. `sub/evil.lua -> /etc/evil.lua`, where
/// `/etc/evil.lua` does not exist yet) would therefore pass the `exists()`
/// check and then have its *target* created and written by `fs::write` —
/// a write outside the scripts root that neither `resolve_new_script_path`
/// nor an `exists()` check catches, since neither one ever has reason to
/// look at `evil.lua` once the parent has resolved. `OpenOptions::create_new`
/// avoids this: POSIX `open` with `O_CREAT | O_EXCL` fails with `EEXIST`
/// when the final path component is a symlink, dangling or not, so this
/// never follows one to create a file. It also removes the
/// check-then-write TOCTOU race between the `exists()` check and the write.
#[utoipa::path(
    post,
    path = "/api/v1/scripts/file",
    tag = "Admin",
    params(ScriptPathQuery),
    request_body = ScriptContent,
    responses(
        (status = 201),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
        (status = 409, body = crate::error::ErrorResponse),
        (status = 500, body = crate::error::ErrorResponse),
    )
)]
pub async fn create_script(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ScriptPathQuery>,
    ApiJson(body): ApiJson<ScriptContent>,
) -> Result<StatusCode, ErrorResponse> {
    let root = scripts_root(&state).await?;
    let target = resolve_new_script_path(&root, &query.path)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|err| match err.kind() {
            // `create_new` reports `EEXIST` both for a real file and for a
            // symlink standing where the new script would go. Either way the
            // caller's answer is the same, and raw OS text ("File exists (os
            // error 17)") is not an answer a browser can show a user.
            std::io::ErrorKind::AlreadyExists => {
                ErrorResponse::conflict(format!("script already exists: {}", query.path))
            }
            // The parent resolved a moment ago; if it is gone now the client
            // is asking for something that is not there.
            std::io::ErrorKind::NotFound => {
                ErrorResponse::not_found(format!("path not found: {}", query.path))
            }
            _ => ErrorResponse::internal(format!("failed to create script: {err}")),
        })?;
    file.write_all(body.code.as_bytes())
        .map_err(|err| ErrorResponse::internal(format!("failed to write script: {err}")))?;
    Ok(StatusCode::CREATED)
}

/// Deletes an existing script file. Deleting a directory is not supported.
#[utoipa::path(
    delete,
    path = "/api/v1/scripts/file",
    tag = "Admin",
    params(ScriptPathQuery),
    responses(
        (status = 204),
        (status = 400, body = crate::error::ErrorResponse),
        (status = 404, body = crate::error::ErrorResponse),
        (status = 500, body = crate::error::ErrorResponse),
    )
)]
pub async fn delete_script(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ScriptPathQuery>,
) -> Result<StatusCode, ErrorResponse> {
    let root = scripts_root(&state).await?;
    let resolved = resolve_script_path(&root, &query.path).map_err(ErrorResponse::from)?;
    if !resolved.is_file() {
        return Err(ErrorResponse::bad_request(format!(
            "not a file: {}",
            query.path
        )));
    }
    std::fs::remove_file(&resolved)
        .map_err(|err| ErrorResponse::internal(format!("failed to delete script: {err}")))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defect this guards against: on a default configuration
    /// (`FactorioSettings::default()` leaves `workspace_path` empty),
    /// `scripts_root_path` must not resolve to the bare relative path
    /// `"scripts"` — canonicalizing that would bind against the server
    /// process's current working directory, which in this repository's own
    /// checkout is a real `scripts/` directory that `GET`/`PUT
    /// /api/v1/scripts*` would then read from and write over.
    ///
    /// This asserts the weaker-but-safe property instead of exercising a
    /// real filesystem fallback: with `workspace_path` empty, the resolved
    /// root equals `paths::workspace_dir().join("scripts")` and is
    /// absolute. A stronger test that actually points the server at the
    /// test process's CWD and proves it does *not* read a `scripts/`
    /// directory placed there would require writing into (or reading
    /// preexisting contents of) either the real data-local workspace
    /// directory or the test binary's CWD — both outside any temp
    /// directory this test owns, which the constraint against tests
    /// writing outside their own temp directory rules out.
    #[test]
    fn empty_workspace_path_falls_back_to_the_data_local_workspace_dir_not_the_cwd() {
        let root = scripts_root_path("").expect("resolves");
        assert_eq!(
            root,
            factorio_bot_core::paths::workspace_dir().join("scripts")
        );
        assert!(
            root.is_absolute(),
            "expected an absolute path, got {root:?}"
        );
    }

    #[test]
    fn configured_workspace_path_is_joined_as_given() {
        let root = scripts_root_path("/configured/workspace").expect("resolves");
        assert_eq!(root, PathBuf::from("/configured/workspace/scripts"));
    }

    /// The bounds guarantee, stated as a property rather than a list of
    /// forbidden strings: whatever `resolve_new_script_path` returns is inside
    /// `root`, for every hostile shape of final component.
    ///
    /// `C:evil.lua` is the interesting one and the reason the old five-way
    /// filter (`/`, `\`, `.`, `..`, empty) was not enough. On Windows it is a
    /// prefixed-but-rootless path, and `PathBuf::push` *replaces* the whole
    /// path with it — discarding the already-verified parent and landing
    /// outside the root entirely. On Unix there are no prefixes, so it is an
    /// ordinary (if odd) filename that stays inside the root; asserting
    /// containment rather than rejection is what lets one test carry the
    /// property on both platforms.
    #[test]
    fn a_resolved_new_script_path_always_stays_inside_the_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("scripts");
        std::fs::create_dir_all(root.join("sub")).expect("mkdir");
        let root = std::fs::canonicalize(&root).expect("canonicalize");

        for requested in [
            "",
            "/",
            ".",
            "..",
            "/.",
            "/..",
            "/sub/.",
            "/sub/..",
            "C:evil.lua",
            "/C:evil.lua",
            "/sub/C:evil.lua",
            "C:/evil.lua",
            "/etc/evil.lua",
            "\\evil.lua",
            "/sub/../../evil.lua",
            "/ok.lua",
            "/sub/ok.lua",
        ] {
            // Rejecting outright is always an acceptable answer here; what
            // must never happen is *accepting* and landing outside the root.
            if let Ok(target) = resolve_new_script_path(&root, requested) {
                assert!(
                    target.starts_with(&root),
                    "{requested:?} resolved to {target:?}, outside {root:?}"
                );
            }
        }
    }

    /// The cases that must be *rejected* on every platform, as opposed to
    /// merely staying inside the root.
    #[test]
    fn a_new_script_name_must_be_a_single_ordinary_component() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("scripts");
        std::fs::create_dir_all(root.join("sub")).expect("mkdir");
        let root = std::fs::canonicalize(&root).expect("canonicalize");

        for requested in ["", "/", ".", "..", "/.", "/..", "/sub/.", "/sub/.."] {
            assert!(
                resolve_new_script_path(&root, requested).is_err(),
                "{requested:?} should not resolve to a creatable path"
            );
        }

        let ok = resolve_new_script_path(&root, "/sub/ok.lua").expect("resolves");
        assert_eq!(ok, root.join("sub").join("ok.lua"));
    }

    /// Belt and braces: a *non-empty* but still relative `workspace_path`
    /// (e.g. a future settings value like `"./foo"`) must be rejected
    /// outright rather than silently canonicalized against the server's
    /// CWD.
    #[test]
    fn relative_workspace_path_is_rejected() {
        let result = scripts_root_path("./configured/workspace");
        assert!(result.is_err(), "expected a relative path to be rejected");
    }
}
