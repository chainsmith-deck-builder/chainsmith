# Security rules

Hygiene, not paranoia. The threat model is "an internet-exposed API holding user-trusted data."

## JWT verification

Supabase Auth issues JWTs. This service verifies them.

- Use the `jsonwebtoken` crate, version 10.3 or higher. Lower versions have GHSA-h395-gr6q-cpjc, a type-confusion authorization bypass. Pin to a specific 10.x release in `Cargo.toml`, not a range.
- Verify the signature, the `iss`, the `aud`, and the `exp` on every request. Do not skip any of these.
- Prefer asymmetric keys via Supabase's JWKS endpoint over the HS256 shared-secret path. The JWKS URL goes in env config. Cache JWKS responses for the rotation period the issuer advertises.
- The auth middleware in `auth/middleware.rs` is the single place that validates tokens. Handlers receive an authenticated user as an Axum extractor, not a raw token.
- A failed verification returns 401 with a generic message. Do not leak which check failed.

## Secrets

- Never commit secrets. Lockfiles are committed, `.env` is not. Add a `.env.example` documenting the required variables with placeholder values.
- Read secrets from env vars in `config.rs`. Fail loudly at startup if a required secret is missing. Do not default to empty strings.
- Do not log secret values, even at debug level. If you must log something derived from a secret, log a short fixed-length hash prefix, never the raw value.

## Dependencies

- Lockfiles (`Cargo.lock`) are committed.
- Dependency versions are pinned to specific releases in `Cargo.toml`. No major-version floats (no `"1"` or `"1.*"` patterns). Use exact versions like `"1.4.2"`.
- `cargo-audit` runs in CI against the RustSec advisory database. A new advisory blocks the build until addressed (fix, mitigate, or accept-with-justification in `audit.toml`).
- Renovate or Dependabot watches for updates. Triage at least weekly during active development.
- Before adding a new dependency: check its maintenance status, recent activity, and download stats. Prefer crates from established orgs (tokio-rs, serde-rs, launchbadge, hyperium) over single-maintainer hobby projects for anything load-bearing.

## TLS and crypto provider

- The default rustls provider in this project is `ring`, brought in transitively via `reqwest` and `jsonwebtoken`.
- Ring had maintainer-stability issues in the past. Rustls maintainers took over crates.io ownership and the situation is currently stable. Monitor.
- Off-ramps if it degrades:
  - Switch the rustls TLS provider to `aws-lc-rs`
  - Switch jsonwebtoken to a `rustcrypto`-backed fork
  - Document the switch in the project change log

## Input validation

- Trust nothing from the client. Validate every input against the API contract before any business logic runs.
- Validation runs in two layers:
  1. Request DTOs derive serde with strict rules. Use `#[serde(deny_unknown_fields)]` on every request struct.
  2. The validation engine in `domain/` enforces semantic rules (deck legality, format constraints, etc.).
- Do not pass user input directly to SQL. sqlx parameter binding handles this when you use the macros, but if you ever drop down to dynamic queries, parameterize explicitly.
- Cap request body size globally via Axum's body limit. The default is too generous for an API like this.

## Card images

Card images are served by LSS's CDN. We surface the per-printing CDN URL on each `Printing` record (sourced from the upstream sync) and clients (web, iOS, Android) load images directly from LSS. We do not proxy or cache imagery on our infrastructure.

Reasoning:

- LSS owns the card art. Hot-linking is the same posture FaBrary, FaBDB, and Talishar take and is currently tolerated. Routing images through our infrastructure — even as a thin pass-through with Cloudflare caching — would have us redistributing LSS imagery on cache hits, which is a stronger infringement claim than direct linking.
- The URL is upstream-controlled. When LSS rotates URLs, the next sync picks them up; clients always re-request the URL from our API rather than caching the URL string, so they get the new URL on the next call.
- Web clients load via plain `<img src="...">` (no CORS implications for image display). Mobile clients use native HTTP and have nothing to configure.

Engineering rules that follow from this:

- The `image_url` field on `Printing` is `Option<String>`. When upstream omits it, surface `None` and let the client show a placeholder. Never substitute our own.
- Do not introduce a `/images/{id}` proxy endpoint. If LSS ever blocks hot-linking via Referer header, the response is to negotiate with LSS for an explicit allowance for our origin, not to silently start re-serving their imagery.
- Surface attribution where appropriate (a credits/legal page is sufficient). Do not misrepresent the image source.

## Rate limiting and abuse

Not enforced in pre-launch. When added in production:

- Per-IP and per-authenticated-user limits.
- Document the headers and limits in the OpenAPI spec.
- A circuit breaker around the card data sync to avoid hammering GitHub on retry storms.

## Disclosure

This project does not yet have a published security policy. When it does, link it from this file. Until then, the response to any security report is to treat it seriously and fix it quickly, even informally.
