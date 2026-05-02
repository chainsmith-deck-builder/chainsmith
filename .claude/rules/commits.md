# Commit message rules

This project uses [Conventional Commits 1.0.0](https://www.conventionalcommits.org/) (a.k.a. Commitizen style) for every commit on every branch. The format gives the log a machine-readable shape — future tooling can generate changelogs, infer semver bumps, and surface breaking changes — without asking much of the author beyond a brief discipline.

Enforcement is mechanical: `.githooks/commit-msg` validates the subject line against the Conventional Commits format on every commit. The hook is pure bash — no Node, no commitlint dependency. Body and footer formatting (72-col wrap, `BREAKING CHANGE:` shape) are still social conventions; only the subject is mechanically checked. If subject-line regex stops being enough later, the natural escalation is `commitlint-rs` or `cocogitto`.

## Format

```
<type>[(scope)][!]: <subject>

[body]

[footer(s)]
```

The subject line is the only required part. Type is required; scope is optional but encouraged when the change is localized.

## Types

Use one of:

- `feat` — a new user-visible feature
- `fix` — a bug fix
- `perf` — a performance improvement that is neither a feature nor a fix
- `refactor` — a code change that neither fixes a bug nor adds a feature
- `docs` — documentation only
- `style` — formatting, whitespace, no logic change
- `test` — adding or correcting tests
- `build` — changes to the build system, dependencies, packaging
- `ci` — changes to CI configuration or scripts
- `chore` — housekeeping, tooling, repo metadata
- `revert` — reverts a previous commit

If a change doesn't fit any of these, the commit is probably doing too many things. Split it.

## Scope

Scope is a noun in parentheses naming the area touched: module path or feature name. Examples we already have: `api`, `db`, `domain`, `auth`, `sync`, `health`, `error`, `config`. Don't invent new vocabulary per commit; reuse existing scope names where they fit.

```
feat(api): add /decks list endpoint
fix(db): retry once on dropped connection during migrations
refactor(error): replace string codes with ErrorCode enum
```

## Subject line

- **Imperative mood.** "add," not "added" or "adds." Read it as completing "If applied, this commit will…"
- **Lowercase** after the colon.
- **No trailing period.**
- **Under 72 characters.** GitHub truncates around 70 in some views.
- **Concrete.** "fix bug" is not enough. "fix off-by-one in pitch counter" is.

## Body (optional)

Add a body when the change benefits from explanation:

- Blank line after the subject.
- Wrap at 72 columns.
- Explain *why* and the relevant context — not *what*, the diff shows that.
- Reference issues in the footer (`Refs: #123`, `Closes: #123`), not the body prose.

## Breaking changes

Mark a breaking change with `!` in the subject **and** a `BREAKING CHANGE:` footer:

```
feat(api)!: rename /cards endpoint to /catalog

BREAKING CHANGE: the /cards endpoint has been renamed to /catalog.
Clients must update to the new path.
```

Use `!` only for genuinely breaking changes. What counts as breaking in this project depends on the phase model — see `api-contract.md` and `database.md`.

## Reverts

```
revert: feat(api): add /decks list endpoint

Refs: <sha-of-reverted-commit>
```

Use the `revert` type plus the original subject verbatim. Put the SHA of the reverted commit in a footer.

## Authorship trailers

This project does **not** use `Co-Authored-By:` trailers by default. The git log credits the human committer only; AI-assisted authorship (Claude, Copilot, Cursor, etc.) is implicit and does not appear in commit metadata.

Add a `Co-Authored-By:` trailer only when crediting a real human collaborator who pair-programmed on the change. When committing on behalf of the user, do not append a Claude attribution trailer even if a default behavior would otherwise add one — the rule in this file overrides that default.

## One change per commit

A commit should be one logical change. If you need the word "and" to describe what a commit does, it's two commits.

This applies even mid-branch: if you've made messy WIP commits, **squash them before merging** so what lands on `main` has a clean Conventional Commits log. The branch's pre-squash history is allowed to be messy; the merged history is not.

## Examples

Good:

```
feat(api): add /health endpoint
fix(db): handle dropped connection during migrations
refactor(error): replace string codes with ErrorCode enum
docs: link gitleaks install instructions in README
chore(deps): bump utoipa-axum to 0.2.0
ci: add cargo-audit to the check workflow
test(domain/format/cc): cover banned-card boundary case
```

Bad — and why:

- `Updated stuff` — no type, vague, wrong tense
- `feat: added new endpoint.` — past tense, trailing period
- `fix(api): Fix the thing` — capital F after colon, placeholder subject
- `feat: add deck endpoint and fix unrelated migration bug` — two changes in one commit; split
- `WIP` — never lands on `main`; squash it out before merge
