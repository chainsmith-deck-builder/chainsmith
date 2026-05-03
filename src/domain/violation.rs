//! Validation engine outputs.
//!
//! A `Violation` is a structured rule failure. The `code` is stable across
//! versions so clients can switch on it; the `message` is human-readable and
//! may change. `details` carries optional structured context (which card,
//! which slot, etc.).

use crate::domain::ids::{CardId, PrintingId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub code: ViolationCode,
    pub message: String,
    pub details: Option<ViolationDetails>,
}

/// Stable codes clients are allowed to switch on.
///
/// Adding a variant is additive. Renaming or removing one is a breaking
/// change in the production phase (see `.claude/rules/api-contract.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationCode {
    HeroPrintingNotFoundInCatalog,
    HeroNotEligibleForFormat,
    HeroLivingLegendRetired,
    HeroMissingHeroType,

    PoolSizeAboveMax,
    DeckSizeBelowMin,
    DeckSizeAboveMax,
    CopyLimitExceeded,
    RestrictedCopyLimitExceeded,

    CardNotFoundInCatalog,
    PrintingNotFoundInCatalog,
    CardNotEligibleForFormat,
    CardBanned,
    SupertypeMismatch,
    SpecializationMismatch,

    LoadoutPrintingNotInPool,
    LoadoutQuantityExceedsPool,
    EquipmentSlotWrong,
    EquipmentNotInPool,
    WeaponConfigInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationDetails {
    Card {
        card_id: CardId,
        name: String,
    },
    Printing {
        printing_id: PrintingId,
    },
    Quantity {
        card_id: CardId,
        found: u32,
        allowed: u32,
    },
    DeckSize {
        found: u32,
        min: u32,
        max: Option<u32>,
    },
    PoolSize {
        found: u32,
        max: u32,
    },
    EquipmentSlot {
        printing_id: PrintingId,
        expected_slot: String,
        actual_slot: Option<String>,
    },
    LoadoutCard {
        loadout: String,
        printing_id: PrintingId,
    },
}
