use actix_web::http::StatusCode;
use actix_web::{HttpResponse, ResponseError};
use serde_json::json;

/// Errors returned by the OpenAI-compatible API, in OpenAI's error shape.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    /// Every upstream failed; carries the last upstream status and message.
    #[error("{message}")]
    Upstream { status: StatusCode, message: String },
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Upstream { status, .. } => *status,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        let kind = match self {
            Self::Unauthorized(_) => "authentication_error",
            Self::BadRequest(_) => "invalid_request_error",
            Self::NotFound(_) => "not_found_error",
            Self::Upstream { .. } => "upstream_error",
            Self::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                "server_error"
            }
        };
        let message = match self {
            Self::Internal(_) => "internal server error".to_string(),
            other => other.to_string(),
        };
        HttpResponse::build(self.status_code())
            .json(json!({ "error": { "message": message, "type": kind } }))
    }
}

/// Errors in the admin UI. Rendered as a plain page; details go to the log.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for WebError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => Self::NotFound,
            e => Self::Internal(e.into()),
        }
    }
}

impl From<askama::Error> for WebError {
    fn from(e: askama::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl ResponseError for WebError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        if let Self::Internal(e) = self {
            tracing::error!(error = %e, "admin UI error");
        }
        HttpResponse::build(self.status_code())
            .content_type("text/plain; charset=utf-8")
            .body(match self {
                Self::NotFound => "Not found",
                Self::Internal(_) => "Something went wrong. Check the gateway log.",
            })
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
pub type WebResult<T> = Result<T, WebError>;
