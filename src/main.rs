use std::net::SocketAddr;

use anyhow::Context;
use chainsmith::{api, config::Config, db, state::AppState};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,chainsmith=debug")),
        )
        .init();

    let config = Config::from_env().context("loading config")?;

    let pool = db::init_pool(&config)
        .await
        .context("initializing database pool")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running migrations")?;

    let state = AppState { pool };
    let app = api::router(state).layer(TraceLayer::new_for_http());

    let addr: SocketAddr = config.bind_addr.parse().context("parsing BIND_ADDR")?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding listener at {addr}"))?;

    info!(%addr, "chainsmith listening");
    axum::serve(listener, app).await.context("serving HTTP")?;
    Ok(())
}
