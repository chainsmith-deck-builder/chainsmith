use std::sync::Arc;

use sqlx::PgPool;

use crate::domain::catalog::Catalog;
use crate::domain::format::classic_constructed::ClassicConstructed;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Card catalog populated at startup from the upstream sync. Shared
    /// across all request handlers via `Arc`.
    pub catalog: Arc<Catalog>,
    /// Classic Constructed format instance pre-built with the current
    /// banned and Living Legend lists from the sync. Reused by validation
    /// requests.
    pub cc_format: Arc<ClassicConstructed>,
}
