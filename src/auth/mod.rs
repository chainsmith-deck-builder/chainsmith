//! JWT verification for the public API.
//!
//! Production verifies Supabase-issued tokens against a JWKS endpoint.
//! Supabase signs with an asymmetric algorithm advertised in the JWT
//! header; we trust the header's `alg` only after checking it against an
//! explicit allowlist (`RS256`, `ES256`) so a hostile token can't downgrade
//! verification to a different scheme. Dev / local environments may use a
//! static HS256 secret; tests use that mode too. When no auth config is
//! set the server still serves public routes (catalog, validate, health)
//! but auth-protected routes refuse with 503 `auth_not_configured`.

pub mod jwks;
pub mod middleware;

use std::sync::Arc;

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use uuid::Uuid;

pub use middleware::AuthenticatedUser;

use crate::auth::jwks::JwksCache;

/// Per-process auth configuration. Held inside `AppState` and consulted by
/// the `AuthenticatedUser` extractor.
#[derive(Clone)]
pub struct AuthContext {
    pub mode: AuthMode,
}

#[derive(Clone)]
pub enum AuthMode {
    /// Production: verify tokens against the Supabase JWKS endpoint. The
    /// JWT header's `alg` is honored, but only after vetting through
    /// `supported_jwks_alg` (currently RS256 and ES256).
    Jwks {
        jwks: Arc<JwksCache>,
        issuer: String,
        audience: String,
    },
    /// Dev / test: verify HS256 tokens with a shared secret. NOT for
    /// production — the secret is symmetric and would let anyone holding
    /// it forge tokens.
    DevSecret {
        secret: Arc<String>,
        issuer: String,
        audience: String,
    },
    /// No auth configured. Auth-protected routes return 503; public routes
    /// still work.
    Disabled,
}

#[derive(Debug, Deserialize)]
pub struct Claims {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    pub iss: String,
    pub aud: String,
    pub exp: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing Authorization header")]
    MissingHeader,
    #[error("malformed Authorization header")]
    MalformedHeader,
    #[error("invalid or expired token")]
    InvalidToken,
    #[error("token signing key id missing or unknown")]
    KeyNotFound,
    #[error("auth is not configured on this server")]
    NotConfigured,
    #[error("JWKS upstream error: {0}")]
    Jwks(String),
}

impl AuthContext {
    /// Validate `token` and return the verified claims. Errors map to either
    /// 401 (token problems) or 503 (server misconfigured); the extractor
    /// decides the HTTP status.
    pub async fn verify(&self, token: &str) -> Result<Claims, AuthError> {
        match &self.mode {
            AuthMode::Jwks {
                jwks,
                issuer,
                audience,
            } => {
                let header = decode_header(token).map_err(|_| AuthError::InvalidToken)?;
                let kid = header.kid.ok_or(AuthError::KeyNotFound)?;
                let alg = supported_jwks_alg(header.alg)?;
                let key = jwks.key_for_kid(&kid).await?;
                decode_with(token, &key, alg, issuer, audience)
            }
            AuthMode::DevSecret {
                secret,
                issuer,
                audience,
            } => {
                let key = DecodingKey::from_secret(secret.as_bytes());
                decode_with(token, &key, Algorithm::HS256, issuer, audience)
            }
            AuthMode::Disabled => Err(AuthError::NotConfigured),
        }
    }
}

fn decode_with(
    token: &str,
    key: &DecodingKey,
    alg: Algorithm,
    issuer: &str,
    audience: &str,
) -> Result<Claims, AuthError> {
    let mut validation = Validation::new(alg);
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[audience]);
    decode::<Claims>(token, key, &validation)
        .map(|data| data.claims)
        .map_err(|_| AuthError::InvalidToken)
}

/// Allowlist of asymmetric algorithms accepted via the JWKS path. Supabase
/// rotated to `ES256` for project signing keys; older or self-hosted
/// deployments may still issue `RS256`. Anything outside this list — `none`,
/// HS256, etc. — is rejected, since a JWKS key resolved by `kid` may
/// otherwise be misused with a different algorithm than it was minted for.
fn supported_jwks_alg(alg: Algorithm) -> Result<Algorithm, AuthError> {
    match alg {
        Algorithm::RS256 | Algorithm::ES256 => Ok(alg),
        _ => Err(AuthError::InvalidToken),
    }
}

/// Convert a `sub` claim string into a UUID. Supabase user IDs are UUIDs,
/// so this should always succeed for a real Supabase JWT.
pub fn parse_subject(sub: &str) -> Result<Uuid, AuthError> {
    Uuid::parse_str(sub).map_err(|_| AuthError::InvalidToken)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{Duration as CDuration, Utc};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;

    use super::*;

    const TEST_ISSUER: &str = "https://test.example/auth";
    const TEST_AUDIENCE: &str = "authenticated";
    const TEST_SECRET: &str = "test-secret-very-long-and-cryptographically-irrelevant";
    const TEST_SUB: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn dev_context(secret: &str) -> AuthContext {
        AuthContext {
            mode: AuthMode::DevSecret {
                secret: Arc::new(secret.into()),
                issuer: TEST_ISSUER.into(),
                audience: TEST_AUDIENCE.into(),
            },
        }
    }

    fn make_token(secret: &str, sub: &str, iss: &str, aud: &str, expires_in_secs: i64) -> String {
        let exp = (Utc::now() + CDuration::seconds(expires_in_secs)).timestamp() as usize;
        let claims = json!({
            "sub": sub,
            "iss": iss,
            "aud": aud,
            "exp": exp,
        });
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn it_accepts_valid_dev_secret_token() {
        let ctx = dev_context(TEST_SECRET);
        let token = make_token(TEST_SECRET, TEST_SUB, TEST_ISSUER, TEST_AUDIENCE, 3600);
        let claims = ctx.verify(&token).await.unwrap();
        assert_eq!(claims.sub, TEST_SUB);
        assert_eq!(claims.iss, TEST_ISSUER);
        assert_eq!(claims.aud, TEST_AUDIENCE);
    }

    #[tokio::test]
    async fn it_rejects_expired_token() {
        let ctx = dev_context(TEST_SECRET);
        let token = make_token(TEST_SECRET, TEST_SUB, TEST_ISSUER, TEST_AUDIENCE, -3600);
        let err = ctx.verify(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn it_rejects_wrong_issuer() {
        let ctx = dev_context(TEST_SECRET);
        let token = make_token(
            TEST_SECRET,
            TEST_SUB,
            "https://other.example/auth",
            TEST_AUDIENCE,
            3600,
        );
        let err = ctx.verify(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn it_rejects_wrong_audience() {
        let ctx = dev_context(TEST_SECRET);
        let token = make_token(
            TEST_SECRET,
            TEST_SUB,
            TEST_ISSUER,
            "different-audience",
            3600,
        );
        let err = ctx.verify(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn it_rejects_token_signed_with_wrong_secret() {
        let ctx = dev_context(TEST_SECRET);
        let token = make_token(
            "different-secret",
            TEST_SUB,
            TEST_ISSUER,
            TEST_AUDIENCE,
            3600,
        );
        let err = ctx.verify(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn it_returns_not_configured_when_auth_is_disabled() {
        let ctx = AuthContext {
            mode: AuthMode::Disabled,
        };
        let err = ctx.verify("any.jwt.string").await.unwrap_err();
        assert!(matches!(err, AuthError::NotConfigured));
    }

    #[test]
    fn it_parses_uuid_subject() {
        let id = parse_subject(TEST_SUB).unwrap();
        assert_eq!(id.to_string(), TEST_SUB);
    }

    #[test]
    fn it_rejects_non_uuid_subject() {
        assert!(matches!(
            parse_subject("not-a-uuid"),
            Err(AuthError::InvalidToken)
        ));
    }

    #[test]
    fn it_accepts_rs256_for_jwks() {
        assert_eq!(
            supported_jwks_alg(Algorithm::RS256).unwrap(),
            Algorithm::RS256
        );
    }

    #[test]
    fn it_accepts_es256_for_jwks() {
        // Supabase rotated to ES256; this is the path most deployments hit.
        assert_eq!(
            supported_jwks_alg(Algorithm::ES256).unwrap(),
            Algorithm::ES256
        );
    }

    #[test]
    fn it_rejects_hs256_via_jwks_path() {
        // HS256 has no business arriving via JWKS — symmetric keys aren't
        // published there. Refuse rather than risk a key-confusion attack
        // where a JWKS-resolved public key gets used as an HMAC secret.
        assert!(matches!(
            supported_jwks_alg(Algorithm::HS256),
            Err(AuthError::InvalidToken)
        ));
    }
}
