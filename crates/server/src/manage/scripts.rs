use crate::error::{ApiResult, ErrorResponse};
use crate::extract::{ApiJson, ApiQuery};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use factorio_bot_core::scripts::resolve_script_path;
use factorio_bot_core::types::PrimeVueTreeNode;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
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
async fn scripts_root(state: &AppState) -> Result<PathBuf, ErrorResponse> {
    let workspace_path = state.settings.read().await.factorio.workspace_path.clone();
    let root = Path::new(workspace_path.as_ref()).join("scripts");
    std::fs::canonicalize(&root).map_err(|_| {
        ErrorResponse::bad_request(format!("missing scripts directory: {}", root.display()))
    })
}

/// Lists a directory under the scripts root as a `PrimeVue` tree
#[utoipa::path(
    get,
    path = "/api/v1/scripts",
    tag = "Admin",
    params(ScriptPathQuery),
    responses(
        (status = 200, body = Vec<PrimeVueTreeNode>),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn list_scripts(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ScriptPathQuery>,
) -> ApiResult<Vec<PrimeVueTreeNode>> {
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
        .map_err(|err| ErrorResponse::bad_request(format!("failed to list directory: {err}")))?;

    let mut entries = Vec::new();
    for entry in read_dir {
        let entry = entry
            .map_err(|err| ErrorResponse::bad_request(format!("failed to read entry: {err}")))?;
        let file_type = entry.file_type().map_err(|err| {
            ErrorResponse::bad_request(format!("failed to read entry type: {err}"))
        })?;
        // A filename that is not valid UTF-8 must not abort the process:
        // fall back to a lossy conversion rather than `.to_str().unwrap()`.
        let file_name = entry.file_name().to_string_lossy().into_owned();
        entries.push((file_name, file_type.is_dir()));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let nodes = entries
        .into_iter()
        .map(|(file_name, is_dir)| PrimeVueTreeNode {
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
        .map_err(|err| ErrorResponse::bad_request(format!("failed to read script: {err}")))?;
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
        .map_err(|err| ErrorResponse::bad_request(format!("failed to write script: {err}")))?;
    Ok(StatusCode::NO_CONTENT)
}
