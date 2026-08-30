use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use factorio_bot_core::scripts::ScriptPathError;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Default HTTP status of an [`ErrorResponse`].
///
/// Every construction site that does not say otherwise keeps the behavior this
/// type had before it carried a status at all.
fn default_status() -> StatusCode {
    StatusCode::BAD_REQUEST
}

/// Error body returned by every failing endpoint.
///
/// `status` is the HTTP status [`IntoResponse`] answers with. It is not part of
/// the JSON body — it would be redundant with the response line, and clients
/// generated from the OpenAPI schema must not see a field the server never
/// serializes — so it is `#[serde(skip)]`ped and defaults to 400 on both
/// construction and deserialization.
#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct ErrorResponse {
    pub message: String,
    pub code: u32,
    /// The job holding the single execution slot, on the `409` from
    /// `POST /api/v1/scripts/execute`. Skipped when absent so every other
    /// error body is byte-for-byte what it was before this field existed --
    /// a client that switched on the presence of a key would otherwise start
    /// seeing `"running_job_id": null` on unrelated failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running_job_id: Option<String>,
    #[serde(skip, default = "default_status")]
    #[schema(ignore)]
    pub status: StatusCode,
}

impl ErrorResponse {
    pub fn new(message: String, code: u32) -> Self {
        ErrorResponse {
            message,
            code,
            running_job_id: None,
            status: default_status(),
        }
    }

    /// Answers with `status` instead of the default 400.
    ///
    /// Chained onto a constructor at the handful of sites that know better:
    /// a missing script is the client asking for something that is not there
    /// (404), creating over an existing one is a conflict (409), and a failed
    /// `AppSettings::save` or `FactorioInstance::stop` is the server's own
    /// fault (500), not the caller's.
    #[must_use]
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// No Factorio instance is running.
    ///
    /// Answers `400`, which is what `POST /api/v1/rcon` and
    /// `POST /api/v1/instance/stop` have always done. See [`not_running`] for
    /// the `503` reading of the same condition.
    ///
    /// [`not_running`]: ErrorResponse::not_running
    pub fn not_started() -> Self {
        ErrorResponse::new("not started".into(), 2)
    }

    /// No Factorio instance is running, for an endpoint that treats that as a
    /// temporary condition of the server rather than a malformed request.
    ///
    /// Deliberately a second constructor rather than a change to
    /// [`not_started`]: "start a script" is not something the caller can fix by
    /// sending a better request, so `503` is the honest answer -- but the two
    /// older endpoints have answered `400` since before this crate carried a
    /// status at all, and their tests pin it.
    ///
    /// [`not_started`]: ErrorResponse::not_started
    pub fn not_running(message: impl Into<String>) -> Self {
        ErrorResponse::new(message.into(), 2).with_status(StatusCode::SERVICE_UNAVAILABLE)
    }

    /// The single script-execution slot is taken, and by whom.
    ///
    /// The id is carried in its own field rather than only in the message so a
    /// client can follow it (to `GET /api/v1/jobs/{id}`) without parsing prose.
    pub fn already_running(running: crate::jobs::JobId) -> Self {
        let mut response =
            ErrorResponse::conflict(format!("a script is already running as job {running}"));
        response.running_job_id = Some(running.to_string());
        response
    }

    /// A query parameter could not be parsed.
    pub fn bad_request(message: impl Into<String>) -> Self {
        ErrorResponse::new(message.into(), 1)
    }

    /// The requested resource does not exist.
    pub fn not_found(message: impl Into<String>) -> Self {
        ErrorResponse::new(message.into(), 4).with_status(StatusCode::NOT_FOUND)
    }

    /// The request cannot be applied to the current state of the resource
    /// (e.g. creating a script that already exists).
    pub fn conflict(message: impl Into<String>) -> Self {
        ErrorResponse::new(message.into(), 5).with_status(StatusCode::CONFLICT)
    }

    /// The server failed at something that is not the caller's fault.
    pub fn internal(message: impl Into<String>) -> Self {
        ErrorResponse::new(message.into(), 6).with_status(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        let status = self.status;
        (status, Json(self)).into_response()
    }
}

impl From<miette::Report> for ErrorResponse {
    fn from(report: miette::Report) -> Self {
        ErrorResponse::new(format!("{report}"), 3)
    }
}

/// A script path that does not resolve is a 404 — the client asked for
/// something that is not there. A path that tries to leave the scripts root is
/// the client's own malformed request, so it stays a 400 (and deliberately
/// does not say what is on the other side).
impl From<ScriptPathError> for ErrorResponse {
    fn from(err: ScriptPathError) -> Self {
        match err {
            ScriptPathError::NotFound { .. } => ErrorResponse::not_found(err.to_string()),
            ScriptPathError::EscapesRoot { .. } => ErrorResponse::bad_request(err.to_string()),
        }
    }
}

/// Maps the script runner's typed failure onto statuses.
///
/// The `Path` arm delegates to the [`ScriptPathError`] conversion above rather
/// than restating it, so the 404/400 split has exactly one definition. This is
/// the whole reason `run_script_file` reports a typed error instead of a
/// formatted string: recovering the split by matching on message text would
/// turn rewording an error into a silent status-code regression that no test
/// could catch.
#[cfg(feature = "lua")]
impl From<factorio_bot_scripting_lua::RunScriptError> for ErrorResponse {
    fn from(err: factorio_bot_scripting_lua::RunScriptError) -> Self {
        use factorio_bot_scripting_lua::RunScriptError;
        match err {
            RunScriptError::Path(err) => ErrorResponse::from(err),
            RunScriptError::Run(report) => ErrorResponse::from(report),
            // A name that is not a file, or whose extension names no
            // interpreter, is a request this server will never accept -- the
            // caller's mistake, not a missing resource.
            other => ErrorResponse::bad_request(other.to_string()),
        }
    }
}

/// axum's own `Query` rejection answers in `text/plain`; convert it so a
/// malformed query string still gets the same JSON shape as every other
/// handler error.
impl From<axum::extract::rejection::QueryRejection> for ErrorResponse {
    fn from(rejection: axum::extract::rejection::QueryRejection) -> Self {
        ErrorResponse::bad_request(rejection.body_text())
    }
}

/// Same as above, but for the `Json` body extractor.
impl From<axum::extract::rejection::JsonRejection> for ErrorResponse {
    fn from(rejection: axum::extract::rejection::JsonRejection) -> Self {
        ErrorResponse::bad_request(rejection.body_text())
    }
}

pub type ApiResult<T> = Result<Json<T>, ErrorResponse>;
