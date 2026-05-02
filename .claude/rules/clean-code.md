# Clean code rules

Project-specific clean-code preferences for Rust code in this repo. Curated, not orthodox — some of Uncle Bob's *Clean Code* rules hold up, others don't, and modern thinking (Ousterhout's *A Philosophy of Software Design*) plus direct experience with LLM-generated code adds rules of its own.

This file overlaps with `rust.md` and the global `~/.claude/CLAUDE.md`. Where they differ, the more specific wins (this file > `rust.md` > global).

## Functions

- **One job per function.** A function is a verb. If the name needs an "and," split it. *Validate, then save* is two functions, not one.
- **Don't extract for the sake of extraction.** Single-use helpers that exist only to make a parent function shorter usually hurt readability — the reader jumps around, and the name lies about how reusable the code is. Inline beats extract when the helper would be called once and is not independently testable.
- **Depth over surface.** A function with a small interface and a substantial body is usually better than four wrappers around a one-liner. Ousterhout's "deep module" idea applied to functions.
- **Parameter count is a smell, not a hard rule.** Three is fine, four is fine, six is a smell — but the fix is a struct, not a field on `self` or `AppState`. Don't move parameters into shared state to dodge a parameter count.
- **Pure where possible, side-effecting where necessary, never both.** A function either computes a value or performs IO. Mixing them makes testing painful. The `domain/` module is kept pure for this reason; respect that boundary.

## Naming

- **Names reveal intent, not implementation.** `fetch_card_by_id` describes implementation; `card` describes intent. Prefer the shorter form when context makes the implementation obvious.
- **Use domain language.** When the FaB rules say *hero*, *pitch*, *talent*, the code says hero, pitch, talent. Don't invent generic synonyms (*item*, *value*, *category*) for things that already have a name in the domain. See `fab-domain.md` for the canonical vocabulary.
- **Length scales with scope.** A closure variable can be `c`. A function-level variable used across 30 lines should be `legal_cards`. A struct field referenced from many call sites should be unambiguous on its own (`legal_cards_by_format`).
- **Verbs for functions, nouns for types and values.** `validate_deck` not `deck_validation`. `Card` not `CardData`. `is_legal` not `legality_check`.
- **Prefer concrete over generic.** `decks` beats `items`. `parse_deck_export` beats `process_input`.

## Comments

- **Explain *why*, not *what*.** The code shows what. Comments exist for context the reader can't deduce: a workaround, a non-obvious tradeoff, a link to a bug, a domain rule that justifies an otherwise odd-looking branch.
- **Comments are not a failure.** Uncle Bob argued they are; that hasn't aged well. A short comment explaining intent is cheaper than a heroic rename, and some context (LSS rulings, GHSA references, performance tradeoffs) genuinely cannot live in a name.
- **Update or delete drifting comments.** A wrong comment is worse than no comment. If you change behavior, scan for comments above and below the change.
- **Module-level doc comments earn their keep.** A `//!` at the top of `domain/format/classic_constructed.rs` explaining what the module enforces saves a future reader twenty minutes.
- **No commit-message comments in code.** "Added to fix #1234" belongs in the commit, not the source.

## Avoiding over-engineering

The most common failure mode in LLM-generated code is solving problems that don't exist yet. Rules to push back:

- **YAGNI.** Don't add a parameter, trait impl, generic, or config flag unless something in this repo uses it now. "We might want this later" is an entry in the issue tracker, not code.
- **No speculative abstractions.** One impl, no trait. One value, no config option. Concrete code is cheaper to generalize later than a wrong abstraction is to undo.
- **No premature builders or `Default` impls.** Add them when a third caller needs them, not before.
- **No wrapper types without a reason.** A newtype exists to enforce an invariant or distinguish two semantically different `String`s. It does not exist to be tidy.
- **No defensive `Option<T>` or `Result<T, E>` in return types.** If the function cannot fail, its return type should not lie about it. If a value is always present, don't wrap it in `Option`.
- **No "just in case" `pub`.** Visibility is the smallest scope that compiles.
- **Don't anticipate parallelism.** Don't add `Arc<Mutex<_>>` or `tokio::spawn` until profiling or a concrete requirement says you need them.

## When to break a rule

Every rule above has exceptions. The bar for breaking one is:

1. Name the rule you are breaking, in the code comment or the PR description.
2. Explain why the alternative is worse for this specific case.
3. Be willing to defend it in review.

If you can't do all three, follow the rule.
