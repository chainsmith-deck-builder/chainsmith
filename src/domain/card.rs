//! Card and printing domain types.
//!
//! These mirror the upstream `the-fab-cube/flesh-and-blood-cards` shape but
//! collapse the fields the validation engine actually needs. Sync layer code
//! (when added) is responsible for turning the upstream JSON into these
//! values; the engine itself does no IO.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::ids::{CardId, HeroMoniker, PrintingId, SetCode};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: CardId,
    pub name: String,
    pub pitch: Option<u8>,
    pub cost: Option<u8>,
    pub power: Option<u8>,
    pub defense: Option<u8>,
    pub types: Vec<CardType>,
    pub subtypes: Vec<String>,
    pub classes: Vec<Class>,
    pub talents: Vec<Talent>,
    pub keywords: Vec<Keyword>,
    pub specializations: Vec<HeroMoniker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functional_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flavor_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero: Option<HeroFacts>,
    /// Grip on weapon cards. `None` for non-weapons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weapon_grip: Option<WeaponGrip>,
    pub legality_summary: LegalitySummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeaponGrip {
    OneHanded,
    TwoHanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CardType {
    Hero,
    Action,
    AttackAction,
    AttackReaction,
    DefenseReaction,
    Instant,
    Equipment,
    Weapon,
    Mentor,
    Resource,
    Token,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Assassin,
    Bard,
    Brute,
    Guardian,
    Illusionist,
    Mechanologist,
    Merchant,
    Ninja,
    Ranger,
    Runeblade,
    Shapeshifter,
    Warrior,
    Wizard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Talent {
    Draconic,
    Earth,
    Elemental,
    Ice,
    Light,
    Lightning,
    Shadow,
    Wind,
}

/// Free-form keyword. Upstream prints new keywords every set, so an enum here
/// would be brittle and is not needed for legality checks (the validator does
/// not switch on keyword).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(transparent)]
pub struct Keyword(String);

impl Keyword {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeroFacts {
    pub kind: HeroKind,
    pub life: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intellect: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arcane: Option<u8>,
    /// Class/Talent supertypes the hero grants via "Essence of X" abilities.
    /// These are unioned with the hero's printed supertypes when checking
    /// CR 1.1.3 (supertype subset rule).
    pub essence_grants: Vec<EssenceGrant>,
    /// Number of weapon zones the hero has — i.e. the maximum number of
    /// `CardType::Weapon` printings the loadout may equip across main and
    /// off-hand combined. Default 2 (the standard hero). Kayo variants
    /// override to 1; Bolfar to 0. Non-weapon equipment in the off-hand slot
    /// (shields, Companion items, etc.) does not consume a weapon zone.
    pub weapon_zone_count: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeroKind {
    Adult,
    Young,
    PitFighter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EssenceGrant {
    Class(Class),
    Talent(Talent),
}

/// Snapshot of per-format legality from the upstream `card.json` flat booleans.
/// Convenience for catalog filtering only — the validator is the source of
/// truth for legality at a specific date.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LegalitySummary {
    pub cc: FormatStatus,
    pub blitz: FormatStatus,
    pub commoner: FormatStatus,
    pub silver_age: FormatStatus,
    pub living_legend: FormatStatus,
    pub upf: FormatStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FormatStatus {
    #[default]
    Legal,
    Banned,
    Suspended,
    Restricted,
    LivingLegendRetired,
    NotEligible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Printing {
    pub id: PrintingId,
    pub card_id: CardId,
    pub set: SetCode,
    pub set_release_date: NaiveDate,
    pub edition: Edition,
    pub foiling: Foiling,
    pub treatment: Treatment,
    pub rarity: Rarity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    pub collector_number: String,
    /// CDN URL for the card image, sourced from upstream. Clients (web,
    /// iOS, Android) load this URL directly from the LSS CDN — we do not
    /// proxy or cache imagery on our infrastructure (see
    /// `.claude/rules/security.md`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Edition {
    Alpha,
    Unlimited,
    First,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Foiling {
    Standard,
    RainbowFoil,
    ColdFoil,
    GoldFoil,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Treatment {
    Standard,
    ExtendedArt,
    Marvel,
    AlternateArt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Rarity {
    Common,
    Rare,
    SuperRare,
    Majestic,
    Legendary,
    Fabled,
    Promo,
    Token,
}
