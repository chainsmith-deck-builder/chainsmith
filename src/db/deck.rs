//! Deck persistence layer.
//!
//! `*Row` structs mirror the SQL schema column-for-column. Domain conversion
//! happens in `api/decks.rs` so the DB layer stays free of domain type
//! coupling. Queries are written with `sqlx::query!`/`query_as!` for
//! compile-time checking against the committed `.sqlx` cache.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DeckRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub format: String,
    pub hero_printing_id: String,
    pub name: String,
    pub description: Option<String>,
    pub visibility: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct PoolEntryRow {
    pub deck_id: Uuid,
    pub printing_id: String,
    pub quantity: i16,
}

#[derive(Debug, Clone)]
pub struct LoadoutRow {
    pub id: Uuid,
    pub deck_id: Uuid,
    pub name: String,
    pub notes: Option<String>,
    pub ordinal: i16,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct LoadoutEntryRow {
    pub loadout_id: Uuid,
    pub printing_id: String,
    pub quantity: i16,
}

#[derive(Debug, Clone)]
pub struct LoadoutEquipmentRow {
    pub loadout_id: Uuid,
    pub slot: String,
    pub printing_id: String,
}

/// Newly-created deck data — what `create_deck` accepts. The DB assigns
/// `id`, `created_at`, `updated_at`.
#[derive(Debug, Clone)]
pub struct NewDeck<'a> {
    pub owner_id: Uuid,
    pub format: &'a str,
    pub hero_printing_id: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub visibility: &'a str,
    pub tags: &'a [String],
    pub pool: &'a [(String, i16)],
    pub loadouts: &'a [NewLoadout<'a>],
}

#[derive(Debug, Clone)]
pub struct NewLoadout<'a> {
    pub name: &'a str,
    pub notes: Option<&'a str>,
    pub ordinal: i16,
    pub deck_cards: &'a [(String, i16)],
    pub equipment: &'a [(&'a str, String)], // (slot, printing_id)
}

/// Insert a new deck plus all child rows in a single transaction.
pub async fn create_deck(pool: &PgPool, new: NewDeck<'_>) -> Result<Uuid, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let deck_id: Uuid = sqlx::query_scalar!(
        r#"
        INSERT INTO decks (
            owner_id, format, hero_printing_id, name, description, visibility, tags
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
        new.owner_id,
        new.format,
        new.hero_printing_id,
        new.name,
        new.description,
        new.visibility,
        new.tags,
    )
    .fetch_one(&mut *tx)
    .await?;

    for (printing_id, quantity) in new.pool {
        sqlx::query!(
            r#"
            INSERT INTO deck_pool_entries (deck_id, printing_id, quantity)
            VALUES ($1, $2, $3)
            "#,
            deck_id,
            printing_id,
            quantity,
        )
        .execute(&mut *tx)
        .await?;
    }

    for loadout in new.loadouts {
        let loadout_id: Uuid = sqlx::query_scalar!(
            r#"
            INSERT INTO deck_loadouts (deck_id, name, notes, ordinal)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
            deck_id,
            loadout.name,
            loadout.notes,
            loadout.ordinal,
        )
        .fetch_one(&mut *tx)
        .await?;

        for (printing_id, quantity) in loadout.deck_cards {
            sqlx::query!(
                r#"
                INSERT INTO deck_loadout_entries (loadout_id, printing_id, quantity)
                VALUES ($1, $2, $3)
                "#,
                loadout_id,
                printing_id,
                quantity,
            )
            .execute(&mut *tx)
            .await?;
        }

        for (slot, printing_id) in loadout.equipment {
            sqlx::query!(
                r#"
                INSERT INTO deck_loadout_equipment (loadout_id, slot, printing_id)
                VALUES ($1, $2, $3)
                "#,
                loadout_id,
                slot,
                printing_id,
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;
    Ok(deck_id)
}

/// All non-deleted decks owned by `owner_id`, newest first.
pub async fn list_decks_for_owner(
    pool: &PgPool,
    owner_id: Uuid,
) -> Result<Vec<DeckRow>, sqlx::Error> {
    sqlx::query_as!(
        DeckRow,
        r#"
        SELECT
            id, owner_id, format, hero_printing_id, name, description,
            visibility, tags, created_at, updated_at, deleted_at
        FROM decks
        WHERE owner_id = $1 AND deleted_at IS NULL
        ORDER BY updated_at DESC
        "#,
        owner_id,
    )
    .fetch_all(pool)
    .await
}

/// Fetch a single non-deleted deck by id. `None` when missing or soft-deleted.
pub async fn fetch_deck(pool: &PgPool, deck_id: Uuid) -> Result<Option<DeckRow>, sqlx::Error> {
    sqlx::query_as!(
        DeckRow,
        r#"
        SELECT
            id, owner_id, format, hero_printing_id, name, description,
            visibility, tags, created_at, updated_at, deleted_at
        FROM decks
        WHERE id = $1 AND deleted_at IS NULL
        "#,
        deck_id,
    )
    .fetch_optional(pool)
    .await
}

pub async fn fetch_pool_entries(
    pool: &PgPool,
    deck_id: Uuid,
) -> Result<Vec<PoolEntryRow>, sqlx::Error> {
    sqlx::query_as!(
        PoolEntryRow,
        r#"
        SELECT deck_id, printing_id, quantity
        FROM deck_pool_entries
        WHERE deck_id = $1
        "#,
        deck_id,
    )
    .fetch_all(pool)
    .await
}

pub async fn fetch_loadouts(pool: &PgPool, deck_id: Uuid) -> Result<Vec<LoadoutRow>, sqlx::Error> {
    sqlx::query_as!(
        LoadoutRow,
        r#"
        SELECT id, deck_id, name, notes, ordinal, created_at
        FROM deck_loadouts
        WHERE deck_id = $1
        ORDER BY ordinal, created_at
        "#,
        deck_id,
    )
    .fetch_all(pool)
    .await
}

pub async fn fetch_loadout_entries(
    pool: &PgPool,
    loadout_ids: &[Uuid],
) -> Result<Vec<LoadoutEntryRow>, sqlx::Error> {
    if loadout_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as!(
        LoadoutEntryRow,
        r#"
        SELECT loadout_id, printing_id, quantity
        FROM deck_loadout_entries
        WHERE loadout_id = ANY($1)
        "#,
        loadout_ids,
    )
    .fetch_all(pool)
    .await
}

pub async fn fetch_loadout_equipment(
    pool: &PgPool,
    loadout_ids: &[Uuid],
) -> Result<Vec<LoadoutEquipmentRow>, sqlx::Error> {
    if loadout_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as!(
        LoadoutEquipmentRow,
        r#"
        SELECT loadout_id, slot, printing_id
        FROM deck_loadout_equipment
        WHERE loadout_id = ANY($1)
        "#,
        loadout_ids,
    )
    .fetch_all(pool)
    .await
}

/// Soft-delete by setting `deleted_at`. Returns true if a row was affected
/// (i.e. the deck exists, is owned by `owner_id`, and was not already
/// deleted). Returns false otherwise.
pub async fn soft_delete_deck(
    pool: &PgPool,
    deck_id: Uuid,
    owner_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        UPDATE decks
        SET deleted_at = now()
        WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL
        "#,
        deck_id,
        owner_id,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}
