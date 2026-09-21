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
    Unauthorized(String),
    RateLimited {
        retry_after: u64,
        limit: u32,
    },
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
            ApiError::Unauthorized(message) => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                message,
                Some("Keys are issued by opening an issue on the repository.".to_string()),
            ),
            ApiError::RateLimited { retry_after, limit } => {
                // Retry-After is the one header a client can act on without
                // reading the docs, so it is set even though the rate limit
                // headers carry the same information.
                let mut response = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(ErrorBody {
                        error: "rate_limited",
                        message: format!("Rate limit of {limit} requests per minute exceeded."),
                        hint: Some(format!(
                            "Retry in {retry_after}s, or use an API key for a higher limit."
                        )),
                    }),
                )
                    .into_response();
                if let Ok(value) = retry_after.to_string().parse() {
                    response.headers_mut().insert("retry-after", value);
                }
                return response;
            }
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
