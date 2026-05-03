//! Mint a development JWT for local API testing.
//!
//! Reads `AUTH_DEV_SECRET`, `AUTH_ISSUER`, `AUTH_AUDIENCE` from env (the
//! same vars the running server consumes in `AuthMode::DevSecret`). Prints
//! a signed HS256 token to stdout.
//!
//! Usage:
//!
//! ```text
//! cargo run --bin dev_jwt                        # default sub + 24h expiry
//! cargo run --bin dev_jwt <sub-uuid>             # custom sub
//! cargo run --bin dev_jwt <sub-uuid> <exp-hours> # custom expiry
//!
//! # Pipe straight into curl:
//! TOKEN=$(cargo run --quiet --bin dev_jwt)
//! curl -H "Authorization: Bearer $TOKEN" http://localhost:8080/decks
//! ```
//!
//! Not for production use — this signs with a symmetric secret. Anyone with
//! the secret can forge tokens.

use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::json;

const DEFAULT_SUB: &str = "550e8400-e29b-41d4-a716-446655440000";
const DEFAULT_AUDIENCE: &str = "authenticated";
const DEFAULT_EXP_HOURS: i64 = 24;

fn main() {
    dotenvy::dotenv().ok();

    let secret = std::env::var("AUTH_DEV_SECRET")
        .expect("AUTH_DEV_SECRET must be set (in .env or environment)");
    let issuer =
        std::env::var("AUTH_ISSUER").expect("AUTH_ISSUER must be set (in .env or environment)");
    let audience = std::env::var("AUTH_AUDIENCE").unwrap_or_else(|_| DEFAULT_AUDIENCE.to_string());

    let mut args = std::env::args().skip(1);
    let sub = args.next().unwrap_or_else(|| DEFAULT_SUB.to_string());
    let exp_hours: i64 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_EXP_HOURS);

    let exp = (Utc::now() + Duration::hours(exp_hours)).timestamp() as usize;
    let claims = json!({
        "sub": sub,
        "iss": issuer,
        "aud": audience,
        "exp": exp,
    });

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("signing JWT");

    println!("{token}");
}
