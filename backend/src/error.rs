use std::fmt::Display;

use actix_web::http::StatusCode;
use actix_web::HttpResponse;
use serde::__private::Formatter;

#[derive(Debug)]
pub struct ActixAnyhowError {
    err: anyhow::Error,
}
impl Display for ActixAnyhowError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.write_str("ERROR")?;
        Ok(())
    }
}

impl actix_web::error::ResponseError for ActixAnyhowError {
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    fn error_response(&self) -> actix_web::HttpResponse {
        HttpResponse::InternalServerError().body(self.err.to_string())
    }
}
impl From<anyhow::Error> for ActixAnyhowError {
    fn from(err: anyhow::Error) -> ActixAnyhowError {
        ActixAnyhowError { err }
    }
}
