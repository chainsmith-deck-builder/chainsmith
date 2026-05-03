use axum::Router;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

pub mod health;
pub mod validate;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Chainsmith API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Headless Rust API for the Chainsmith Flesh and Blood deck builder."
    )
)]
struct ApiDoc;

pub fn router(state: AppState) -> Router {
    build().0.with_state(state)
}

pub fn openapi() -> utoipa::openapi::OpenApi {
    build().1
}

fn build() -> (Router<AppState>, utoipa::openapi::OpenApi) {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(health::router())
        .merge(validate::router())
        .split_for_parts()
}
