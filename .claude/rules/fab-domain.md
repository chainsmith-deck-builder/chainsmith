# Flesh and Blood domain rules

This file captures the FaB-specific knowledge the codebase needs. Keep it short and current. Defer to the official Comprehensive Rules and the Living Legend / B&R announcements at fabtcg.com for anything not stated here.

## Terminology

- **Hero**: A card that defines the deck. Decks are built around exactly one hero.
- **Class**: The faction the hero belongs to (Guardian, Warrior, Wizard, Mechanologist, Brute, Ninja, Runeblade, Ranger, Illusionist, and so on). Most non-hero cards are restricted to one class plus Generic.
- **Talent**: A subtype that further restricts deck inclusion (Light, Shadow, Elemental, Earth, Ice, Lightning, etc.). Some heroes have talents, some do not.
- **Pitch**: A card's pitch value, color-coded as Red (1 resource), Yellow (2), Blue (3).
- **Equipment**: Cards that go in equipment slots (head, chest, arms, legs, weapons, off-hand). Slot rules vary by format.
- **Combat chain**: The sequence of attacks and defenses played in a turn. The namesake of this product. Not directly modeled by the deck builder.
- **Living Legend**: A rotation status that retires powerful heroes from Classic Constructed once their LL points threshold is hit. Cards from those heroes' sets remain legal where the format allows.
- **Young hero**: A reduced-statline version of a hero used in Blitz and certain other formats.

## Formats the validation engine supports

Each format has its own module under `domain/format/`. The validation engine never hardcodes format rules outside these modules.

Initial scope:

- **Classic Constructed (CC)**: the flagship constructed format
- **Blitz**: short-game format, Young heroes only, restricted equipment loadout
- **Commoner**: Common-rarity-only, Young heroes only

Limited formats (Draft, Sealed) are out of scope for the deck builder and are not modeled. The exact deck size, equipment limits, and legality cutoffs for each format live in the format module, not in this file. When in doubt, the official rules document is the source of truth.

## Card data shape

Cards come from `the-fab-cube/flesh-and-blood-cards`. The fields the engine cares about most:

- `unique_id`, `name`, `pitch`, `cost`, `power`, `defense`
- `types`, `class`, `talents`
- `keywords`
- `printings` (set, edition, rarity, art variant)
- `legality_by_format` (derived during sync)

Card image URLs are constructed at request time from the card's printing, not stored on the card row.

## Banned and restricted lists

Each format has a banned list and possibly a restricted list. These come from LSS announcements, not from the card data repo. They live in `domain/format/<format>/restrictions.rs` as static data with a "last updated" date constant.

When LSS announces a B&R update:

1. Update the constants
2. Bump the date
3. Add or update tests covering the newly banned or restricted cards
4. Ship

## Validation engine rules of engagement

- The engine is pure. Given a deck, a format, and a date, it returns either Legal or a list of violations. No IO, no clock reads, no env access. The date is an input.
- Violations are structured: a machine-readable code, a human message, and optional details. Never free-form strings only.
- The same violation type for the same input always produces the same code. Codes are stable enough for clients to switch on.
- Adding a format means a new module under `domain/format/`, registration in the format registry, and a complete test suite covering the rules of that format.
- Removing a format is rare. Even in pre-launch, run a deprecation cycle since clients may have decks saved in that format.

## Format module shape

Each format module exposes:

```rust
pub fn validate(deck: &Deck, date: NaiveDate) -> Result<(), Vec<Violation>>;
```

Internally the format breaks validation into a series of focused checks (deck size, hero legality, class restrictions, talent restrictions, banned cards, equipment loadout, and so on). Each check is its own function and its own test. Do not stuff multiple rules into a single function.

## What the deck builder does not do

- Match playing or simulation
- Combat chain resolution
- Goldfishing or playtest mode
- Tournament or event management

These are explicitly not in scope for this service. If a feature request implies any of them, push back and confirm before building.
