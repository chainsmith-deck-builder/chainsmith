# Rust rules

Project-specific Rust rules. Generic clean-code preferences live in the global `~/.claude/CLAUDE.md`.

## Crate choices

The default crate set is fixed for this project. Do not introduce a competing crate without flagging it for review.

- HTTP framework: `axum`
- Async runtime: `tokio` (multi-thread runtime)
- Database: `sqlx` with compile-time checked queries against Postgres
- JSON: `serde` and `serde_json`
- Outbound HTTP: `reqwest` (rustls TLS, no native-tls)
- JWT verification: `jsonwebtoken` 10.3 or higher (see `security.md` for the version pin reason)
- OpenAPI generation: `utoipa` and `utoipa-axum`
- Logging and tracing: `tracing` and `tracing-subscriber`
- Errors: `thiserror` for library and domain errors, `anyhow` only at the binary boundary

If a need arises for a crate not on this list, propose it with a one-line justification and a link to its docs.

## Error handling

- Library and domain code returns `Result<T, E>` with concrete `thiserror`-derived enums. Never `anyhow::Result<T>` inside the validation engine or DB layer.
- Handlers return `Result<impl IntoResponse, AppError>` where `AppError` implements `IntoResponse` and maps to the standard JSON error shape (see `api-contract.md`).
- Never `.unwrap()` or `.expect()` outside of tests, build scripts, or paths where the panic is genuinely unreachable. If you do use `.expect()`, the message must explain why the path is unreachable.
- Do not swallow errors. Log them with context via `tracing` and propagate.

## Naming

- Modules: `snake_case`
- Types: `PascalCase`
- Functions and variables: `snake_case`
- Const and static: `SCREAMING_SNAKE_CASE`
- Database row structs end in `Row` (e.g. `CardRow`)
- Domain types do not (e.g. `Card`)
- DTOs sent over the wire end in `Dto`, `Request`, or `Response` (e.g. `CreateDeckRequest`, `DeckResponse`)

## Async

- Default to `async fn` for handlers and IO. Do not block the runtime.
- Use `tokio::spawn` for fire-and-forget work, but log on JoinHandle errors.
- CPU-bound work goes on `tokio::task::spawn_blocking`.
- Hold `MutexGuard` and similar non-Send values only inside synchronous scopes. Drop before any `.await`.

## Logging

- Use `tracing` macros (`info!`, `warn!`, `error!`, `debug!`). Never `println!` or `eprintln!` in committed code.
- Each request handler instruments a span with the request id, route, and authenticated user id when present.
- Do not log PII (email addresses, raw tokens, full card-collection contents). When in doubt, log a hash or a fixed-length prefix.

## Compiler settings

- `RUSTFLAGS="-D warnings"` locally before commit. CI enforces this.
- `clippy::all` is enabled by default. Address lints before merging.
- `clippy::pedantic` is not enabled by default. Opt in per-crate or per-module if it earns its keep.

## Module layout

```
src/
  main.rs             # binary entrypoint, builds Axum router and starts server
  lib.rs              # re-exports for integration tests
  config.rs           # env var parsing
  error.rs            # AppError, IntoResponse impl
  state.rs            # AppState struct held by the router
  auth/
    jwks.rs           # JWKS fetch and cache
    middleware.rs     # auth extractor for Axum
  db/
    mod.rs            # pool setup
    card.rs           # card queries
    deck.rs           # deck queries
  domain/             # validation engine, no IO
    card.rs
    deck.rs
    format/
      mod.rs
      classic_constructed.rs
      blitz.rs
      commoner.rs
  api/
    mod.rs            # route registration
    cards.rs          # /cards handlers
    decks.rs          # /decks handlers
  sync/               # nightly card data sync
    fab_cube.rs
```

The `domain` module is pure: no IO, no clock reads, no env access. The `api` module handles HTTP. The `db` module handles SQL. Keep these separated. The validation engine in `domain/` must be testable without a database.

## Style nits

- Prefer `match` over chained `if let` when matching multiple variants.
- Prefer iterator chains over manual loops when the chain stays under three operations.
- Avoid one-letter variable names except for closures over short-lived values (`|x| ...`).
- Comments explain why, not what. The code shows what.
