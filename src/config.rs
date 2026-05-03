use std::env;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";
const DEFAULT_DB_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_SYNC_CACHE_DIR: &str = ".chainsmith-cache/fab-cube";
const DEFAULT_SYNC_UPSTREAM_REF: &str = "main";
const DEFAULT_SYNC_CACHE_TTL_SECS: u64 = 86_400; // 24h

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub db_max_connections: u32,
    /// Local directory the sync writes/reads JSON files under. Cache is
    /// further partitioned by upstream git ref.
    pub sync_cache_dir: PathBuf,
    /// Upstream git ref to fetch from. Use a tag or commit SHA in
    /// production; `main` is fine for dev.
    pub sync_upstream_ref: String,
    /// Cache freshness TTL. After this elapses the next sync re-fetches.
    pub sync_cache_ttl: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let database_url =
            env::var("DATABASE_URL").map_err(|_| ConfigError::Missing("DATABASE_URL"))?;

        let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());

        let db_max_connections = match env::var("DB_MAX_CONNECTIONS") {
            Ok(raw) => raw.parse().map_err(|source| ConfigError::Parse {
                var: "DB_MAX_CONNECTIONS",
                source,
            })?,
            Err(_) => DEFAULT_DB_MAX_CONNECTIONS,
        };

        let sync_cache_dir = env::var("SYNC_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_SYNC_CACHE_DIR));

        let sync_upstream_ref =
            env::var("SYNC_UPSTREAM_REF").unwrap_or_else(|_| DEFAULT_SYNC_UPSTREAM_REF.to_string());

        let sync_cache_ttl_secs = match env::var("SYNC_CACHE_TTL_SECS") {
            Ok(raw) => raw.parse().map_err(|source| ConfigError::Parse {
                var: "SYNC_CACHE_TTL_SECS",
                source,
            })?,
            Err(_) => DEFAULT_SYNC_CACHE_TTL_SECS,
        };

        Ok(Self {
            database_url,
            bind_addr,
            db_max_connections,
            sync_cache_dir,
            sync_upstream_ref,
            sync_cache_ttl: Duration::from_secs(sync_cache_ttl_secs),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required env var: {0}")]
    Missing(&'static str),

    #[error("parsing env var {var}: {source}")]
    Parse {
        var: &'static str,
        #[source]
        source: std::num::ParseIntError,
    },
}
