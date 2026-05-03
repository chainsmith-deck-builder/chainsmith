use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use chainsmith::{
    api,
    config::Config,
    db,
    domain::format::classic_constructed::ClassicConstructed,
    state::AppState,
    sync::fab_cube::{self, SyncCache, UpstreamSource},
};
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

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent("chainsmith-sync")
        .build()
        .context("building HTTP client")?;

    let cache = SyncCache {
        directory: config.sync_cache_dir.clone(),
        ttl: config.sync_cache_ttl,
    };
    let upstream = UpstreamSource::at(config.sync_upstream_ref.clone());

    info!(git_ref = %upstream.git_ref, "loading card data");
    let sync_start = Instant::now();
    let sync_output = fab_cube::load_or_fetch(&http_client, &upstream, Some(&cache))
        .await
        .context("loading card data")?;
    info!(
        cards = sync_output.card_count,
        printings = sync_output.printing_count,
        cc_banned = sync_output.cc_banned.len(),
        cc_ll_retired = sync_output.cc_living_legend.len(),
        elapsed_ms = sync_start.elapsed().as_millis() as u64,
        "card data ready",
    );

    let cc_format = Arc::new(ClassicConstructed::new(
        sync_output.cc_banned,
        sync_output.cc_living_legend,
    ));
    let catalog = Arc::new(sync_output.catalog);

    let state = AppState {
        pool,
        catalog,
        cc_format,
    };
    let app = api::router(state).layer(TraceLayer::new_for_http());

    let addr: SocketAddr = config.bind_addr.parse().context("parsing BIND_ADDR")?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding listener at {addr}"))?;

    info!(%addr, "chainsmith listening");
    axum::serve(listener, app).await.context("serving HTTP")?;
    Ok(())
}
