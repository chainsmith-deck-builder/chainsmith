//! Pure validation engine — no IO, no clock reads, no env access.
//!
//! Per `.claude/rules/rust.md`: "The domain module is pure: no IO, no clock
//! reads, no env access. The validation engine in `domain/` must be testable
//! without a database." All time inputs flow in as `chrono::NaiveDate`
//! parameters; nothing here calls `Utc::now()` or anything similar.

pub mod card;
pub mod catalog;
pub mod deck;
pub mod format;
pub mod ids;
pub mod special_rules;
pub mod violation;

#[cfg(test)]
pub(crate) mod test_support;
