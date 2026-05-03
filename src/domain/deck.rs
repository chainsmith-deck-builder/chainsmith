//! Deck, pool, and loadout domain types.
//!
//! The `Deck` is the format-validated entity: its `pool` is the canonical
//! 80-card-or-fewer set the player owns and registers. `Loadout`s are
//! pre-saved 60-card subsets of the pool with an equipment configuration,
//! intended to support per-matchup sideboard plans (e.g. "vs aggro"). The
//! engine validates the pool against the format; loadouts are validated for
//! "is this a legal selection from the parent pool?" but introduce no new
//! format rules.

use crate::domain::format::FormatId;
use crate::domain::ids::PrintingId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deck {
    pub format: FormatId,
    pub hero: PrintingId,
    pub pool: Vec<PoolEntry>,
    pub loadouts: Vec<Loadout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolEntry {
    pub printing: PrintingId,
    pub quantity: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loadout {
    pub name: String,
    pub deck_cards: Vec<LoadoutEntry>,
    pub equipment: EquipmentLoadout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadoutEntry {
    pub printing: PrintingId,
    pub quantity: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EquipmentLoadout {
    pub head: Option<PrintingId>,
    pub chest: Option<PrintingId>,
    pub arms: Option<PrintingId>,
    pub legs: Option<PrintingId>,
    /// Main-hand slot. Holds a weapon (1H or 2H). A 2H weapon here occupies
    /// both hand slots; see `off_hand` for the Companion exception.
    pub main_hand: Option<PrintingId>,
    /// Off-hand slot. Holds either a 1H weapon, an off-hand equipment piece
    /// (shield, Trade Routes, etc.), or — when `main_hand` carries a 2H
    /// weapon — only a Companion-subtyped off-hand equipment piece.
    pub off_hand: Option<PrintingId>,
}
