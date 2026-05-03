//! Test-only fixture builders shared across the engine's unit tests.
//!
//! Compiled only under `#[cfg(test)]`. Not part of the public API. Builders
//! are deliberately small — large fixture data should live in
//! `tests/fixtures/` per `.claude/rules/testing.md`.

use chrono::NaiveDate;

use crate::domain::card::{
    Card, CardType, Class, Edition, EssenceGrant, Foiling, FormatStatus, HeroFacts, HeroKind,
    Keyword, LegalitySummary, Printing, Rarity, Talent, Treatment, WeaponGrip,
};
use crate::domain::catalog::Catalog;
use crate::domain::deck::{Deck, EquipmentLoadout, Loadout, LoadoutEntry, PoolEntry};
use crate::domain::format::FormatId;
use crate::domain::ids::{CardId, HeroMoniker, PrintingId, SetCode};

/// A reasonable default release date used when tests don't care about
/// chronology. Picked to predate any LL retirement or B&R announcement we
/// might exercise.
pub fn default_release_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2019, 10, 11).unwrap()
}

pub fn release_on(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

pub fn make_hero(
    id: &str,
    name: &str,
    kind: HeroKind,
    classes: Vec<Class>,
    talents: Vec<Talent>,
) -> Card {
    Card {
        id: CardId::new(id),
        name: name.into(),
        pitch: None,
        cost: None,
        power: None,
        defense: None,
        types: vec![CardType::Hero],
        subtypes: Vec::new(),
        classes,
        talents,
        keywords: Vec::new(),
        specializations: Vec::new(),
        functional_text: None,
        type_text: None,
        flavor_text: None,
        hero: Some(HeroFacts {
            kind,
            life: 40,
            intellect: Some(4),
            arcane: None,
            essence_grants: Vec::new(),
            weapon_zone_count: 2,
        }),
        weapon_grip: None,
        legality_summary: LegalitySummary::default(),
    }
}

/// Override the hero's weapon zone count (default 2; Kayo = 1; Bolfar = 0).
pub fn with_weapon_zones(mut hero: Card, count: u8) -> Card {
    if let Some(facts) = hero.hero.as_mut() {
        facts.weapon_zone_count = count;
    }
    hero
}

/// Convenience: an Adult-CC hero with the given class and talents.
pub fn make_adult_hero(id: &str, name: &str, classes: Vec<Class>, talents: Vec<Talent>) -> Card {
    make_hero(id, name, HeroKind::Adult, classes, talents)
}

pub fn make_young_hero(id: &str, name: &str, classes: Vec<Class>, talents: Vec<Talent>) -> Card {
    make_hero(id, name, HeroKind::Young, classes, talents)
}

pub fn with_essence_grants(mut hero: Card, grants: Vec<EssenceGrant>) -> Card {
    if let Some(facts) = hero.hero.as_mut() {
        facts.essence_grants = grants;
    }
    hero
}

pub fn make_action(
    id: &str,
    name: &str,
    pitch: u8,
    classes: Vec<Class>,
    talents: Vec<Talent>,
) -> Card {
    Card {
        id: CardId::new(id),
        name: name.into(),
        pitch: Some(pitch),
        cost: Some(0),
        power: Some(4),
        defense: Some(3),
        types: vec![CardType::Action],
        subtypes: Vec::new(),
        classes,
        talents,
        keywords: Vec::new(),
        specializations: Vec::new(),
        functional_text: None,
        type_text: None,
        flavor_text: None,
        hero: None,
        weapon_grip: None,
        legality_summary: LegalitySummary {
            cc: FormatStatus::Legal,
            ..LegalitySummary::default()
        },
    }
}

/// Convenience: a Generic action with no class or talent restrictions.
pub fn make_generic_action(id: &str, name: &str, pitch: u8) -> Card {
    make_action(id, name, pitch, Vec::new(), Vec::new())
}

pub fn make_specialized_action(
    id: &str,
    name: &str,
    classes: Vec<Class>,
    monikers: Vec<&str>,
) -> Card {
    let mut card = make_action(id, name, 1, classes, Vec::new());
    card.specializations = monikers.into_iter().map(HeroMoniker::new).collect();
    card
}

pub fn make_token(id: &str, name: &str) -> Card {
    let mut card = make_generic_action(id, name, 0);
    card.types = vec![CardType::Token];
    card.keywords.push(Keyword::new("Token"));
    card
}

pub fn make_equipment(id: &str, name: &str, slot_subtype: &str) -> Card {
    Card {
        id: CardId::new(id),
        name: name.into(),
        pitch: None,
        cost: None,
        power: None,
        defense: Some(2),
        types: vec![CardType::Equipment],
        subtypes: vec![slot_subtype.into()],
        classes: Vec::new(),
        talents: Vec::new(),
        keywords: Vec::new(),
        specializations: Vec::new(),
        functional_text: None,
        type_text: None,
        flavor_text: None,
        hero: None,
        weapon_grip: None,
        legality_summary: LegalitySummary::default(),
    }
}

/// Off-hand equipment, optionally with the Companion subtype that allows
/// pairing with a two-handed weapon per CR.
pub fn make_off_hand_equipment(id: &str, name: &str, companion: bool) -> Card {
    let mut card = make_equipment(id, name, "Off-Hand");
    if companion {
        card.subtypes.push("Companion".into());
    }
    card
}

pub fn make_weapon(id: &str, name: &str, grip: WeaponGrip) -> Card {
    Card {
        id: CardId::new(id),
        name: name.into(),
        pitch: None,
        cost: None,
        power: Some(4),
        defense: None,
        types: vec![CardType::Weapon],
        subtypes: Vec::new(),
        classes: Vec::new(),
        talents: Vec::new(),
        keywords: Vec::new(),
        specializations: Vec::new(),
        functional_text: None,
        type_text: None,
        flavor_text: None,
        hero: None,
        weapon_grip: Some(grip),
        legality_summary: LegalitySummary::default(),
    }
}

pub fn make_printing(printing_id: &str, card_id: &str) -> Printing {
    make_printing_at(printing_id, card_id, default_release_date())
}

pub fn make_printing_at(printing_id: &str, card_id: &str, release: NaiveDate) -> Printing {
    Printing {
        id: PrintingId::new(printing_id),
        card_id: CardId::new(card_id),
        set: SetCode::new("WTR"),
        set_release_date: release,
        edition: Edition::First,
        foiling: Foiling::Standard,
        treatment: Treatment::Standard,
        rarity: Rarity::Common,
        artist: None,
        collector_number: "001".into(),
        image_url: None,
    }
}

pub fn pool_entry(printing_id: &str, qty: u8) -> PoolEntry {
    PoolEntry {
        printing: PrintingId::new(printing_id),
        quantity: qty,
    }
}

pub fn loadout_entry(printing_id: &str, qty: u8) -> LoadoutEntry {
    LoadoutEntry {
        printing: PrintingId::new(printing_id),
        quantity: qty,
    }
}

pub fn build_deck(
    format: FormatId,
    hero_printing: &str,
    pool: Vec<PoolEntry>,
    loadouts: Vec<Loadout>,
) -> Deck {
    Deck {
        format,
        hero: PrintingId::new(hero_printing),
        pool,
        loadouts,
    }
}

pub fn make_loadout(name: &str, deck_cards: Vec<LoadoutEntry>) -> Loadout {
    Loadout {
        name: name.into(),
        deck_cards,
        equipment: EquipmentLoadout::default(),
    }
}

/// Build a catalog populated with the given cards and printings. Most tests
/// will pair a hero card with a hero printing and one or more action
/// cards/printings.
pub fn catalog_with(cards: Vec<Card>, printings: Vec<Printing>) -> Catalog {
    let mut catalog = Catalog::new();
    for c in cards {
        catalog.insert_card(c);
    }
    for p in printings {
        catalog.insert_printing(p);
    }
    catalog
}
