//! Classic Constructed format implementation.
//!
//! Rules captured here come from the FaB Tournament Rules and Policy and the
//! Card Legality Policy as of 2026-05-02. Banned and Living Legend lists are
//! injected at construction time so the engine itself stays a pure function
//! of its inputs — production code constructs `ClassicConstructed::new(...)`
//! with data loaded by the sync layer from
//! `the-fab-cube/flesh-and-blood-cards`'s `banned-cc.json` and
//! `living-legend-cc.json`. Tests construct empty or hand-crafted lists.

use chrono::NaiveDate;

use crate::domain::card::{Card, CardType, HeroKind, Printing};
use crate::domain::ids::CardId;
use crate::domain::violation::{Violation, ViolationCode, ViolationDetails};

use super::{CopyLimit, Format, FormatId, FormatRules};

/// CC's static rule values.
const RULES: FormatRules = FormatRules {
    min_deck_size: 60,
    max_deck_size: None,
    card_pool_size: Some(80),
    card_copy_limit: CopyLimit::Exact(3),
    equipment_inventory_limit: None,
};

#[derive(Debug, Clone)]
pub struct ClassicConstructed {
    rules: FormatRules,
    banned: Vec<BannedEntry>,
    living_legend: Vec<LivingLegendEntry>,
}

/// One row from the upstream `banned-cc.json` announcement audit trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannedEntry {
    pub card_id: CardId,
    pub effective_date: NaiveDate,
    /// `true` if the ban is currently in force; `false` for historical entries
    /// that have since been unbanned. The validator filters by date AND active
    /// status: an unbanned card with `status_active = false` is not treated
    /// as banned.
    pub status_active: bool,
}

/// One row from the upstream `living-legend-cc.json` announcement trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivingLegendEntry {
    pub card_id: CardId,
    pub effective_date: NaiveDate,
}

impl ClassicConstructed {
    pub fn new(banned: Vec<BannedEntry>, living_legend: Vec<LivingLegendEntry>) -> Self {
        Self {
            rules: RULES,
            banned,
            living_legend,
        }
    }

    /// CC with empty banned and Living Legend lists. Useful as a baseline in
    /// tests that don't care about either, and for the engine in environments
    /// where the lists haven't been loaded yet.
    pub fn empty() -> Self {
        Self::new(Vec::new(), Vec::new())
    }

    fn is_living_legend_retired(&self, hero_id: &CardId, date: NaiveDate) -> bool {
        self.living_legend
            .iter()
            .filter(|e| e.effective_date <= date)
            .any(|e| &e.card_id == hero_id)
    }
}

impl Default for ClassicConstructed {
    fn default() -> Self {
        Self::empty()
    }
}

impl Format for ClassicConstructed {
    fn id(&self) -> FormatId {
        FormatId::ClassicConstructed
    }

    fn rules(&self) -> &FormatRules {
        &self.rules
    }

    fn hero_eligible(&self, hero: &Card, date: NaiveDate) -> Result<(), Violation> {
        let kind = hero.hero.as_ref().map(|f| f.kind);
        if !matches!(kind, Some(HeroKind::Adult)) {
            return Err(Violation {
                code: ViolationCode::HeroNotEligibleForFormat,
                message: format!(
                    "{} is not eligible for Classic Constructed (Adult heroes only)",
                    hero.name
                ),
                details: Some(ViolationDetails::Card {
                    card_id: hero.id.clone(),
                    name: hero.name.clone(),
                }),
            });
        }
        if self.is_living_legend_retired(&hero.id, date) {
            return Err(Violation {
                code: ViolationCode::HeroLivingLegendRetired,
                message: format!(
                    "{} has been retired to Living Legend in Classic Constructed",
                    hero.name
                ),
                details: Some(ViolationDetails::Card {
                    card_id: hero.id.clone(),
                    name: hero.name.clone(),
                }),
            });
        }
        Ok(())
    }

    fn card_eligible(
        &self,
        card: &Card,
        _printing: &Printing,
        _date: NaiveDate,
    ) -> Result<(), Violation> {
        // Tokens are excluded from the card-pool per CR 1.3.2 / CR 8.1.8a.
        // Set, rarity, and edition are unrestricted in CC; the only filter
        // applied by this hook is "not a token."
        if card.types.contains(&CardType::Token) {
            return Err(Violation {
                code: ViolationCode::CardNotEligibleForFormat,
                message: format!(
                    "'{}' is a token and cannot be in a Classic Constructed pool",
                    card.name
                ),
                details: Some(ViolationDetails::Card {
                    card_id: card.id.clone(),
                    name: card.name.clone(),
                }),
            });
        }
        Ok(())
    }

    fn banned_at(&self, card_id: &CardId, date: NaiveDate) -> bool {
        self.banned
            .iter()
            .filter(|e| e.effective_date <= date && e.status_active)
            .any(|e| &e.card_id == card_id)
    }

    fn restricted_at(&self, _card_id: &CardId, _date: NaiveDate) -> bool {
        // CC has no restricted list as of this engine version.
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::card::Class;
    use crate::domain::deck::PoolEntry;
    use crate::domain::format::{validate, FormatRules};
    use crate::domain::ids::PrintingId;
    use crate::domain::test_support::*;

    fn date_2026_05_02() -> NaiveDate {
        release_on(2026, 5, 2)
    }

    #[test]
    fn it_exposes_cc_rules_with_60_pool_80_copy_3() {
        let cc = ClassicConstructed::empty();
        let r: &FormatRules = cc.rules();
        assert_eq!(r.min_deck_size, 60);
        assert_eq!(r.card_pool_size, Some(80));
        assert_eq!(r.card_copy_limit, CopyLimit::Exact(3));
    }

    #[test]
    fn it_reports_format_id_classic_constructed() {
        assert_eq!(
            ClassicConstructed::empty().id(),
            FormatId::ClassicConstructed
        );
    }

    // -------- hero_eligible --------

    #[test]
    fn it_accepts_adult_hero() {
        let cc = ClassicConstructed::empty();
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        assert!(cc.hero_eligible(&hero, date_2026_05_02()).is_ok());
    }

    #[test]
    fn it_rejects_young_hero() {
        let cc = ClassicConstructed::empty();
        let hero = make_young_hero("h", "Young Bravo", vec![Class::Guardian], vec![]);
        let err = cc.hero_eligible(&hero, date_2026_05_02()).unwrap_err();
        assert_eq!(err.code, ViolationCode::HeroNotEligibleForFormat);
    }

    #[test]
    fn it_rejects_pit_fighter_hero() {
        let cc = ClassicConstructed::empty();
        let hero = make_hero(
            "h",
            "Levia, Shadowborn Abomination",
            HeroKind::PitFighter,
            vec![Class::Brute],
            vec![],
        );
        let err = cc.hero_eligible(&hero, date_2026_05_02()).unwrap_err();
        assert_eq!(err.code, ViolationCode::HeroNotEligibleForFormat);
    }

    #[test]
    fn it_rejects_card_used_as_hero_when_no_hero_facts() {
        let cc = ClassicConstructed::empty();
        // A card without HeroFacts should be ineligible as a hero.
        let mut card = make_generic_action("h", "Not A Hero", 1);
        card.types = vec![CardType::Hero]; // typed as Hero but no facts
        let err = cc.hero_eligible(&card, date_2026_05_02()).unwrap_err();
        assert_eq!(err.code, ViolationCode::HeroNotEligibleForFormat);
    }

    #[test]
    fn it_rejects_adult_hero_when_living_legend_retired_on_or_before_date() {
        // Hero was retired on 2025-02-25 (Viserai). Validation date 2026-05-02.
        let cc = ClassicConstructed::new(
            Vec::new(),
            vec![LivingLegendEntry {
                card_id: CardId::new("hero_id"),
                effective_date: release_on(2025, 2, 25),
            }],
        );
        let hero = make_adult_hero("hero_id", "Viserai", vec![Class::Runeblade], vec![]);
        let err = cc.hero_eligible(&hero, date_2026_05_02()).unwrap_err();
        assert_eq!(err.code, ViolationCode::HeroLivingLegendRetired);
    }

    #[test]
    fn it_accepts_adult_hero_when_validation_date_is_before_living_legend_effective_date() {
        // Hero retires 2025-02-25; validating on 2025-02-24 should pass.
        let cc = ClassicConstructed::new(
            Vec::new(),
            vec![LivingLegendEntry {
                card_id: CardId::new("hero_id"),
                effective_date: release_on(2025, 2, 25),
            }],
        );
        let hero = make_adult_hero("hero_id", "Viserai", vec![Class::Runeblade], vec![]);
        assert!(cc.hero_eligible(&hero, release_on(2025, 2, 24)).is_ok());
    }

    #[test]
    fn it_rejects_adult_hero_exactly_on_living_legend_effective_date() {
        // The retirement effective date is inclusive — that day's tournaments
        // already exclude the hero.
        let cc = ClassicConstructed::new(
            Vec::new(),
            vec![LivingLegendEntry {
                card_id: CardId::new("hero_id"),
                effective_date: release_on(2025, 2, 25),
            }],
        );
        let hero = make_adult_hero("hero_id", "Viserai", vec![Class::Runeblade], vec![]);
        let err = cc
            .hero_eligible(&hero, release_on(2025, 2, 25))
            .unwrap_err();
        assert_eq!(err.code, ViolationCode::HeroLivingLegendRetired);
    }

    // -------- card_eligible --------

    #[test]
    fn it_accepts_a_normal_action_card() {
        let cc = ClassicConstructed::empty();
        let card = make_generic_action("c", "Strike", 1);
        let printing = make_printing("p", "c");
        assert!(cc
            .card_eligible(&card, &printing, date_2026_05_02())
            .is_ok());
    }

    #[test]
    fn it_rejects_a_token_card() {
        let cc = ClassicConstructed::empty();
        let card = make_token("t", "Spectral Shield");
        let printing = make_printing("p", "t");
        let err = cc
            .card_eligible(&card, &printing, date_2026_05_02())
            .unwrap_err();
        assert_eq!(err.code, ViolationCode::CardNotEligibleForFormat);
    }

    // -------- banned_at --------

    #[test]
    fn it_reports_card_banned_when_active_entry_is_on_or_before_date() {
        let cc = ClassicConstructed::new(
            vec![BannedEntry {
                card_id: CardId::new("ball_lightning_red"),
                effective_date: release_on(2024, 1, 1),
                status_active: true,
            }],
            Vec::new(),
        );
        assert!(cc.banned_at(&CardId::new("ball_lightning_red"), date_2026_05_02()));
    }

    #[test]
    fn it_does_not_report_card_banned_when_entry_is_after_date() {
        // Pre-announced ban effective 2026-05-28; validating on 2026-05-02 is
        // before, so the card is still legal.
        let cc = ClassicConstructed::new(
            vec![BannedEntry {
                card_id: CardId::new("phantom_tidemaw"),
                effective_date: release_on(2026, 5, 28),
                status_active: true,
            }],
            Vec::new(),
        );
        assert!(!cc.banned_at(&CardId::new("phantom_tidemaw"), date_2026_05_02()));
    }

    #[test]
    fn it_does_not_report_card_banned_when_entry_is_inactive() {
        // Card was banned but later unbanned: status_active = false.
        let cc = ClassicConstructed::new(
            vec![BannedEntry {
                card_id: CardId::new("scepter_of_pain"),
                effective_date: release_on(2024, 1, 1),
                status_active: false,
            }],
            Vec::new(),
        );
        assert!(!cc.banned_at(&CardId::new("scepter_of_pain"), date_2026_05_02()));
    }

    #[test]
    fn it_treats_a_card_with_two_announcements_as_currently_status_of_latest() {
        // Realistic scenario: a card was banned, then unbanned. The upstream
        // emits two rows. The validator's `status_active` flag on each row is
        // authoritative. The active=false row alone is enough.
        let cc = ClassicConstructed::new(
            vec![
                BannedEntry {
                    card_id: CardId::new("rootbound_carapace"),
                    effective_date: release_on(2024, 6, 1),
                    status_active: false,
                },
                BannedEntry {
                    card_id: CardId::new("rootbound_carapace"),
                    effective_date: release_on(2026, 3, 24),
                    status_active: false,
                },
            ],
            Vec::new(),
        );
        assert!(!cc.banned_at(&CardId::new("rootbound_carapace"), date_2026_05_02()));
    }

    // -------- restricted_at --------

    #[test]
    fn it_reports_no_restricted_cards_in_cc() {
        let cc = ClassicConstructed::empty();
        assert!(!cc.restricted_at(&CardId::new("any"), date_2026_05_02()));
    }

    // -------- end-to-end --------

    #[test]
    fn it_rejects_deck_with_banned_card_in_pool() {
        let cc = ClassicConstructed::new(
            vec![BannedEntry {
                card_id: CardId::new("c1"),
                effective_date: release_on(2024, 1, 1),
                status_active: true,
            }],
            Vec::new(),
        );
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let banned_card = make_action("c1", "Banned Card", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, banned_card],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c1")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![PoolEntry {
                printing: PrintingId::new("p1"),
                quantity: 1,
            }],
            vec![],
        );
        let err = validate(&deck, &cc, date_2026_05_02(), &catalog).unwrap_err();
        assert!(err.iter().any(|v| v.code == ViolationCode::CardBanned));
    }

    #[test]
    fn it_rejects_deck_with_pool_over_eighty() {
        let cc = ClassicConstructed::empty();
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let card = make_action("c1", "Generic", 1, vec![Class::Guardian], vec![]);
        let catalog = catalog_with(
            vec![hero, card],
            vec![make_printing("hero_p", "h"), make_printing("p1", "c1")],
        );
        // 81 of one printing also exceeds the 3-copy limit, but the pool-size
        // check fires regardless, and that's what we're asserting here.
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![PoolEntry {
                printing: PrintingId::new("p1"),
                quantity: 81,
            }],
            vec![],
        );
        let err = validate(&deck, &cc, date_2026_05_02(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::PoolSizeAboveMax));
    }

    #[test]
    fn it_rejects_deck_with_token_in_pool() {
        let cc = ClassicConstructed::empty();
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let token = make_token("t", "Phantasm Token");
        let catalog = catalog_with(
            vec![hero, token],
            vec![make_printing("hero_p", "h"), make_printing("p1", "t")],
        );
        let deck = build_deck(
            FormatId::ClassicConstructed,
            "hero_p",
            vec![PoolEntry {
                printing: PrintingId::new("p1"),
                quantity: 1,
            }],
            vec![],
        );
        let err = validate(&deck, &cc, date_2026_05_02(), &catalog).unwrap_err();
        assert!(err
            .iter()
            .any(|v| v.code == ViolationCode::CardNotEligibleForFormat));
    }
}
