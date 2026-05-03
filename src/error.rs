use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use utoipa::ToSchema;

/// Stable, machine-readable error codes. Clients are allowed to switch on
/// these. Adding a variant is additive; renaming or removing one is a
/// breaking change in production phase (see `.claude/rules/api-contract.md`).
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    DatabaseError,
    UnsupportedFormat,
    NotFound,
    Unauthorized,
    Forbidden,
    AuthNotConfigured,
    Conflict,
    PreconditionRequired,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorDetail {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("format {0:?} is not supported by the validator")]
    UnsupportedFormat(crate::domain::format::FormatId),
    #[error("{resource} '{id}' not found")]
    NotFound { resource: &'static str, id: String },
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("auth is not configured on this server")]
    AuthNotConfigured,
    /// The `If-Match` precondition didn't match the current resource state —
    /// a concurrent edit happened between read and write. Maps to 412.
    #[error("resource was modified by another request")]
    Conflict,
    /// The handler requires an `If-Match` header and the client didn't send
    /// one. Maps to 428 (RFC 6585).
    #[error("if-match header is required")]
    PreconditionRequired,
}

impl AppError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Database(_) => ErrorCode::DatabaseError,
            Self::UnsupportedFormat(_) => ErrorCode::UnsupportedFormat,
            Self::NotFound { .. } => ErrorCode::NotFound,
            Self::Unauthorized => ErrorCode::Unauthorized,
            Self::Forbidden => ErrorCode::Forbidden,
            Self::AuthNotConfigured => ErrorCode::AuthNotConfigured,
            Self::Conflict => ErrorCode::Conflict,
            Self::PreconditionRequired => ErrorCode::PreconditionRequired,
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::UnsupportedFormat(_) => StatusCode::BAD_REQUEST,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::AuthNotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            Self::Conflict => StatusCode::PRECONDITION_FAILED,
            Self::PreconditionRequired => StatusCode::PRECONDITION_REQUIRED,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let code = self.code();
        let message = self.to_string();

        if status.is_server_error() {
            tracing::error!(error = ?self, "request failed");
        }

        let body = ErrorBody {
            error: ErrorDetail {
                code,
                message,
                details: None,
            },
        };
        (status, Json(body)).into_response()
    }
}
