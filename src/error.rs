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
}

impl AppError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Database(_) => ErrorCode::DatabaseError,
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
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
