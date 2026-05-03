//! In-memory card catalog used by the validation engine for lookups.
//!
//! The validator depends only on the methods exposed here, so a future
//! database-backed catalog can implement the same surface without disturbing
//! the engine. For now the only implementation is a pair of `HashMap`s.

use std::collections::HashMap;

use crate::domain::card::{Card, Printing};
use crate::domain::ids::{CardId, PrintingId};

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    cards: HashMap<CardId, Card>,
    printings: HashMap<PrintingId, Printing>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_card(&mut self, card: Card) {
        self.cards.insert(card.id.clone(), card);
    }

    pub fn insert_printing(&mut self, printing: Printing) {
        self.printings.insert(printing.id.clone(), printing);
    }

    pub fn card(&self, id: &CardId) -> Option<&Card> {
        self.cards.get(id)
    }

    pub fn printing(&self, id: &PrintingId) -> Option<&Printing> {
        self.printings.get(id)
    }

    /// Resolve a printing id to its `(printing, card)` pair. Returns `None` if
    /// the printing is missing or its referenced card is not in the catalog.
    pub fn resolve(&self, id: &PrintingId) -> Option<(&Printing, &Card)> {
        let printing = self.printing(id)?;
        let card = self.card(&printing.card_id)?;
        Some((printing, card))
    }

    /// Iterate over every card. Order is unspecified; callers that need
    /// deterministic ordering must sort.
    pub fn cards(&self) -> impl Iterator<Item = &Card> {
        self.cards.values()
    }

    /// All printings of a given card. Linear scan — fine for current data
    /// volume (~14k printings). If this becomes a hotspot, add an index from
    /// `CardId -> Vec<PrintingId>` at insertion time.
    pub fn printings_for_card(&self, card_id: &CardId) -> impl Iterator<Item = &Printing> + '_ {
        let card_id = card_id.clone();
        self.printings
            .values()
            .filter(move |p| p.card_id == card_id)
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::domain::card::{
        Card, CardType, Edition, Foiling, FormatStatus, LegalitySummary, Printing, Rarity,
        Treatment,
    };
    use crate::domain::ids::SetCode;

    fn dummy_card(id: &str) -> Card {
        Card {
            id: CardId::new(id),
            name: format!("card {id}"),
            pitch: None,
            cost: None,
            power: None,
            defense: None,
            types: vec![CardType::Action],
            subtypes: Vec::new(),
            classes: Vec::new(),
            talents: Vec::new(),
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

    fn dummy_printing(printing_id: &str, card_id: &str) -> Printing {
        Printing {
            id: PrintingId::new(printing_id),
            card_id: CardId::new(card_id),
            set: SetCode::new("WTR"),
            set_release_date: NaiveDate::from_ymd_opt(2019, 10, 11).unwrap(),
            edition: Edition::First,
            foiling: Foiling::Standard,
            treatment: Treatment::Standard,
            rarity: Rarity::Common,
            artist: None,
            collector_number: "001".into(),
            image_url: None,
        }
    }

    #[test]
    fn it_resolves_a_printing_to_its_card_when_both_present() {
        let mut catalog = Catalog::new();
        catalog.insert_card(dummy_card("c1"));
        catalog.insert_printing(dummy_printing("p1", "c1"));

        let (printing, card) = catalog.resolve(&PrintingId::new("p1")).unwrap();
        assert_eq!(printing.id, PrintingId::new("p1"));
        assert_eq!(card.id, CardId::new("c1"));
    }

    #[test]
    fn it_returns_none_when_resolving_missing_printing() {
        let catalog = Catalog::new();
        assert!(catalog.resolve(&PrintingId::new("missing")).is_none());
    }

    #[test]
    fn it_returns_none_when_printing_is_present_but_card_is_missing() {
        // Distinct case from missing printing — the catalog has been populated
        // inconsistently. Validator should report this as a separate error.
        let mut catalog = Catalog::new();
        catalog.insert_printing(dummy_printing("p1", "c1"));
        assert!(catalog.printing(&PrintingId::new("p1")).is_some());
        assert!(catalog.card(&CardId::new("c1")).is_none());
        assert!(catalog.resolve(&PrintingId::new("p1")).is_none());
    }

    #[test]
    fn it_overwrites_a_card_when_inserted_with_same_id() {
        let mut catalog = Catalog::new();
        let mut first = dummy_card("c1");
        first.name = "first".into();
        catalog.insert_card(first);

        let mut second = dummy_card("c1");
        second.name = "second".into();
        catalog.insert_card(second);

        assert_eq!(catalog.card(&CardId::new("c1")).unwrap().name, "second");
    }
}
