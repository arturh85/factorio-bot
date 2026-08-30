use crate::error::ApiResult;
use crate::extract::ApiQuery;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
pub struct ExistsParams {
    pub path: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExistsResponse {
    pub exists: bool,
}

/// Reports whether a path exists on the server
#[utoipa::path(
    get,
    path = "/api/v1/fs/exists",
    tag = "Admin",
    params(ExistsParams),
    responses(
        (status = 200, body = ExistsResponse),
        (status = 400, body = crate::error::ErrorResponse),
    )
)]
pub async fn exists(ApiQuery(params): ApiQuery<ExistsParams>) -> ApiResult<ExistsResponse> {
    Ok(Json(ExistsResponse {
        exists: std::path::Path::new(&params.path).exists(),
    }))
}
