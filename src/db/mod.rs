use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::config::Config;

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn init_pool(config: &Config) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect(&config.database_url)
        .await
}
