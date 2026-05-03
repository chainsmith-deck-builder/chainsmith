//! Hero-driven deck construction rules.
//!
//! Some heroes carry text on their hero card that changes how the standard
//! validation rules apply. Examples:
//!
//! - **Brevant, Civic Protector**: "You may have any number of Chivalry in
//!   your deck." Relaxes the per-card copy limit for one named card.
//! - **Emperor, Dracai of Aesir**: "You may only have red cards in your
//!   deck." Adds a pool-wide color restriction.
//! - **Brutus, Summa Rudis**: "You may have cards with **clash** of any
//!   class or talent in your deck." Relaxes class/talent matching for cards
//!   carrying the `Clash` subtype.
//! - **Shiyana, Diamond Gemini**: "You may have **specialization** cards of
//!   any hero in your deck." Disables the specialization-moniker check.
//!
//! Each rule is a self-contained struct implementing [`SpecialRule`]. The
//! validator calls [`all`] after running its standard checks; each rule that
//! [`SpecialRule::applies`] to the deck gets a chance to mutate the
//! violations vector — adding new violations or removing relaxed ones.
//!
//! Adding a new rule: write a new struct + impl, append it to [`all`].
//! Standard checks do not need to know about it.

use std::collections::HashSet;

use crate::domain::catalog::Catalog;
use crate::domain::deck::Deck;
use crate::domain::format::hero_moniker;
use crate::domain::ids::CardId;
use crate::domain::violation::{Violation, ViolationCode, ViolationDetails};

pub trait SpecialRule {
    /// Stable name for logging/debugging.
    fn name(&self) -> &'static str;

    /// Does this rule fire for the given deck? Most rules match by hero
    /// moniker.
    fn applies(&self, deck: &Deck, catalog: &Catalog) -> bool;

    /// Adjust the violations vector. Rules may add violations or remove
    /// existing ones.
    fn apply(&self, deck: &Deck, catalog: &Catalog, violations: &mut Vec<Violation>);
}

/// All registered special rules. Validator iterates over this list after
/// standard checks. Order is not significant for the currently-printed rule
/// set — no two rules apply to the same hero.
pub fn all() -> Vec<Box<dyn SpecialRule>> {
    vec![
        Box::new(BrevantChivalryOverride),
        Box::new(EmperorRedOnly),
        Box::new(BrutusClashRelaxation),
        Box::new(ShiyanaSpecializationRelaxation),
    ]
}

// ---- shared helpers ----

fn hero_moniker_is(deck: &Deck, catalog: &Catalog, expected: &str) -> bool {
    catalog
        .resolve(&deck.hero)
        .and_then(|(_, c)| hero_moniker(c))
        .map(|m| m.as_str() == expected)
        .unwrap_or(false)
}

fn violation_card_id(v: &Violation) -> Option<&CardId> {
    match &v.details {
        Some(ViolationDetails::Card { card_id, .. })
        | Some(ViolationDetails::Quantity { card_id, .. }) => Some(card_id),
        _ => None,
    }
}

// ---- rules ----

pub struct BrevantChivalryOverride;

impl SpecialRule for BrevantChivalryOverride {
    fn name(&self) -> &'static str {
        "Brevant: any number of Chivalry"
    }

    fn applies(&self, deck: &Deck, catalog: &Catalog) -> bool {
        hero_moniker_is(deck, catalog, "Brevant")
    }

    fn apply(&self, _deck: &Deck, catalog: &Catalog, violations: &mut Vec<Violation>) {
        violations.retain(|v| {
            if v.code != ViolationCode::CopyLimitExceeded {
                return true;
            }
            let Some(card_id) = violation_card_id(v) else {
                return true;
            };
            let Some(card) = catalog.card(card_id) else {
                return true;
            };
            card.name != "Chivalry"
        });
    }
}

pub struct EmperorRedOnly;

impl SpecialRule for EmperorRedOnly {
    fn name(&self) -> &'static str {
        "Emperor: only red cards"
    }

    fn applies(&self, deck: &Deck, catalog: &Catalog) -> bool {
        hero_moniker_is(deck, catalog, "Emperor")
    }

    fn apply(&self, deck: &Deck, catalog: &Catalog, violations: &mut Vec<Violation>) {
        let mut already_reported: HashSet<CardId> = HashSet::new();
        for entry in &deck.pool {
            let Some((_printing, card)) = catalog.resolve(&entry.printing) else {
                continue;
            };
            if !already_reported.insert(card.id.clone()) {
                continue;
            }
            // Cards without a pitch value (equipment, weapons) carry no color
            // and are exempt from the color restriction.
            let Some(pitch) = card.pitch else {
                continue;
            };
            if pitch != 1 {
                violations.push(Violation {
                    code: ViolationCode::CardNotEligibleForFormat,
                    message: format!(
                        "'{}' is not a red card (Emperor allows only red cards)",
                        card.name
                    ),
                    details: Some(ViolationDetails::Card {
                        card_id: card.id.clone(),
                        name: card.name.clone(),
                    }),
                });
            }
        }
    }
}

pub struct BrutusClashRelaxation;

impl SpecialRule for BrutusClashRelaxation {
    fn name(&self) -> &'static str {
        "Brutus: any class/talent for clash cards"
    }

    fn applies(&self, deck: &Deck, catalog: &Catalog) -> bool {
        hero_moniker_is(deck, catalog, "Brutus")
    }

    fn apply(&self, _deck: &Deck, catalog: &Catalog, violations: &mut Vec<Violation>) {
        violations.retain(|v| {
            if v.code != ViolationCode::SupertypeMismatch {
                return true;
            }
            let Some(card_id) = violation_card_id(v) else {
                return true;
            };
            let Some(card) = catalog.card(card_id) else {
                return true;
            };
            // Suppress the supertype mismatch only for cards carrying the
            // Clash subtype. The match is case-insensitive in case upstream
            // data ever shifts casing.
            !card
                .subtypes
                .iter()
                .any(|s| s.eq_ignore_ascii_case("clash"))
        });
    }
}

pub struct ShiyanaSpecializationRelaxation;

impl SpecialRule for ShiyanaSpecializationRelaxation {
    fn name(&self) -> &'static str {
        "Shiyana: specialization of any hero"
    }

    fn applies(&self, deck: &Deck, catalog: &Catalog) -> bool {
        hero_moniker_is(deck, catalog, "Shiyana")
    }

    fn apply(&self, _deck: &Deck, _catalog: &Catalog, violations: &mut Vec<Violation>) {
        violations.retain(|v| v.code != ViolationCode::SpecializationMismatch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::card::Class;
    use crate::domain::format::classic_constructed::ClassicConstructed;
    use crate::domain::format::{validate, FormatId};
    use crate::domain::test_support::*;

    fn cc() -> ClassicConstructed {
        ClassicConstructed::empty()
    }

    fn date() -> chrono::NaiveDate {
        release_on(2026, 5, 2)
    }

    // -------- BrevantChivalryOverride --------

    #[test]
    fn it_allows_brevant_to_run_four_chivalry() {
        let hero = make_adult_hero(
            "h",
            "Brevant, Civic Protector",
            vec![Class::Guardian],
            vec![],
        );
        let chivalry = make_action("c", "Chivalry", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, chivalry],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 4)],
            vec![],
        );
        let violations = validate(&deck, &cc(), date(), &catalog)
            .err()
            .unwrap_or_default();
        assert!(!violations
            .iter()
            .any(|v| v.code == ViolationCode::CopyLimitExceeded));
    }

    #[test]
    fn it_still_enforces_copy_limit_for_non_chivalry_when_hero_is_brevant() {
        let hero = make_adult_hero(
            "h",
            "Brevant, Civic Protector",
            vec![Class::Guardian],
            vec![],
        );
        let other = make_action("c", "Sledge", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, other],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 4)],
            vec![],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::CopyLimitExceeded));
    }

    #[test]
    fn it_does_not_relax_chivalry_copy_limit_for_non_brevant_heroes() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let chivalry = make_action("c", "Chivalry", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, chivalry],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 4)],
            vec![],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::CopyLimitExceeded));
    }

    // -------- EmperorRedOnly --------

    #[test]
    fn it_allows_emperor_with_only_red_cards() {
        let hero = make_adult_hero("h", "Emperor, Dracai of Aesir", vec![Class::Wizard], vec![]);
        let red = make_action("c", "Burning Embers", 1, vec![Class::Wizard], vec![]);
        let catalog = catalog_with(
            vec![hero, red],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 1)],
            vec![],
        );
        assert!(validate(&deck, &cc(), date(), &catalog).is_ok());
    }

    #[test]
    fn it_rejects_yellow_card_for_emperor() {
        let hero = make_adult_hero("h", "Emperor, Dracai of Aesir", vec![Class::Wizard], vec![]);
        // pitch=2 is Yellow.
        let yellow = make_action("c", "Lesson in Lava", 2, vec![Class::Wizard], vec![]);
        let catalog = catalog_with(
            vec![hero, yellow],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 1)],
            vec![],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::CardNotEligibleForFormat));
    }

    #[test]
    fn it_exempts_pitchless_equipment_for_emperor() {
        // Equipment has no pitch and is unrestricted by the color rule.
        let hero = make_adult_hero("h", "Emperor, Dracai of Aesir", vec![Class::Wizard], vec![]);
        let helm = make_equipment("e", "Crown of Dominion", "Head");
        let catalog = catalog_with(
            vec![hero, helm],
            vec![make_printing("hero_p", "h"), make_printing("ep", "e")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("ep", 1)],
            vec![],
        );
        assert!(validate(&deck, &cc(), date(), &catalog).is_ok());
    }

    #[test]
    fn it_does_not_apply_color_restriction_when_hero_is_not_emperor() {
        let hero = make_adult_hero("h", "Wiz", vec![Class::Wizard], vec![]);
        let yellow = make_action("c", "Lesson in Lava", 2, vec![Class::Wizard], vec![]);
        let catalog = catalog_with(
            vec![hero, yellow],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 1)],
            vec![],
        );
        assert!(validate(&deck, &cc(), date(), &catalog).is_ok());
    }

    // -------- BrutusClashRelaxation --------

    #[test]
    fn it_allows_brutus_to_include_off_class_clash_cards() {
        let hero = make_adult_hero("h", "Brutus, Summa Rudis", vec![Class::Warrior], vec![]);
        // Off-class (Wizard) clash card. Without Brutus this would be a
        // SupertypeMismatch.
        let mut clash_card = make_action("c", "Clash Spell", 1, vec![Class::Wizard], vec![]);
        clash_card.subtypes = vec!["Clash".into()];
        let catalog = catalog_with(
            vec![hero, clash_card],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 1)],
            vec![],
        );
        let violations = validate(&deck, &cc(), date(), &catalog)
            .err()
            .unwrap_or_default();
        assert!(!violations
            .iter()
            .any(|v| v.code == ViolationCode::SupertypeMismatch));
    }

    #[test]
    fn it_still_rejects_off_class_non_clash_cards_for_brutus() {
        let hero = make_adult_hero("h", "Brutus, Summa Rudis", vec![Class::Warrior], vec![]);
        let off = make_action("c", "Wizard Bolt", 1, vec![Class::Wizard], vec![]);
        let catalog = catalog_with(
            vec![hero, off],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 1)],
            vec![],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::SupertypeMismatch));
    }

    // -------- ShiyanaSpecializationRelaxation --------

    #[test]
    fn it_allows_shiyana_to_include_any_hero_specialization_card() {
        // Shiyana is, per FaB, an Illusionist by canonical class, but for the
        // engine the class doesn't matter for this test — just that the
        // moniker is "Shiyana" and the card carries a specialization for a
        // different hero.
        let hero = make_adult_hero(
            "h",
            "Shiyana, Diamond Gemini",
            vec![Class::Illusionist],
            vec![],
        );
        // Specialization card for Bravo, in the Guardian class. Without
        // Shiyana's relaxation this would emit SpecializationMismatch (and
        // separately SupertypeMismatch — that one is unrelated to this rule).
        let specialized =
            make_specialized_action("c", "Bravo's Special", vec![Class::Guardian], vec!["Bravo"]);
        let catalog = catalog_with(
            vec![hero, specialized],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 1)],
            vec![],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(!err
            .iter()
            .any(|v| v.code == ViolationCode::SpecializationMismatch));
    }

    #[test]
    fn it_does_not_relax_specialization_for_non_shiyana_heroes() {
        let hero = make_adult_hero("h", "Katsu, the Wanderer", vec![Class::Ninja], vec![]);
        let specialized =
            make_specialized_action("c", "Bravo's Special", vec![Class::Guardian], vec!["Bravo"]);
        let catalog = catalog_with(
            vec![hero, specialized],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 1)],
            vec![],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::SpecializationMismatch));
    }

    // -------- registry sanity --------

    #[test]
    fn it_registers_all_four_rules() {
        let names: Vec<_> = all().iter().map(|r| r.name()).collect();
        assert_eq!(names.len(), 4);
        assert!(names.iter().any(|n| n.starts_with("Brevant")));
        assert!(names.iter().any(|n| n.starts_with("Emperor")));
        assert!(names.iter().any(|n| n.starts_with("Brutus")));
        assert!(names.iter().any(|n| n.starts_with("Shiyana")));
    }

    #[test]
    fn it_drops_a_violation_when_a_rule_uses_pinpoint_resolution() {
        // Combination scenario: Brevant + 4 Chivalry + 4 of another card.
        // Only the Chivalry copy-limit violation should be suppressed; the
        // other should remain.
        let hero = make_adult_hero(
            "h",
            "Brevant, Civic Protector",
            vec![Class::Guardian],
            vec![],
        );
        let chivalry = make_action("c1", "Chivalry", 1, vec![Class::Guardian], vec![]);
        let other = make_action("c2", "Sledge", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, chivalry, other],
            vec![
                make_printing("hero_p", "h"),
                make_printing("p1", "c1"),
                make_printing("p2", "c2"),
            ],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 4), pool_entry("p2", 4)],
            vec![],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        let copy_violations: Vec<_> = err
            .iter()
            .filter(|v| v.code == ViolationCode::CopyLimitExceeded)
            .collect();
        assert_eq!(copy_violations.len(), 1, "violations: {err:?}");
        // The remaining violation should be for "Sledge", not Chivalry.
        match &copy_violations[0].details {
            Some(ViolationDetails::Quantity { card_id, .. }) => {
                assert_eq!(card_id.as_str(), "c2");
            }
            other => panic!("unexpected details: {other:?}"),
        }
    }
}
