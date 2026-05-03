//! `AuthenticatedUser` extractor — drop-in for any handler that requires a
//! valid Supabase JWT in the `Authorization: Bearer ...` header.
//!
//! Failed verification returns 401 with a deliberately generic body so we
//! don't leak which specific check failed (per `.claude/rules/security.md`).
//! When auth is not configured at all (no env vars set), the extractor
//! returns 503 — a config error, not a client error.

use axum::extract::FromRequestParts;
use axum::http::{header::AUTHORIZATION, request::Parts};
use uuid::Uuid;

use super::{parse_subject, AuthError};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub id: Uuid,
    pub email: Option<String>,
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).map_err(map_auth_error)?;
        let claims = state.auth.verify(token).await.map_err(map_auth_error)?;
        let id = parse_subject(&claims.sub).map_err(map_auth_error)?;
        Ok(AuthenticatedUser {
            id,
            email: claims.email,
        })
    }
}

fn bearer_token(parts: &Parts) -> Result<&str, AuthError> {
    let header = parts
        .headers
        .get(AUTHORIZATION)
        .ok_or(AuthError::MissingHeader)?;
    let value = header.to_str().map_err(|_| AuthError::MalformedHeader)?;
    value
        .strip_prefix("Bearer ")
        .ok_or(AuthError::MalformedHeader)
}

fn map_auth_error(err: AuthError) -> AppError {
    match err {
        AuthError::NotConfigured => AppError::AuthNotConfigured,
        _ => AppError::Unauthorized,
    }
}
