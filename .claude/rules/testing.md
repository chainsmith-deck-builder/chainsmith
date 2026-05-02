# Testing rules

Strict testing discipline applies regardless of phase. This is not negotiable.

## What "tested" means

A function or endpoint is tested when it has:

1. At least one happy-path test confirming it does what its name says.
2. At least one negative-path test confirming it fails the way it should when given bad input.
3. Variant coverage for every meaningfully distinct input class. "It works for one input" is not coverage.

Snapshot tests do not count as coverage on their own. They confirm the output has not changed, not that it is correct. A snapshot test is acceptable as a regression guard alongside real assertions, never instead of them.

## When a test fails

A failing test is a finding, not a chore. Investigate before rewriting it.

The order of operations:

1. Read what the test is asserting and what actually happened.
2. Decide whether the production code is wrong or the test is wrong.
3. If the production code is wrong, fix the production code. Do not weaken the assertion to make the test pass.
4. If the test is wrong, fix the test, and add a comment explaining what the test was originally trying to assert and why the new version is correct.
5. If you cannot tell which is wrong, stop and ask before changing either.

Never delete a failing test to "clean up" without an explicit, documented reason.

## Test layout

- Unit tests live in `#[cfg(test)] mod tests` blocks at the bottom of the module they test.
- Integration tests live in `tests/` and exercise the HTTP layer end-to-end against a real Postgres test database.
- Domain logic in `domain/` is tested with pure unit tests. No DB, no HTTP.
- DB queries in `db/` are tested with integration tests against a real Postgres instance.
- Handlers in `api/` are tested via the integration tests in `tests/`.

## Test database

- Integration tests run against a dedicated test database, isolated per test where useful via transactions that roll back at the end.
- Never run tests against a shared dev database.
- The test database setup is automated. If it is not, fix the setup, do not work around it.
- Tests that require a fresh database (migration tests, for example) are explicit about it and run serially.

## Test naming

- Unit tests: `fn it_<does_thing>_when_<condition>()`. The name should read as a sentence describing what is true.
- Integration tests: `<endpoint>_<scenario>` (e.g. `create_deck_rejects_unauthenticated`).
- Avoid `test_foo` style names. They do not describe what is being asserted.

## What to test for the validation engine

The FaB validation engine has high blast radius. Every format rule it implements gets:

- A test for the canonical legal case
- A test for each canonical illegal case (banned card, wrong hero class, oversized deck, ineligible equipment, and so on)
- A test for boundary cases (exactly at the deck size limit, exactly at the legality cutoff date)
- A regression test for any bug fixed in the engine, with a comment linking to the issue or the bug report

See `fab-domain.md` for the format and rule definitions the engine implements.

## What not to test

- Generated code (utoipa-generated schemas, sqlx-generated types). Their generators are tested upstream.
- Pure data structures with no behavior.
- Trivial wrappers that do nothing but forward to a tested function.
- Third-party crate behavior. If you find yourself testing that `serde_json::to_string` works, stop.

## Test data

- Fixtures live in `tests/fixtures/` as JSON or SQL. Avoid huge inline literals in test files.
- Card fixtures should be a small representative subset, not the full upstream dataset.
- Each fixture has a comment at the top explaining what scenarios it is designed to cover.
