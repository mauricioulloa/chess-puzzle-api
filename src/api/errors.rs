use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest {
        message: String,
        hint: Option<String>,
    },
    NoMatch(String),
    Internal(anyhow::Error),
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error, message, hint) = match self {
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", message, None),
            ApiError::BadRequest { message, hint } => {
                (StatusCode::BAD_REQUEST, "bad_request", message, hint)
            }
            // The request was well formed, it just describes an empty slice of
            // the dataset. 404 says so more usefully than an empty 200 body.
            ApiError::NoMatch(message) => (
                StatusCode::NOT_FOUND,
                "no_match",
                message,
                Some("Loosen the rating range or drop a theme.".to_string()),
            ),
            ApiError::Internal(err) => {
                tracing::error!("unhandled error: {err:#}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "Something went wrong on our side.".to_string(),
                    None,
                )
            }
        };

        (
            status,
            Json(ErrorBody {
                error,
                message,
                hint,
            }),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::Internal(err)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
