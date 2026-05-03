//! Format trait, universal validation checks, and the `validate` entry point.
//!
//! The shared engine here runs every check that is invariant across formats —
//! supertype matching (CR 1.1.3), specializations, copy limits, pool size,
//! loadout coherence — and delegates format-specific predicates (eligibility,
//! banned/restricted lists) through the `Format` trait. Most format
//! implementations leave `extra_checks` empty.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::card::{Card, CardType, Class, EssenceGrant, Printing, Talent, WeaponGrip};
use crate::domain::catalog::Catalog;
use crate::domain::deck::{Deck, Loadout};
use crate::domain::ids::{CardId, HeroMoniker, PrintingId};
use crate::domain::violation::{Violation, ViolationCode, ViolationDetails};

pub mod classic_constructed;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FormatId {
    ClassicConstructed,
    Blitz,
    Commoner,
    SilverAge,
    LivingLegend,
    UltimatePitFight,
}

#[derive(Debug, Clone, Copy)]
pub struct FormatRules {
    pub min_deck_size: u16,
    pub max_deck_size: Option<u16>,
    pub card_pool_size: Option<u16>,
    pub card_copy_limit: CopyLimit,
    pub equipment_inventory_limit: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyLimit {
    Exact(u8),
    Unlimited,
}

pub trait Format {
    fn id(&self) -> FormatId;
    fn rules(&self) -> &FormatRules;

    /// Hero-level eligibility (Adult/Young, LL retirement, etc).
    fn hero_eligible(&self, hero: &Card, date: NaiveDate) -> Result<(), Violation>;

    /// Card-level eligibility for the format itself (rarity, set, type).
    /// Banned and restricted are separate, layered concerns.
    fn card_eligible(
        &self,
        card: &Card,
        printing: &Printing,
        date: NaiveDate,
    ) -> Result<(), Violation>;

    fn banned_at(&self, card_id: &CardId, date: NaiveDate) -> bool;
    fn restricted_at(&self, card_id: &CardId, date: NaiveDate) -> bool;

    fn extra_checks(&self, _deck: &Deck, _date: NaiveDate, _catalog: &Catalog) -> Vec<Violation> {
        Vec::new()
    }
}

/// Top-level entry point. Runs every check and aggregates violations.
pub fn validate(
    deck: &Deck,
    format: &dyn Format,
    date: NaiveDate,
    catalog: &Catalog,
) -> Result<(), Vec<Violation>> {
    let mut violations = Vec::new();

    let hero_card = resolve_hero(deck, catalog, &mut violations);
    if let Some(hero) = hero_card {
        check_hero_is_hero_type(hero, &mut violations);
        if let Err(v) = format.hero_eligible(hero, date) {
            violations.push(v);
        }
    }

    check_pool_size(deck, format.rules(), &mut violations);
    check_pool_printings_resolve(deck, catalog, &mut violations);
    check_pool_eligibility(deck, format, date, catalog, &mut violations);
    check_pool_banned(deck, format, date, catalog, &mut violations);
    check_copy_limit(deck, format.rules(), catalog, &mut violations);
    check_restricted_limit(deck, format, date, catalog, &mut violations);

    if let Some(hero) = hero_card {
        check_supertype_subset(deck, hero, catalog, &mut violations);
        check_specialization(deck, hero, catalog, &mut violations);
    }

    check_loadouts(deck, format.rules(), hero_card, catalog, &mut violations);

    violations.extend(format.extra_checks(deck, date, catalog));

    // Hero-driven special rules run last so they observe every prior
    // violation and may add new ones or relax existing ones (see
    // `crate::domain::special_rules`).
    for rule in crate::domain::special_rules::all() {
        if rule.applies(deck, catalog) {
            rule.apply(deck, catalog, &mut violations);
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

fn resolve_hero<'c>(
    deck: &Deck,
    catalog: &'c Catalog,
    violations: &mut Vec<Violation>,
) -> Option<&'c Card> {
    match catalog.resolve(&deck.hero) {
        Some((_, card)) => Some(card),
        None => {
            violations.push(violation(
                ViolationCode::HeroPrintingNotFoundInCatalog,
                format!("hero printing {} not found in catalog", deck.hero),
                Some(ViolationDetails::Printing {
                    printing_id: deck.hero.clone(),
                }),
            ));
            None
        }
    }
}

fn check_hero_is_hero_type(hero: &Card, violations: &mut Vec<Violation>) {
    if !hero.types.contains(&CardType::Hero) {
        violations.push(violation(
            ViolationCode::HeroMissingHeroType,
            format!("card '{}' is not a hero", hero.name),
            Some(ViolationDetails::Card {
                card_id: hero.id.clone(),
                name: hero.name.clone(),
            }),
        ));
    }
}

fn check_pool_size(deck: &Deck, rules: &FormatRules, violations: &mut Vec<Violation>) {
    let Some(max) = rules.card_pool_size else {
        return;
    };
    let total = pool_total(deck);
    if total > u32::from(max) {
        violations.push(violation(
            ViolationCode::PoolSizeAboveMax,
            format!("pool has {total} cards; max is {max}"),
            Some(ViolationDetails::PoolSize {
                found: total,
                max: u32::from(max),
            }),
        ));
    }
}

fn check_pool_printings_resolve(deck: &Deck, catalog: &Catalog, violations: &mut Vec<Violation>) {
    for entry in &deck.pool {
        match catalog.printing(&entry.printing) {
            None => violations.push(violation(
                ViolationCode::PrintingNotFoundInCatalog,
                format!("printing {} not in catalog", entry.printing),
                Some(ViolationDetails::Printing {
                    printing_id: entry.printing.clone(),
                }),
            )),
            Some(printing) => {
                if catalog.card(&printing.card_id).is_none() {
                    violations.push(violation(
                        ViolationCode::CardNotFoundInCatalog,
                        format!(
                            "card {} (referenced by printing {}) not in catalog",
                            printing.card_id, entry.printing
                        ),
                        Some(ViolationDetails::Card {
                            card_id: printing.card_id.clone(),
                            name: String::new(),
                        }),
                    ));
                }
            }
        }
    }
}

fn check_pool_eligibility(
    deck: &Deck,
    format: &dyn Format,
    date: NaiveDate,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    let mut already_reported: HashSet<CardId> = HashSet::new();
    for entry in &deck.pool {
        let Some((printing, card)) = catalog.resolve(&entry.printing) else {
            continue;
        };
        if !already_reported.insert(card.id.clone()) {
            continue;
        }
        if let Err(v) = format.card_eligible(card, printing, date) {
            violations.push(v);
        }
    }
}

fn check_pool_banned(
    deck: &Deck,
    format: &dyn Format,
    date: NaiveDate,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    let mut already_reported: HashSet<CardId> = HashSet::new();
    for entry in &deck.pool {
        let Some((_, card)) = catalog.resolve(&entry.printing) else {
            continue;
        };
        if !already_reported.insert(card.id.clone()) {
            continue;
        }
        if format.banned_at(&card.id, date) {
            violations.push(violation(
                ViolationCode::CardBanned,
                format!("'{}' is banned in this format", card.name),
                Some(ViolationDetails::Card {
                    card_id: card.id.clone(),
                    name: card.name.clone(),
                }),
            ));
        }
    }
}

fn check_copy_limit(
    deck: &Deck,
    rules: &FormatRules,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    let CopyLimit::Exact(limit) = rules.card_copy_limit else {
        return;
    };
    for (card_id, count, name) in counts_by_card(deck, catalog) {
        if count > u32::from(limit) {
            violations.push(violation(
                ViolationCode::CopyLimitExceeded,
                format!("'{name}' appears {count} times; limit is {limit}"),
                Some(ViolationDetails::Quantity {
                    card_id,
                    found: count,
                    allowed: u32::from(limit),
                }),
            ));
        }
    }
}

fn check_restricted_limit(
    deck: &Deck,
    format: &dyn Format,
    date: NaiveDate,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    for (card_id, count, name) in counts_by_card(deck, catalog) {
        if format.restricted_at(&card_id, date) && count > 1 {
            violations.push(violation(
                ViolationCode::RestrictedCopyLimitExceeded,
                format!("'{name}' is restricted; max 1 copy, found {count}"),
                Some(ViolationDetails::Quantity {
                    card_id,
                    found: count,
                    allowed: 1,
                }),
            ));
        }
    }
}

fn check_supertype_subset(
    deck: &Deck,
    hero: &Card,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    let mut already_reported: HashSet<CardId> = HashSet::new();
    for entry in &deck.pool {
        let Some((_, card)) = catalog.resolve(&entry.printing) else {
            continue;
        };
        if !already_reported.insert(card.id.clone()) {
            continue;
        }
        // The hero's own card is in the catalog, but it's not in the pool;
        // even if it were, its supertypes trivially match itself, so this is
        // safe regardless.
        if !supertypes_match_hero(card, hero) {
            violations.push(violation(
                ViolationCode::SupertypeMismatch,
                format!(
                    "'{}' cannot be played by {} (class/talent mismatch)",
                    card.name, hero.name
                ),
                Some(ViolationDetails::Card {
                    card_id: card.id.clone(),
                    name: card.name.clone(),
                }),
            ));
        }
    }
}

fn check_specialization(
    deck: &Deck,
    hero: &Card,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    let moniker = hero_moniker(hero);
    let mut already_reported: HashSet<CardId> = HashSet::new();
    for entry in &deck.pool {
        let Some((_, card)) = catalog.resolve(&entry.printing) else {
            continue;
        };
        if !already_reported.insert(card.id.clone()) {
            continue;
        }
        if card.specializations.is_empty() {
            continue;
        }
        let allowed = match &moniker {
            Some(m) => card.specializations.iter().any(|spec| spec == m),
            None => false,
        };
        if !allowed {
            violations.push(violation(
                ViolationCode::SpecializationMismatch,
                format!(
                    "'{}' is hero-specialized; current hero {} cannot include it",
                    card.name, hero.name
                ),
                Some(ViolationDetails::Card {
                    card_id: card.id.clone(),
                    name: card.name.clone(),
                }),
            ));
        }
    }
}

fn check_loadouts(
    deck: &Deck,
    rules: &FormatRules,
    hero: Option<&Card>,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    let pool_by_printing: HashMap<&PrintingId, u8> = deck
        .pool
        .iter()
        .map(|e| (&e.printing, e.quantity))
        .collect();

    for loadout in &deck.loadouts {
        check_loadout_entries_in_pool(loadout, &pool_by_printing, violations);
        check_loadout_deck_size(loadout, rules, violations);
        check_loadout_equipment_in_pool(loadout, &pool_by_printing, violations);
        check_loadout_armor_slots(loadout, catalog, violations);
        check_loadout_main_hand_is_weapon(loadout, catalog, violations);
        check_loadout_off_hand_is_eligible(loadout, catalog, violations);
        check_loadout_two_handed_companion_rule(loadout, catalog, violations);
        if let Some(hero) = hero {
            check_loadout_weapon_zone_count(loadout, hero, catalog, violations);
        }
    }
}

fn check_loadout_entries_in_pool(
    loadout: &Loadout,
    pool_by_printing: &HashMap<&PrintingId, u8>,
    violations: &mut Vec<Violation>,
) {
    for entry in &loadout.deck_cards {
        match pool_by_printing.get(&entry.printing).copied() {
            None => violations.push(violation(
                ViolationCode::LoadoutPrintingNotInPool,
                format!(
                    "loadout '{}' references printing {} which is not in the pool",
                    loadout.name, entry.printing
                ),
                Some(ViolationDetails::LoadoutCard {
                    loadout: loadout.name.clone(),
                    printing_id: entry.printing.clone(),
                }),
            )),
            Some(pool_qty) if entry.quantity > pool_qty => {
                violations.push(violation(
                    ViolationCode::LoadoutQuantityExceedsPool,
                    format!(
                        "loadout '{}' uses {} of {}; pool only has {}",
                        loadout.name, entry.quantity, entry.printing, pool_qty
                    ),
                    Some(ViolationDetails::LoadoutCard {
                        loadout: loadout.name.clone(),
                        printing_id: entry.printing.clone(),
                    }),
                ));
            }
            _ => {}
        }
    }
}

fn check_loadout_deck_size(
    loadout: &Loadout,
    rules: &FormatRules,
    violations: &mut Vec<Violation>,
) {
    let total: u32 = loadout
        .deck_cards
        .iter()
        .map(|e| u32::from(e.quantity))
        .sum();
    let min = u32::from(rules.min_deck_size);
    let max = rules.max_deck_size.map(u32::from);

    if total < min {
        violations.push(violation(
            ViolationCode::DeckSizeBelowMin,
            format!(
                "loadout '{}' has {} cards; min is {}",
                loadout.name, total, rules.min_deck_size
            ),
            Some(ViolationDetails::DeckSize {
                found: total,
                min,
                max,
            }),
        ));
    }
    if let Some(max_v) = max {
        if total > max_v {
            violations.push(violation(
                ViolationCode::DeckSizeAboveMax,
                format!(
                    "loadout '{}' has {} cards; max is {}",
                    loadout.name, total, max_v
                ),
                Some(ViolationDetails::DeckSize {
                    found: total,
                    min,
                    max: Some(max_v),
                }),
            ));
        }
    }
}

fn check_loadout_equipment_in_pool(
    loadout: &Loadout,
    pool_by_printing: &HashMap<&PrintingId, u8>,
    violations: &mut Vec<Violation>,
) {
    for piece in equipped_pieces_iter(loadout) {
        if !pool_by_printing.contains_key(piece) {
            violations.push(violation(
                ViolationCode::EquipmentNotInPool,
                format!(
                    "loadout '{}' equips {}; printing not in pool",
                    loadout.name, piece
                ),
                Some(ViolationDetails::Printing {
                    printing_id: piece.clone(),
                }),
            ));
        }
    }
}

fn check_loadout_armor_slots(
    loadout: &Loadout,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    let slots: [(&str, Option<&PrintingId>); 4] = [
        ("Head", loadout.equipment.head.as_ref()),
        ("Chest", loadout.equipment.chest.as_ref()),
        ("Arms", loadout.equipment.arms.as_ref()),
        ("Legs", loadout.equipment.legs.as_ref()),
    ];
    for (slot_name, opt_id) in slots {
        let Some(printing_id) = opt_id else { continue };
        let Some((_, card)) = catalog.resolve(printing_id) else {
            continue;
        };
        let actual = inferred_slot(card);
        if actual.as_deref() != Some(slot_name) {
            violations.push(violation(
                ViolationCode::EquipmentSlotWrong,
                format!(
                    "loadout '{}' equips '{}' in {} slot; card slot is {}",
                    loadout.name,
                    card.name,
                    slot_name,
                    actual.as_deref().unwrap_or("<unknown>")
                ),
                Some(ViolationDetails::EquipmentSlot {
                    printing_id: printing_id.clone(),
                    expected_slot: slot_name.to_string(),
                    actual_slot: actual,
                }),
            ));
        }
    }
}

fn check_loadout_main_hand_is_weapon(
    loadout: &Loadout,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    let Some(printing_id) = loadout.equipment.main_hand.as_ref() else {
        return;
    };
    let Some((_, card)) = catalog.resolve(printing_id) else {
        return;
    };
    if !card.types.contains(&CardType::Weapon) {
        violations.push(violation(
            ViolationCode::EquipmentSlotWrong,
            format!(
                "loadout '{}' equips '{}' in main hand; card is not a weapon",
                loadout.name, card.name
            ),
            Some(ViolationDetails::EquipmentSlot {
                printing_id: printing_id.clone(),
                expected_slot: "Main Hand".to_string(),
                actual_slot: inferred_slot(card),
            }),
        ));
    }
}

fn check_loadout_off_hand_is_eligible(
    loadout: &Loadout,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    let Some(printing_id) = loadout.equipment.off_hand.as_ref() else {
        return;
    };
    let Some((_, card)) = catalog.resolve(printing_id) else {
        return;
    };

    // Off-hand may hold a one-handed weapon (a 2H weapon would occupy the
    // off-hand by virtue of its grip, never as an off-hand entry on its own)
    // or any equipment carrying the Off-Hand subtype.
    let is_one_handed_weapon = card.types.contains(&CardType::Weapon)
        && matches!(card.weapon_grip, Some(WeaponGrip::OneHanded));
    let is_off_hand_equipment =
        card.types.contains(&CardType::Equipment) && card.subtypes.iter().any(|s| s == "Off-Hand");

    if !is_one_handed_weapon && !is_off_hand_equipment {
        violations.push(violation(
            ViolationCode::EquipmentSlotWrong,
            format!(
                "loadout '{}' equips '{}' in off-hand; must be a one-handed weapon or off-hand equipment",
                loadout.name, card.name
            ),
            Some(ViolationDetails::EquipmentSlot {
                printing_id: printing_id.clone(),
                expected_slot: "Off-Hand".to_string(),
                actual_slot: inferred_slot(card),
            }),
        ));
    }
}

fn check_loadout_weapon_zone_count(
    loadout: &Loadout,
    hero: &Card,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    // Most heroes have 2 weapon zones; Kayo variants override to 1, Bolfar to
    // 0. Non-weapon entries in the off-hand slot don't count.
    let zones = hero
        .hero
        .as_ref()
        .map(|f| u32::from(f.weapon_zone_count))
        .unwrap_or(2);

    let weapon_count: u32 = [
        loadout.equipment.main_hand.as_ref(),
        loadout.equipment.off_hand.as_ref(),
    ]
    .into_iter()
    .flatten()
    .filter_map(|p| catalog.resolve(p))
    .filter(|(_, card)| card.types.contains(&CardType::Weapon))
    .count() as u32;

    if weapon_count > zones {
        violations.push(violation(
            ViolationCode::WeaponConfigInvalid,
            format!(
                "loadout '{}' equips {} weapon(s); hero {} has {} weapon zone(s)",
                loadout.name, weapon_count, hero.name, zones
            ),
            None,
        ));
    }
}

fn check_loadout_two_handed_companion_rule(
    loadout: &Loadout,
    catalog: &Catalog,
    violations: &mut Vec<Violation>,
) {
    let Some(main_hand_id) = loadout.equipment.main_hand.as_ref() else {
        return;
    };
    let Some((_, main_card)) = catalog.resolve(main_hand_id) else {
        return;
    };
    if !matches!(main_card.weapon_grip, Some(WeaponGrip::TwoHanded)) {
        return;
    }

    // Main hand is 2H. Per CR, the off-hand must be empty or a Companion-
    // subtyped off-hand equipment piece.
    let Some(off_hand_id) = loadout.equipment.off_hand.as_ref() else {
        return;
    };
    let Some((_, off_card)) = catalog.resolve(off_hand_id) else {
        return;
    };

    let is_companion = off_card.types.contains(&CardType::Equipment)
        && off_card.subtypes.iter().any(|s| s == "Companion");

    if !is_companion {
        violations.push(violation(
            ViolationCode::WeaponConfigInvalid,
            format!(
                "loadout '{}' equips '{}' alongside two-handed weapon '{}'; only Companion-subtyped off-hand equipment is permitted with a 2H weapon",
                loadout.name, off_card.name, main_card.name
            ),
            None,
        ));
    }
}

// ---- pure helpers (also exposed for direct testing) ----

/// CR 1.1.3 — a card's supertypes (classes and talents) must be a subset of
/// the hero's effective supertypes (printed plus any "Essence of X" grants).
/// Generic cards (no supertypes) are legal for any hero.
///
/// Not yet implemented: CR 1.1.3b ("hybrid" cards — a card carrying two
/// supertype sets, legal if *either* set is a subset of the hero's
/// supertypes). The current `Card` model has a single class/talent vector,
/// not a `Vec<SupertypeSet>`, so hybrids cannot be expressed. No hybrid card
/// has actually printed in FaB as of 2026-05; revisit when one does. The
/// data shape change would propagate through sync, the upstream parser, and
/// every fixture, so it's not a small refactor.
pub fn supertypes_match_hero(card: &Card, hero: &Card) -> bool {
    if card.classes.is_empty() && card.talents.is_empty() {
        return true;
    }
    let (hero_classes, hero_talents) = effective_hero_supertypes(hero);
    let class_ok = card.classes.iter().all(|c| hero_classes.contains(c));
    let talent_ok = card.talents.iter().all(|t| hero_talents.contains(t));
    class_ok && talent_ok
}

fn effective_hero_supertypes(hero: &Card) -> (Vec<Class>, Vec<Talent>) {
    let mut classes = hero.classes.clone();
    let mut talents = hero.talents.clone();
    if let Some(facts) = &hero.hero {
        for grant in &facts.essence_grants {
            match grant {
                EssenceGrant::Class(c) => {
                    if !classes.contains(c) {
                        classes.push(*c);
                    }
                }
                EssenceGrant::Talent(t) => {
                    if !talents.contains(t) {
                        talents.push(*t);
                    }
                }
            }
        }
    }
    (classes, talents)
}

/// Hero monikers in FaB are the comma-separated prefix of the hero's name.
/// "Bravo, Star of the Show" -> "Bravo". Names without commas return the
/// trimmed name itself.
pub fn hero_moniker(hero: &Card) -> Option<HeroMoniker> {
    let trimmed = hero.name.split(',').next()?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(HeroMoniker::new(trimmed))
    }
}

fn inferred_slot(card: &Card) -> Option<String> {
    // FaB encodes equipment slot as a subtype on the equipment card.
    for sub in &card.subtypes {
        if matches!(
            sub.as_str(),
            "Head" | "Chest" | "Arms" | "Legs" | "Off-Hand"
        ) {
            return Some(sub.clone());
        }
    }
    None
}

fn pool_total(deck: &Deck) -> u32 {
    deck.pool.iter().map(|e| u32::from(e.quantity)).sum()
}

fn counts_by_card(deck: &Deck, catalog: &Catalog) -> Vec<(CardId, u32, String)> {
    let mut by_card: HashMap<CardId, (u32, String)> = HashMap::new();
    for entry in &deck.pool {
        let Some((printing, card)) = catalog.resolve(&entry.printing) else {
            continue;
        };
        let counter = by_card
            .entry(printing.card_id.clone())
            .or_insert_with(|| (0, card.name.clone()));
        counter.0 += u32::from(entry.quantity);
    }
    by_card
        .into_iter()
        .map(|(id, (count, name))| (id, count, name))
        .collect()
}

fn equipped_pieces_iter(loadout: &Loadout) -> impl Iterator<Item = &PrintingId> {
    [
        loadout.equipment.head.as_ref(),
        loadout.equipment.chest.as_ref(),
        loadout.equipment.arms.as_ref(),
        loadout.equipment.legs.as_ref(),
        loadout.equipment.main_hand.as_ref(),
        loadout.equipment.off_hand.as_ref(),
    ]
    .into_iter()
    .flatten()
}

fn violation(code: ViolationCode, message: String, details: Option<ViolationDetails>) -> Violation {
    Violation {
        code,
        message,
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::card::EssenceGrant;
    use crate::domain::format::classic_constructed::ClassicConstructed;
    use crate::domain::test_support::*;

    fn cc() -> ClassicConstructed {
        ClassicConstructed::empty()
    }

    fn date() -> NaiveDate {
        release_on(2026, 5, 2)
    }

    // -------- supertypes_match_hero --------

    #[test]
    fn it_treats_a_generic_card_as_legal_for_any_hero() {
        let hero = make_adult_hero(
            "h",
            "Bravo, Star of the Show",
            vec![Class::Guardian],
            vec![],
        );
        let generic = make_generic_action("g", "Generic Strike", 1);
        assert!(supertypes_match_hero(&generic, &hero));
    }

    #[test]
    fn it_allows_a_class_card_when_hero_has_that_class() {
        let hero = make_adult_hero("h", "Wiz", vec![Class::Wizard], vec![]);
        let card = make_action("c", "Wizard Spell", 1, vec![Class::Wizard], vec![]);
        assert!(supertypes_match_hero(&card, &hero));
    }

    #[test]
    fn it_rejects_a_class_card_when_hero_has_a_different_class() {
        let hero = make_adult_hero("h", "Wiz", vec![Class::Wizard], vec![]);
        let card = make_action("c", "Warrior Strike", 1, vec![Class::Warrior], vec![]);
        assert!(!supertypes_match_hero(&card, &hero));
    }

    #[test]
    fn it_allows_a_talent_card_when_hero_has_that_talent() {
        let hero = make_adult_hero("h", "Boltyn", vec![Class::Warrior], vec![Talent::Light]);
        let card = make_action(
            "c",
            "Light Strike",
            1,
            vec![Class::Warrior],
            vec![Talent::Light],
        );
        assert!(supertypes_match_hero(&card, &hero));
    }

    #[test]
    fn it_rejects_a_talent_card_when_hero_lacks_the_talent() {
        let hero = make_adult_hero("h", "Plain Warrior", vec![Class::Warrior], vec![]);
        let card = make_action(
            "c",
            "Light Strike",
            1,
            vec![Class::Warrior],
            vec![Talent::Light],
        );
        assert!(!supertypes_match_hero(&card, &hero));
    }

    #[test]
    fn it_requires_all_card_classes_match_hero() {
        // Hypothetical card with two classes; hero has only one of them.
        let hero = make_adult_hero("h", "Wiz", vec![Class::Wizard], vec![]);
        let card = make_action(
            "c",
            "Warrior-Wizard Hybrid",
            1,
            vec![Class::Wizard, Class::Warrior],
            vec![],
        );
        assert!(!supertypes_match_hero(&card, &hero));
    }

    #[test]
    fn it_requires_all_card_talents_match_hero() {
        // Hypothetical card with two talents; hero has only one.
        let hero = make_adult_hero("h", "Boltyn", vec![Class::Warrior], vec![Talent::Light]);
        let card = make_action(
            "c",
            "Light Lightning Warrior Strike",
            1,
            vec![Class::Warrior],
            vec![Talent::Light, Talent::Lightning],
        );
        assert!(!supertypes_match_hero(&card, &hero));
    }

    #[test]
    fn it_includes_essence_grants_in_hero_supertypes() {
        // Hero printed as Wizard, Essence of Lightning grants Lightning.
        let hero = with_essence_grants(
            make_adult_hero("h", "Hypothetical", vec![Class::Wizard], vec![]),
            vec![EssenceGrant::Talent(Talent::Lightning)],
        );
        let card = make_action(
            "c",
            "Lightning Wizard",
            1,
            vec![Class::Wizard],
            vec![Talent::Lightning],
        );
        assert!(supertypes_match_hero(&card, &hero));
    }

    // -------- hero_moniker --------

    #[test]
    fn it_takes_the_comma_prefix_as_moniker() {
        let hero = make_adult_hero(
            "h",
            "Bravo, Star of the Show",
            vec![Class::Guardian],
            vec![],
        );
        assert_eq!(hero_moniker(&hero), Some(HeroMoniker::new("Bravo")));
    }

    #[test]
    fn it_returns_full_trimmed_name_when_no_comma() {
        let hero = make_adult_hero("h", "Katsu", vec![Class::Ninja], vec![]);
        assert_eq!(hero_moniker(&hero), Some(HeroMoniker::new("Katsu")));
    }

    #[test]
    fn it_returns_none_for_empty_name() {
        let hero = make_adult_hero("h", "", vec![], vec![]);
        assert!(hero_moniker(&hero).is_none());
    }

    // -------- check_pool_size --------

    #[test]
    fn it_passes_pool_size_when_at_or_under_limit() {
        let rules = FormatRules {
            min_deck_size: 60,
            max_deck_size: None,
            card_pool_size: Some(80),
            card_copy_limit: CopyLimit::Exact(3),
            equipment_inventory_limit: None,
        };
        // Right at the limit: 80 cards in pool.
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 80)],
            vec![],
        );
        let mut violations = Vec::new();
        check_pool_size(&deck, &rules, &mut violations);
        assert!(violations.is_empty(), "violations: {violations:?}");
    }

    #[test]
    fn it_violates_pool_size_when_one_over_limit() {
        let rules = FormatRules {
            min_deck_size: 60,
            max_deck_size: None,
            card_pool_size: Some(80),
            card_copy_limit: CopyLimit::Exact(3),
            equipment_inventory_limit: None,
        };
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 81)],
            vec![],
        );
        let mut violations = Vec::new();
        check_pool_size(&deck, &rules, &mut violations);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].code, ViolationCode::PoolSizeAboveMax);
    }

    #[test]
    fn it_skips_pool_size_when_format_has_no_pool_cap() {
        let rules = FormatRules {
            min_deck_size: 30,
            max_deck_size: None,
            card_pool_size: None, // Limited-style: unbounded
            card_copy_limit: CopyLimit::Unlimited,
            equipment_inventory_limit: None,
        };
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 200)],
            vec![],
        );
        let mut violations = Vec::new();
        check_pool_size(&deck, &rules, &mut violations);
        assert!(violations.is_empty());
    }

    // -------- check_copy_limit --------

    #[test]
    fn it_passes_copy_limit_when_at_max_copies() {
        // 3 copies of a single card, all via one entry.
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let card = make_generic_action("c1", "Card", 1);
        let catalog = catalog_with(
            vec![hero, card],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c1")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 3)],
            vec![],
        );
        let rules = cc().rules().to_owned();
        let mut violations = Vec::new();
        check_copy_limit(&deck, &rules, &catalog, &mut violations);
        assert!(violations.is_empty());
    }

    #[test]
    fn it_violates_copy_limit_when_one_over_max() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let card = make_generic_action("c1", "Card", 1);
        let catalog = catalog_with(
            vec![hero, card],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c1")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 4)],
            vec![],
        );
        let rules = cc().rules().to_owned();
        let mut violations = Vec::new();
        check_copy_limit(&deck, &rules, &catalog, &mut violations);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].code, ViolationCode::CopyLimitExceeded);
    }

    #[test]
    fn it_aggregates_copies_across_printings_of_same_card() {
        // Two printings of the same card — one standard, one cold-foil.
        // Total of 4 should violate.
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let card = make_generic_action("c1", "Crouching Tiger", 1);
        let mut p_alt = make_printing("p1_alt", "c1");
        p_alt.foiling = crate::domain::card::Foiling::ColdFoil;
        let catalog = catalog_with(
            vec![hero, card],
            vec![
                make_printing("hero_p", "h"),
                make_printing("p1", "c1"),
                p_alt,
            ],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 3), pool_entry("p1_alt", 1)],
            vec![],
        );
        let rules = cc().rules().to_owned();
        let mut violations = Vec::new();
        check_copy_limit(&deck, &rules, &catalog, &mut violations);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].code, ViolationCode::CopyLimitExceeded);
    }

    // -------- validate end-to-end --------

    #[test]
    fn it_validates_a_minimal_legal_cc_deck() {
        let hero = make_adult_hero(
            "h",
            "Bravo, Star of the Show",
            vec![Class::Guardian],
            vec![],
        );
        let action = make_action("c1", "Crippling Crush", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, action],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c1")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            // 60 copies via one row would violate the per-card 3 cap; but
            // for the minimal legal scenario we just need the pool to pass
            // each check. Use 3 + 60 padding via the same action would fail
            // copy limit. To keep the legal scenario clean, we'd need many
            // distinct cards. Instead, pick a smaller min_deck_size scenario
            // by building with no loadout, so the deck-size check doesn't
            // fire. Validates pool/eligibility/banned/etc. only.
            vec![pool_entry("p1", 3)],
            vec![],
        );
        let result = validate(&deck, &cc(), date(), &catalog);
        assert!(
            result.is_ok(),
            "expected legal deck; got {:?}",
            result.err()
        );
    }

    #[test]
    fn it_reports_supertype_mismatch_for_off_class_card() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let off_class = make_action("c1", "Wizard Bolt", 1, vec![Class::Wizard], vec![]);
        let catalog = catalog_with(
            vec![hero, off_class],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c1")],
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

    #[test]
    fn it_reports_specialization_mismatch_when_hero_moniker_does_not_match() {
        let hero = make_adult_hero("h", "Katsu, the Wanderer", vec![Class::Ninja], vec![]);
        let bravo_card = make_specialized_action(
            "c1",
            "Bravo Spec Card",
            vec![Class::Guardian],
            vec!["Bravo"],
        );
        let catalog = catalog_with(
            vec![hero, bravo_card],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c1")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 1)],
            vec![],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        // Will also report supertype mismatch (Ninja can't play Guardian);
        // assert specifically that the specialization code is present.
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::SpecializationMismatch));
    }

    #[test]
    fn it_allows_specialized_card_for_matching_hero() {
        let hero = make_adult_hero(
            "h",
            "Bravo, Star of the Show",
            vec![Class::Guardian],
            vec![],
        );
        let bravo_card = make_specialized_action(
            "c1",
            "Bravo Spec Card",
            vec![Class::Guardian],
            vec!["Bravo"],
        );
        let catalog = catalog_with(
            vec![hero, bravo_card],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c1")],
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
    fn it_reports_missing_hero_when_printing_not_in_catalog() {
        let catalog = Catalog::new();
        let deck = build_deck(FormatId::ClassicConstructed, "missing_hero", vec![], vec![]);
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::HeroPrintingNotFoundInCatalog));
    }

    #[test]
    fn it_reports_missing_pool_printing() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let catalog = catalog_with(vec![hero], vec![make_printing("hero_p", "h")]);
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("ghost", 1)],
            vec![],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::PrintingNotFoundInCatalog));
    }

    #[test]
    fn it_reports_loadout_card_not_in_pool() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let action = make_action("c1", "Crippling Crush", 1, vec![Class::Guardian], vec![]);
        let other = make_action("c2", "Sledge", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, action, other],
            vec![
                make_printing("hero_p", "h"),
                make_printing("p1", "c1"),
                make_printing("p2", "c2"),
            ],
        );
        let loadout = make_loadout("Main", vec![loadout_entry("p1", 1), loadout_entry("p2", 1)]);
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            // p2 isn't in the pool but the loadout uses it.
            vec![pool_entry("p1", 3)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::LoadoutPrintingNotInPool));
    }

    #[test]
    fn it_reports_loadout_below_min_deck_size() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let action = make_action("c1", "Crippling Crush", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, action],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c1")],
        );
        let loadout = make_loadout("Skinny", vec![loadout_entry("p1", 3)]); // 3 < 60
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 3)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::DeckSizeBelowMin));
    }

    #[test]
    fn it_reports_equipment_slot_wrong_when_helm_in_chest_slot() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let helm = make_equipment("e_helm", "Heart of Fyendal", "Head");
        let catalog = catalog_with(
            vec![hero, helm],
            vec![
                make_printing("hero_p", "h"),
                make_printing("e_helm_p", "e_helm"),
            ],
        );
        let mut loadout = make_loadout("Main", vec![]);
        // Place a Head piece into the Chest slot.
        loadout.equipment.chest = Some(PrintingId::new("e_helm_p"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("e_helm_p", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::EquipmentSlotWrong));
    }

    #[test]
    fn it_reports_equipment_not_in_pool() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let helm = make_equipment("e_helm", "Heart of Fyendal", "Head");
        let catalog = catalog_with(
            vec![hero, helm],
            vec![
                make_printing("hero_p", "h"),
                make_printing("e_helm_p", "e_helm"),
            ],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.head = Some(PrintingId::new("e_helm_p"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![], // pool is empty; equipment refers to a printing not in pool
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::EquipmentNotInPool));
    }

    // -------- hand-slot configuration --------

    #[test]
    fn it_allows_zero_hand_slots_filled() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let catalog = catalog_with(vec![hero], vec![make_printing("hero_p", "h")]);
        let loadout = make_loadout("Main", vec![]);
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(!err.iter().any(|v| matches!(
            v.code,
            ViolationCode::WeaponConfigInvalid | ViolationCode::EquipmentSlotWrong
        )));
    }

    #[test]
    fn it_allows_one_handed_weapon_in_main_hand() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let dagger = make_weapon("w", "Dagger", WeaponGrip::OneHanded);
        let catalog = catalog_with(
            vec![hero, dagger],
            vec![make_printing("hero_p", "h"), make_printing("w1", "w")],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("w1"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("w1", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(!err.iter().any(|v| matches!(
            v.code,
            ViolationCode::WeaponConfigInvalid | ViolationCode::EquipmentSlotWrong
        )));
    }

    #[test]
    fn it_allows_one_handed_weapon_in_off_hand() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let dagger = make_weapon("w", "Dagger", WeaponGrip::OneHanded);
        let catalog = catalog_with(
            vec![hero, dagger],
            vec![make_printing("hero_p", "h"), make_printing("w1", "w")],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.off_hand = Some(PrintingId::new("w1"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("w1", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(!err.iter().any(|v| matches!(
            v.code,
            ViolationCode::WeaponConfigInvalid | ViolationCode::EquipmentSlotWrong
        )));
    }

    #[test]
    fn it_allows_two_one_handed_weapons() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let dagger = make_weapon("w", "Dagger", WeaponGrip::OneHanded);
        let catalog = catalog_with(
            vec![hero, dagger],
            vec![
                make_printing("hero_p", "h"),
                make_printing("w1", "w"),
                make_printing("w2", "w"),
            ],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("w1"));
        loadout.equipment.off_hand = Some(PrintingId::new("w2"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("w1", 1), pool_entry("w2", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(!err.iter().any(|v| matches!(
            v.code,
            ViolationCode::WeaponConfigInvalid | ViolationCode::EquipmentSlotWrong
        )));
    }

    #[test]
    fn it_allows_two_handed_weapon_in_main_hand_with_empty_off_hand() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let anothos = make_weapon("w", "Anothos", WeaponGrip::TwoHanded);
        let catalog = catalog_with(
            vec![hero, anothos],
            vec![make_printing("hero_p", "h"), make_printing("w1", "w")],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("w1"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("w1", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(!err.iter().any(|v| matches!(
            v.code,
            ViolationCode::WeaponConfigInvalid | ViolationCode::EquipmentSlotWrong
        )));
    }

    #[test]
    fn it_allows_off_hand_equipment_in_off_hand() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        // A non-Companion off-hand piece (e.g. a shield) — legal in the
        // off-hand slot when paired with a 1H weapon or no weapon.
        let shield = make_off_hand_equipment("e", "Tower of Rampart", false);
        let catalog = catalog_with(
            vec![hero, shield],
            vec![make_printing("hero_p", "h"), make_printing("ep", "e")],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.off_hand = Some(PrintingId::new("ep"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("ep", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(!err.iter().any(|v| matches!(
            v.code,
            ViolationCode::WeaponConfigInvalid | ViolationCode::EquipmentSlotWrong
        )));
    }

    #[test]
    fn it_allows_two_handed_with_companion_off_hand_equipment() {
        // The CR Companion exception: 2H weapon + Companion-typed off-hand
        // equipment is permitted simultaneously.
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let two_h = make_weapon("two_h", "Anothos", WeaponGrip::TwoHanded);
        let companion = make_off_hand_equipment("comp", "Trade Routes", true);
        let catalog = catalog_with(
            vec![hero, two_h, companion],
            vec![
                make_printing("hero_p", "h"),
                make_printing("p_two", "two_h"),
                make_printing("p_comp", "comp"),
            ],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("p_two"));
        loadout.equipment.off_hand = Some(PrintingId::new("p_comp"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p_two", 1), pool_entry("p_comp", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(!err.iter().any(|v| matches!(
            v.code,
            ViolationCode::WeaponConfigInvalid | ViolationCode::EquipmentSlotWrong
        )));
    }

    #[test]
    fn it_rejects_non_weapon_in_main_hand() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        // Action card passes supertype matching but isn't a weapon.
        let action = make_action("a", "Crippling Crush", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, action],
            vec![make_printing("hero_p", "h"), make_printing("a1", "a")],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("a1"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("a1", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        let slot_violations: Vec<_> = err
            .iter()
            .filter(|v| v.code == ViolationCode::EquipmentSlotWrong)
            .collect();
        assert_eq!(slot_violations.len(), 1, "violations: {err:?}");
        match &slot_violations[0].details {
            Some(ViolationDetails::EquipmentSlot { expected_slot, .. }) => {
                assert_eq!(expected_slot, "Main Hand");
            }
            other => panic!("unexpected details: {other:?}"),
        }
    }

    #[test]
    fn it_rejects_armor_in_main_hand() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let helm = make_equipment("e_helm", "Heart of Fyendal", "Head");
        let catalog = catalog_with(
            vec![hero, helm],
            vec![
                make_printing("hero_p", "h"),
                make_printing("e_helm_p", "e_helm"),
            ],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("e_helm_p"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("e_helm_p", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::EquipmentSlotWrong));
    }

    #[test]
    fn it_rejects_two_handed_weapon_in_off_hand() {
        // A 2H weapon in the off-hand slot is invalid regardless of what is
        // in the main hand — the off-hand can hold only 1H weapons or
        // off-hand-typed equipment.
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let two_h = make_weapon("w", "Anothos", WeaponGrip::TwoHanded);
        let catalog = catalog_with(
            vec![hero, two_h],
            vec![make_printing("hero_p", "h"), make_printing("w1", "w")],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.off_hand = Some(PrintingId::new("w1"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("w1", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::EquipmentSlotWrong));
    }

    #[test]
    fn it_rejects_armor_in_off_hand() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let helm = make_equipment("e_helm", "Heart of Fyendal", "Head");
        let catalog = catalog_with(
            vec![hero, helm],
            vec![
                make_printing("hero_p", "h"),
                make_printing("e_helm_p", "e_helm"),
            ],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.off_hand = Some(PrintingId::new("e_helm_p"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("e_helm_p", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::EquipmentSlotWrong));
    }

    #[test]
    fn it_rejects_action_card_in_off_hand() {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let action = make_action("a", "Crippling Crush", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, action],
            vec![make_printing("hero_p", "h"), make_printing("a1", "a")],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.off_hand = Some(PrintingId::new("a1"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("a1", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::EquipmentSlotWrong));
    }

    #[test]
    fn it_rejects_two_handed_with_one_handed_weapon_in_off_hand() {
        // 1H weapon is otherwise legal in off-hand, but with a 2H main hand
        // the 2H+Companion rule fires.
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let two_h = make_weapon("two_h", "Anothos", WeaponGrip::TwoHanded);
        let one_h = make_weapon("one_h", "Dagger", WeaponGrip::OneHanded);
        let catalog = catalog_with(
            vec![hero, two_h, one_h],
            vec![
                make_printing("hero_p", "h"),
                make_printing("p_two", "two_h"),
                make_printing("p_one", "one_h"),
            ],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("p_two"));
        loadout.equipment.off_hand = Some(PrintingId::new("p_one"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p_two", 1), pool_entry("p_one", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::WeaponConfigInvalid));
    }

    // -------- weapon zone count (Kayo, Bolfar) --------

    #[test]
    fn it_allows_one_weapon_for_a_one_zone_hero() {
        // Kayo, Armed and Dangerous: 1 weapon zone.
        let hero = with_weapon_zones(
            make_adult_hero("h", "Kayo, Armed and Dangerous", vec![Class::Brute], vec![]),
            1,
        );
        let dagger = make_weapon("w", "Romping Club", WeaponGrip::OneHanded);
        let catalog = catalog_with(
            vec![hero, dagger],
            vec![make_printing("hero_p", "h"), make_printing("w1", "w")],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("w1"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("w1", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(!err
            .iter()
            .any(|v| v.code == ViolationCode::WeaponConfigInvalid));
    }

    #[test]
    fn it_rejects_two_weapons_for_a_one_zone_hero() {
        let hero = with_weapon_zones(
            make_adult_hero("h", "Kayo, Armed and Dangerous", vec![Class::Brute], vec![]),
            1,
        );
        let dagger = make_weapon("w", "Romping Club", WeaponGrip::OneHanded);
        let catalog = catalog_with(
            vec![hero, dagger],
            vec![
                make_printing("hero_p", "h"),
                make_printing("w1", "w"),
                make_printing("w2", "w"),
            ],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("w1"));
        loadout.equipment.off_hand = Some(PrintingId::new("w2"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("w1", 1), pool_entry("w2", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::WeaponConfigInvalid));
    }

    #[test]
    fn it_allows_off_hand_equipment_alongside_weapon_for_a_one_zone_hero() {
        // 1-zone hero may still use off-hand equipment as long as only one
        // *weapon* is equipped overall.
        let hero = with_weapon_zones(
            make_adult_hero("h", "Kayo, Armed and Dangerous", vec![Class::Brute], vec![]),
            1,
        );
        let weapon = make_weapon("w", "Romping Club", WeaponGrip::OneHanded);
        let shield = make_off_hand_equipment("e", "Tower of Rampart", false);
        let catalog = catalog_with(
            vec![hero, weapon, shield],
            vec![
                make_printing("hero_p", "h"),
                make_printing("w1", "w"),
                make_printing("ep", "e"),
            ],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("w1"));
        loadout.equipment.off_hand = Some(PrintingId::new("ep"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("w1", 1), pool_entry("ep", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(!err.iter().any(|v| matches!(
            v.code,
            ViolationCode::WeaponConfigInvalid | ViolationCode::EquipmentSlotWrong
        )));
    }

    #[test]
    fn it_rejects_any_weapon_for_a_zero_zone_hero() {
        // Bolfar, Bear Hands: 0 weapon zones (Pit-Fighter; not CC-eligible
        // here, but the engine should still reject the weapon by the zone
        // rule independent of format eligibility).
        let hero = with_weapon_zones(
            make_adult_hero("h", "Bolfar, Bear Hands", vec![Class::Guardian], vec![]),
            0,
        );
        let weapon = make_weapon("w", "Anothos", WeaponGrip::OneHanded);
        let catalog = catalog_with(
            vec![hero, weapon],
            vec![make_printing("hero_p", "h"), make_printing("w1", "w")],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("w1"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("w1", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::WeaponConfigInvalid));
    }

    #[test]
    fn it_rejects_two_handed_with_non_companion_off_hand_equipment() {
        // A plain (non-Companion) off-hand equipment piece does not satisfy
        // the 2H exception; only Companion-subtyped pieces qualify.
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let two_h = make_weapon("two_h", "Anothos", WeaponGrip::TwoHanded);
        let shield = make_off_hand_equipment("e", "Tower of Rampart", false);
        let catalog = catalog_with(
            vec![hero, two_h, shield],
            vec![
                make_printing("hero_p", "h"),
                make_printing("p_two", "two_h"),
                make_printing("ep", "e"),
            ],
        );
        let mut loadout = make_loadout("Main", vec![]);
        loadout.equipment.main_hand = Some(PrintingId::new("p_two"));
        loadout.equipment.off_hand = Some(PrintingId::new("ep"));
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p_two", 1), pool_entry("ep", 1)],
            vec![loadout],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::WeaponConfigInvalid));
    }

    #[test]
    fn it_reports_hero_missing_hero_type_when_action_is_used_as_hero() {
        let mut not_a_hero = make_generic_action("h", "Not A Hero", 1);
        // Not actually a Hero type, but used as the deck's hero.
        not_a_hero.types = vec![CardType::Action];
        let catalog = catalog_with(vec![not_a_hero], vec![make_printing("hero_p", "h")]);
        let deck = build_deck(FormatId::ClassicConstructed, "hero_p", vec![], vec![]);
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::HeroMissingHeroType));
    }

    #[test]
    fn it_aggregates_multiple_violations() {
        // Hero off-class card + off-class card + missing printing.
        let hero = make_adult_hero("h", "Wiz", vec![Class::Wizard], vec![]);
        let off = make_action("c1", "Warrior Strike", 1, vec![Class::Warrior], vec![]);
        let catalog = catalog_with(
            vec![hero, off],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c1")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![pool_entry("p1", 1), pool_entry("ghost", 1)],
            vec![],
        );
        let err = validate(&deck, &cc(), date(), &catalog).unwrap_err();
        assert!(err.len() >= 2);
        let codes: Vec<_> = err.iter().map(|v| v.code).collect();
        assert!(codes.contains(&ViolationCode::SupertypeMismatch));
        assert!(codes.contains(&ViolationCode::PrintingNotFoundInCatalog));
    }
}
