//! Newtype identifiers used across the domain layer.
//!
//! Distinguishing `CardId` from `PrintingId` at the type level prevents the
//! easy mistake of passing one where the other is expected — the validator
//! aggregates legality by card identity but renders by printing identity.

use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
pub struct CardId(String);

impl CardId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
pub struct PrintingId(String);

impl PrintingId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PrintingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The first comma-separated token of a hero's name (e.g. "Bravo, Star of the
/// Show" -> "Bravo"). FaB specialization rules match on moniker, not full name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
pub struct HeroMoniker(String);

impl HeroMoniker {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HeroMoniker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
pub struct SetCode(String);

impl SetCode {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SetCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_displays_card_id_as_inner_string() {
        let id = CardId::new("foo-123");
        assert_eq!(id.to_string(), "foo-123");
        assert_eq!(id.as_str(), "foo-123");
    }

    #[test]
    fn it_treats_two_card_ids_with_same_value_as_equal() {
        assert_eq!(CardId::new("a"), CardId::new("a"));
    }

    #[test]
    fn it_treats_card_id_and_printing_id_as_distinct_types() {
        // Compile-time check: this would not compile if these were aliases.
        let _card: CardId = CardId::new("a");
        let _printing: PrintingId = PrintingId::new("a");
    }

    #[test]
    fn it_displays_hero_moniker() {
        assert_eq!(HeroMoniker::new("Bravo").to_string(), "Bravo");
    }

    #[test]
    fn it_uses_card_id_as_hashmap_key() {
        let mut map = std::collections::HashMap::new();
        map.insert(CardId::new("k"), 1u8);
        assert_eq!(map.get(&CardId::new("k")), Some(&1));
    }
}
