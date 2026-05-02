use std::env;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";
const DEFAULT_DB_MAX_CONNECTIONS: u32 = 10;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub db_max_connections: u32,
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

        Ok(Self {
            database_url,
            bind_addr,
            db_max_connections,
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
