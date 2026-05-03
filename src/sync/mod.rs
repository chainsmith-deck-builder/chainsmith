//! Card data sync — pulls structured card data from the upstream
//! `the-fab-cube/flesh-and-blood-cards` repo into the engine's domain types.
//!
//! Per `.claude/rules/database.md`, the sync runs in a transaction and is
//! idempotent. This module exposes the parsing/conversion logic; database
//! persistence will come in a separate slice once the schema is defined.

pub mod fab_cube;
