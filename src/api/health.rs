use axum::{extract::State, Json};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    error::{AppError, ErrorBody},
    state::AppState,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(health))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: &'static str,
    pub db: &'static str,
}

#[utoipa::path(
    get,
    path = "/health",
    operation_id = "getHealth",
    tags = ["Health"],
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse),
        (status = 500, description = "Service is unhealthy", body = ErrorBody)
    )
)]
async fn health(State(state): State<AppState>) -> Result<Json<HealthResponse>, AppError> {
    // SELECT 1 has no parameters and no schema worth checking, so the
    // sqlx::query! macro adds no value here. Per .claude/rules/database.md,
    // the runtime form is the documented exception for trivial connection checks.
    sqlx::query("SELECT 1").execute(&state.pool).await?;

    Ok(Json(HealthResponse {
        status: "ok",
        db: "ok",
    }))
}
