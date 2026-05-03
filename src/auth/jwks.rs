//! JWKS fetcher with TTL-based caching.
//!
//! Supabase rotates signing keys on a slow cadence. We fetch the JWKS once
//! at startup (or on first auth request), cache for the configured TTL, and
//! refresh lazily when the next request arrives after expiry. If a token's
//! `kid` is not found in the cached set we trigger an immediate refresh —
//! handles the case where Supabase has rotated keys mid-TTL.

use std::time::{Duration, Instant};

use jsonwebtoken::{jwk::JwkSet, DecodingKey};
use tokio::sync::RwLock;

use super::AuthError;

pub struct JwksCache {
    url: String,
    client: reqwest::Client,
    state: RwLock<CacheState>,
    ttl: Duration,
}

#[derive(Default)]
struct CacheState {
    jwks: Option<JwkSet>,
    fetched_at: Option<Instant>,
}

impl JwksCache {
    pub fn new(client: reqwest::Client, url: impl Into<String>, ttl: Duration) -> Self {
        Self {
            url: url.into(),
            client,
            state: RwLock::new(CacheState::default()),
            ttl,
        }
    }

    /// Look up a decoding key by `kid`. Refreshes the cache if missing or
    /// stale; refreshes again on `kid` miss in case keys just rotated.
    pub async fn key_for_kid(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        if let Some(key) = self.try_key(kid).await? {
            return Ok(key);
        }
        // Possible mid-TTL key rotation: force a refresh and retry once.
        self.refresh().await?;
        self.try_key(kid).await?.ok_or(AuthError::KeyNotFound)
    }

    async fn try_key(&self, kid: &str) -> Result<Option<DecodingKey>, AuthError> {
        self.ensure_fresh().await?;
        let state = self.state.read().await;
        let Some(jwks) = state.jwks.as_ref() else {
            return Ok(None);
        };
        let Some(jwk) = jwks.find(kid) else {
            return Ok(None);
        };
        DecodingKey::from_jwk(jwk)
            .map(Some)
            .map_err(|e| AuthError::Jwks(e.to_string()))
    }

    async fn ensure_fresh(&self) -> Result<(), AuthError> {
        let needs_refresh = {
            let state = self.state.read().await;
            match state.fetched_at {
                Some(at) => at.elapsed() >= self.ttl,
                None => true,
            }
        };
        if needs_refresh {
            self.refresh().await?;
        }
        Ok(())
    }

    async fn refresh(&self) -> Result<(), AuthError> {
        let resp = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|e| AuthError::Jwks(e.to_string()))?
            .error_for_status()
            .map_err(|e| AuthError::Jwks(e.to_string()))?;
        let jwks: JwkSet = resp
            .json()
            .await
            .map_err(|e| AuthError::Jwks(e.to_string()))?;
        let mut state = self.state.write().await;
        state.jwks = Some(jwks);
        state.fetched_at = Some(Instant::now());
        tracing::debug!(url = %self.url, "JWKS refreshed");
        Ok(())
    }
}
