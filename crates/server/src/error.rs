use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Error body returned by every failing endpoint.
#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct ErrorResponse {
    pub message: String,
    pub code: u32,
}

impl ErrorResponse {
    pub fn new(message: String, code: u32) -> Self {
        ErrorResponse { message, code }
    }

    /// No Factorio instance is running.
    pub fn not_started() -> Self {
        ErrorResponse::new("not started".into(), 2)
    }

    /// A query parameter could not be parsed.
    pub fn bad_request(message: impl Into<String>) -> Self {
        ErrorResponse::new(message.into(), 1)
    }
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

impl From<miette::Report> for ErrorResponse {
    fn from(report: miette::Report) -> Self {
        ErrorResponse::new(format!("{report}"), 3)
    }
}

pub type ApiResult<T> = Result<Json<T>, ErrorResponse>;
