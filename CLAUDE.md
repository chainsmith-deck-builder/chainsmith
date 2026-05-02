# Chainsmith Backend

Headless Rust API and validation engine for the Chainsmith Flesh and Blood deck builder. The web client (`chainsmith-web`) talks to this service over HTTP. Future iOS and Android clients will too.

This repo contains the API, the validation engine, the database access layer, and the card data sync. No UI code lives here.

## Stack

- HTTP: Axum on Tokio
- Database: Postgres (Supabase-hosted), accessed via sqlx with compile-time checked queries
- Auth: Supabase Auth issues JWTs, this service verifies signatures via the `jsonwebtoken` crate using JWKS
- Outbound HTTP: reqwest (card image proxy, GitHub data sync)
- Serialization: serde and serde_json
- API contract: utoipa derives OpenAPI 3.x annotations on Axum handlers, emitted to `openapi.json` in CI
- Card data: synced nightly from `the-fab-cube/flesh-and-blood-cards` on GitHub
- Build and deploy: Dockerfile, target Fly.io (provisional)

## Project phase

**Current phase**: `pre-launch` <!-- Change to `production` when real users have data -->

The phase governs how aggressive the rules in `.claude/rules/` are about backwards compatibility. In `pre-launch` we can break API shapes, squash migrations, and rename anything. In `production` we cannot, and the rules become strict additive-only.

If you are unsure which phase you are in, read this file. Do not assume.

## Hard requirements (regardless of phase)

- Strict testing discipline. Negative cases and variants are required, not optional. No snapshot-as-coverage. If a test fails, investigate before rewriting it. See `.claude/rules/testing.md`.
- Security hygiene from day one. cargo-audit runs in CI, dependencies are pinned, lockfile is committed. See `.claude/rules/security.md`.
- Compile-time SQL checking via sqlx. No runtime-string SQL except for a documented reason. See `.claude/rules/database.md`.

## Rules files

Read these for their respective domains:

- `.claude/rules/rust.md`: Rust style, error handling, crate choices
- `.claude/rules/clean-code.md`: function design, naming, comments, anti-over-engineering
- `.claude/rules/commits.md`: Conventional Commits / Commitizen-style commit messages
- `.claude/rules/api-contract.md`: utoipa, OpenAPI, JSON shapes, phase-dependent additive rules
- `.claude/rules/database.md`: sqlx, migrations (phase-dependent), Postgres specifics, Supabase pooler
- `.claude/rules/testing.md`: testing discipline
- `.claude/rules/security.md`: JWT verification, dependency hygiene, supply chain
- `.claude/rules/fab-domain.md`: FaB terminology, validation engine rules

## Git hooks

Two hooks live in `.githooks/`, wired up by one `git config core.hooksPath .githooks` per clone:

- **`pre-commit`** — scans staged changes for secrets via `gitleaks`, then runs `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test --lib --bins`. Integration tests in `tests/` are deliberately excluded — they need Postgres and run in CI.
- **`commit-msg`** — validates the commit subject line against `.claude/rules/commits.md` (Conventional Commits 1.0). Pure bash, no commitlint dependency.

One-time setup per clone:

```bash
git config core.hooksPath .githooks
```

If `gitleaks` is not on PATH, install it (`winget install Gitleaks.Gitleaks` on Windows, see the [gitleaks readme](https://github.com/gitleaks/gitleaks) elsewhere). The pre-commit hook hard-fails when gitleaks is missing — secret scanning is not optional.

## Common commands

```bash
# Run the server locally
cargo run

# Run tests
cargo test

# Compile-time SQL check (requires a populated .sqlx cache committed to the repo,
# or a live DATABASE_URL at compile time)
cargo sqlx prepare --check

# Run migrations (uses the direct Postgres endpoint, port 5432, not the pooler)
sqlx migrate run

# Audit dependencies
cargo audit

# Generate OpenAPI spec to stdout
cargo run --bin export_openapi > openapi.json
```

## Definition of done

A change is done when:

- Code compiles without warnings (treat warnings as errors locally and in CI)
- Tests pass, including negative and variant cases for the changed code
- New endpoints have utoipa annotations and appear in `openapi.json`
- New SQL queries are compile-time checked by sqlx and the `.sqlx` cache is committed
- Migration files are append-only if in `production` phase, freely editable in `pre-launch`
- cargo-audit reports no new advisories
- Manual smoke test against a local Postgres or staging Supabase for endpoints that touch the DB

## What lives in this repo vs elsewhere

In this repo:

- HTTP API
- Validation engine (pure, no IO)
- Database schema and migrations
- Card data sync job
- OpenAPI spec generation

Not in this repo:

- Any UI (web client lives in `chainsmith-web`, future native clients in their own repos)
- Generated TypeScript or other client code (consumers regenerate from `openapi.json`)
- Issue tracking that spans repos (use the `tracker` repo in the same org)
