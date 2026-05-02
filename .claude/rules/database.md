# Database rules

Postgres on Supabase, accessed via sqlx with compile-time checked queries. There is no ORM. Application connections go through Supabase's pooler endpoint on port 6543. Migrations go through the direct Postgres endpoint on port 5432.

## Connection setup

- Application connection: pooler endpoint, port 6543. The pooler is in transaction mode and does not support session-level features.
- Migrations connection: direct Postgres endpoint, port 5432. sqlx-cli reads `DATABASE_URL` for migrations, so use a separate env var (`DATABASE_MIGRATION_URL`) and switch the env in CI as needed.
- Connection strings come from env vars only, never hardcoded. See `config.rs`.
- Set a sensible pool max (start at 10, tune from observed load). Do not leave it at the sqlx default.

## sqlx usage

- All queries use the `query!`, `query_as!`, or `query_scalar!` macros for compile-time checking.
- The macros require either a populated `.sqlx` cache committed to the repo, or a live `DATABASE_URL` at compile time. Prefer the committed cache for CI.
- After changing or adding a query, run `cargo sqlx prepare` and commit the resulting `.sqlx` files in the same PR as the code change.
- Dynamic queries are rare. If you genuinely need a runtime-built query string, use `sqlx::query` (no bang) and add a code comment explaining why a static macro form did not work.
- Bind parameters via positional placeholders (`$1`, `$2`). Never concatenate user input into SQL strings.

## Row vs domain types

- DB row structs (`CardRow`, `DeckRow`) live in `db/`. They mirror the SQL schema, with column-for-column field types.
- Domain types (`Card`, `Deck`) live in `domain/`. They are the shape the validation engine and API care about.
- Convert between them at the `db/` boundary. Never let `Row` types leak out of `db/`.
- If a domain type and a row type happen to be identical, keep them as separate type aliases anyway. They will diverge.

## Migrations

Migration files live in `/migrations` and are run via `sqlx migrate run`.

### Pre-launch phase

- Migrations can be edited, squashed, or deleted as the schema evolves. The dev database is recreatable.
- Before transitioning to production, squash the migration history into a clean baseline. The first migration in the production-era history should produce the schema exactly as it was at the moment of cutover.

### Production phase

- Append-only. Never edit a migration that has been applied to a non-throwaway database.
- Destructive changes (drop column, rename column, change type) go through expand-and-contract: add new, dual-write, backfill, switch reads, drop old. Each step is its own migration.
- Never drop a column in the same migration that adds its replacement.
- A single migration should be runnable in a single transaction unless there is a specific reason it cannot (e.g. CONCURRENTLY-built indexes). Document the exception in the migration file.

## Schema conventions

- Tables: `snake_case`, plural (`cards`, `decks`, `deck_cards`).
- Primary keys: `id`, type `uuid` with `gen_random_uuid()` default, unless there is a reason to use a different type.
- Timestamps: `created_at` and `updated_at` (`timestamptz`, default `now()`). Decks and other user-owned entities also have `deleted_at` for soft delete.
- Foreign keys: explicit, named `<other_table_singular>_id`. Always have an index on the foreign key column.
- Booleans named with positive sense (`is_published`, not `is_unpublished`).

## Indexing

- Default to no index. Add one when query patterns demand it, not preemptively.
- Document each non-obvious index with a comment in the migration explaining the query it serves.
- Composite indexes: order columns by selectivity then by query usage. Test with `EXPLAIN` before merging.

## Card data sync

The card data sync job pulls from `the-fab-cube/flesh-and-blood-cards` on GitHub. Rules:

- Treat the upstream JSON as source of truth for card facts. Do not edit cards in the local DB outside of the sync.
- Sync is idempotent. Running it twice in a row produces no diff.
- Sync runs in a transaction. If it fails partway, the DB is unchanged.
- Schema changes upstream require a code change here. Pin to specific commits or tags when possible to avoid silent breakage.
- Log a structured summary at the end of each sync (rows added, updated, untouched). This is the easiest signal that the job is healthy.
