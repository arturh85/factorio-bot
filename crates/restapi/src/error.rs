use rocket::response::status::BadRequest;
use rocket::serde::json::Json;

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
pub struct ErrorResponse {
    message: String,
    code: u32,
}

impl ErrorResponse {
    pub fn new(message: String, code: u32) -> BadRequest<Json<ErrorResponse>> {
        BadRequest(Json(ErrorResponse { message, code }))
    }
}

pub type RestApiResult<T> = Result<Json<T>, BadRequest<Json<ErrorResponse>>>;
