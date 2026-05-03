//! `POST/GET/PUT/PATCH/DELETE /decks` — saved deck CRUD.
//!
//! All endpoints require a valid Supabase JWT (`Authorization: Bearer ...`).
//! Decks are scoped to the authenticated user via `owner_id`. Soft delete
//! (`deleted_at`) preserves history; deck listing and lookup filter it out.
//!
//! ## Optimistic concurrency
//!
//! `PUT` and `PATCH` use the standard `If-Match` header for optimistic
//! concurrency control. `GET`/`POST`/`PUT`/`PATCH` responses carry an `ETag`
//! header derived from the deck's `updated_at`; clients echo that value via
//! `If-Match` on the next mutation. A mismatch returns 412; missing the
//! header returns 428. The ETag value is opaque — clients should treat it as
//! a literal string, not parse it.

use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    http::{
        header::{ETAG, IF_MATCH},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::db::deck as db;
use crate::db::deck::UpdateOutcome;
use crate::domain::deck::{Deck, EquipmentLoadout, Loadout, LoadoutEntry, PoolEntry};
use crate::domain::format::FormatId;
use crate::domain::ids::PrintingId;
use crate::error::{AppError, ErrorBody};
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_deck))
        .routes(routes!(list_decks))
        .routes(routes!(get_deck))
        .routes(routes!(replace_deck))
        .routes(routes!(update_deck_metadata))
        .routes(routes!(delete_deck))
}

// ---- request / response shapes ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Private,
    Unlisted,
    Public,
}

impl Visibility {
    fn as_str(&self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Unlisted => "unlisted",
            Visibility::Public => "public",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "unlisted" => Visibility::Unlisted,
            "public" => Visibility::Public,
            _ => Visibility::Private,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDeckRequest {
    /// Display name shown to the user (and to other users for public decks).
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The pool/hero/format/loadouts to save.
    pub deck: Deck,
}

/// Body for `PUT /decks/{id}` — replaces the entire deck (metadata + pool +
/// loadouts) atomically. Use this for normal "save my deck" flows where the
/// client edits in memory and writes the whole thing back.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplaceDeckRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub deck: Deck,
}

/// Body for `PATCH /decks/{id}` — updates a subset of metadata fields. Any
/// field omitted is left unchanged. To clear `description` back to absent,
/// use `PUT` (PATCH cannot represent the difference between "not changing"
/// and "setting to null" without doubling the field's nesting).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchDeckRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeckResponse {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub visibility: Visibility,
    pub tags: Vec<String>,
    pub deck: Deck,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeckSummary {
    pub id: Uuid,
    pub name: String,
    pub format: FormatId,
    pub hero: PrintingId,
    pub visibility: Visibility,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeckListResponse {
    pub items: Vec<DeckSummary>,
}

// ---- handlers ----

#[utoipa::path(
    post,
    path = "/decks",
    operation_id = "createDeck",
    tags = ["Decks"],
    request_body = CreateDeckRequest,
    responses(
        (status = 201, description = "Deck created", body = DeckResponse),
        (status = 400, description = "Malformed request", body = ErrorBody),
        (status = 401, description = "Missing or invalid Authorization header", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
async fn create_deck(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(req): Json<CreateDeckRequest>,
) -> Result<impl IntoResponse, AppError> {
    let visibility = req.visibility.unwrap_or(Visibility::Private);
    let format_str = format_id_to_str(req.deck.format);

    let pool: Vec<(String, i16)> = req
        .deck
        .pool
        .iter()
        .map(|e| (e.printing.as_str().to_string(), e.quantity as i16))
        .collect();

    let loadouts_owned: Vec<OwnedLoadout> = req
        .deck
        .loadouts
        .iter()
        .enumerate()
        .map(|(i, l)| OwnedLoadout::from_domain(l, i as i16))
        .collect();
    let loadouts_borrowed: Vec<db::NewLoadout<'_>> =
        loadouts_owned.iter().map(OwnedLoadout::borrow).collect();

    let new = db::NewDeck {
        owner_id: user.id,
        format: format_str,
        hero_printing_id: req.deck.hero.as_str(),
        name: &req.name,
        description: req.description.as_deref(),
        visibility: visibility.as_str(),
        tags: &req.tags,
        pool: &pool,
        loadouts: &loadouts_borrowed,
    };

    let id = db::create_deck(&state.pool, new).await?;

    // Fetch back the complete deck so we return the exact server view
    // (with timestamps and any DB-set defaults).
    let response = build_deck_response(&state, id).await?;
    Ok((StatusCode::CREATED, etag_headers(&response), Json(response)))
}

#[utoipa::path(
    get,
    path = "/decks",
    operation_id = "listDecks",
    tags = ["Decks"],
    responses(
        (status = 200, description = "Decks owned by the authenticated user", body = DeckListResponse),
        (status = 401, description = "Missing or invalid Authorization header", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
async fn list_decks(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<Json<DeckListResponse>, AppError> {
    let rows = db::list_decks_for_owner(&state.pool, user.id).await?;
    let items: Vec<DeckSummary> = rows.into_iter().map(deck_summary_from_row).collect();
    Ok(Json(DeckListResponse { items }))
}

#[utoipa::path(
    get,
    path = "/decks/{id}",
    operation_id = "getDeck",
    tags = ["Decks"],
    params(("id" = Uuid, Path, description = "Deck id")),
    responses(
        (status = 200, description = "Full deck", body = DeckResponse),
        (status = 401, description = "Missing or invalid Authorization header", body = ErrorBody),
        (status = 404, description = "Deck not found or not visible to caller", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
async fn get_deck(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let row = db::fetch_deck(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound {
            resource: "deck",
            id: id.to_string(),
        })?;

    if !can_view(&row, user.id) {
        // Don't leak existence — return 404 not 403.
        return Err(AppError::NotFound {
            resource: "deck",
            id: id.to_string(),
        });
    }

    let response = build_deck_response_from_row(&state, row).await?;
    Ok((etag_headers(&response), Json(response)))
}

#[utoipa::path(
    put,
    path = "/decks/{id}",
    operation_id = "replaceDeck",
    params(
        ("id" = Uuid, Path, description = "Deck id"),
        ("If-Match" = String, Header, description = "ETag from a prior GET/POST/PUT/PATCH response. Required."),
    ),
    request_body = ReplaceDeckRequest,
    responses(
        (status = 200, description = "Deck replaced", body = DeckResponse),
        (status = 401, description = "Missing or invalid Authorization header", body = ErrorBody),
        (status = 404, description = "Deck not found or not owned by caller", body = ErrorBody),
        (status = 412, description = "If-Match did not match the current deck version", body = ErrorBody),
        (status = 428, description = "If-Match header was not provided", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
async fn replace_deck(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<ReplaceDeckRequest>,
) -> Result<impl IntoResponse, AppError> {
    let expected = parse_if_match(&headers)?;
    let visibility = req.visibility.unwrap_or(Visibility::Private);
    let format_str = format_id_to_str(req.deck.format);

    let pool: Vec<(String, i16)> = req
        .deck
        .pool
        .iter()
        .map(|e| (e.printing.as_str().to_string(), e.quantity as i16))
        .collect();

    let loadouts_owned: Vec<OwnedLoadout> = req
        .deck
        .loadouts
        .iter()
        .enumerate()
        .map(|(i, l)| OwnedLoadout::from_domain(l, i as i16))
        .collect();
    let loadouts_borrowed: Vec<db::NewLoadout<'_>> =
        loadouts_owned.iter().map(OwnedLoadout::borrow).collect();

    let update = db::ReplaceDeck {
        format: format_str,
        hero_printing_id: req.deck.hero.as_str(),
        name: &req.name,
        description: req.description.as_deref(),
        visibility: visibility.as_str(),
        tags: &req.tags,
        pool: &pool,
        loadouts: &loadouts_borrowed,
    };

    match db::replace_deck(&state.pool, id, user.id, expected, update).await? {
        UpdateOutcome::Updated(_) => {
            // Re-fetch the full body so the response includes the rebuilt
            // pool/loadouts in the same shape as `GET /decks/{id}`.
            let response = build_deck_response(&state, id).await?;
            Ok((etag_headers(&response), Json(response)))
        }
        UpdateOutcome::NotFound => Err(AppError::NotFound {
            resource: "deck",
            id: id.to_string(),
        }),
        UpdateOutcome::PreconditionFailed => Err(AppError::Conflict),
    }
}

#[utoipa::path(
    patch,
    path = "/decks/{id}",
    operation_id = "patchDeck",
    params(
        ("id" = Uuid, Path, description = "Deck id"),
        ("If-Match" = String, Header, description = "ETag from a prior GET/POST/PUT/PATCH response. Required."),
    ),
    request_body = PatchDeckRequest,
    responses(
        (status = 200, description = "Metadata updated", body = DeckResponse),
        (status = 401, description = "Missing or invalid Authorization header", body = ErrorBody),
        (status = 404, description = "Deck not found or not owned by caller", body = ErrorBody),
        (status = 412, description = "If-Match did not match the current deck version", body = ErrorBody),
        (status = 428, description = "If-Match header was not provided", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
async fn update_deck_metadata(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<PatchDeckRequest>,
) -> Result<impl IntoResponse, AppError> {
    let expected = parse_if_match(&headers)?;

    // An empty patch is a no-op; the precondition is still checked so the
    // client learns whether its view is current.
    let visibility_str = req.visibility.map(|v| v.as_str());
    let tags_slice = req.tags.as_deref();
    let patch = db::PatchDeckMetadata {
        name: req.name.as_deref(),
        description: req.description.as_deref(),
        visibility: visibility_str,
        tags: tags_slice,
    };

    match db::update_deck_metadata(&state.pool, id, user.id, expected, patch).await? {
        UpdateOutcome::Updated(_) => {
            let response = build_deck_response(&state, id).await?;
            Ok((etag_headers(&response), Json(response)))
        }
        UpdateOutcome::NotFound => Err(AppError::NotFound {
            resource: "deck",
            id: id.to_string(),
        }),
        UpdateOutcome::PreconditionFailed => Err(AppError::Conflict),
    }
}

#[utoipa::path(
    delete,
    path = "/decks/{id}",
    operation_id = "deleteDeck",
    tags = ["Decks"],
    params(("id" = Uuid, Path, description = "Deck id")),
    responses(
        (status = 204, description = "Deleted (soft)"),
        (status = 401, description = "Missing or invalid Authorization header", body = ErrorBody),
        (status = 404, description = "Deck not found or not owned by caller", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
async fn delete_deck(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let deleted = db::soft_delete_deck(&state.pool, id, user.id).await?;
    if !deleted {
        return Err(AppError::NotFound {
            resource: "deck",
            id: id.to_string(),
        });
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---- helpers ----

/// Build the `ETag` header for a deck response. Format is a quoted i64
/// microseconds-since-epoch — opaque to clients, but stable across
/// serialization round-trips since both Postgres and chrono store
/// microsecond precision.
fn etag_headers(response: &DeckResponse) -> [(axum::http::HeaderName, HeaderValue); 1] {
    let value = format!("\"{}\"", response.updated_at.timestamp_micros());
    // `value` is constructed from a digit string and ASCII quotes, so the
    // header conversion cannot fail.
    let header_value =
        HeaderValue::from_str(&value).expect("etag is ASCII digits surrounded by ASCII quotes");
    [(ETAG, header_value)]
}

/// Parse the `If-Match` header into a `DateTime<Utc>` matching what the DB
/// layer stores. Surrounding quotes are tolerated so clients can echo the
/// `ETag` value verbatim. Any malformed input maps to 428 — we treat
/// "unparseable" the same as "missing" since either way the client needs to
/// re-read and try again.
fn parse_if_match(headers: &HeaderMap) -> Result<DateTime<Utc>, AppError> {
    let raw = headers
        .get(IF_MATCH)
        .ok_or(AppError::PreconditionRequired)?
        .to_str()
        .map_err(|_| AppError::PreconditionRequired)?;
    let trimmed = raw.trim().trim_matches('"');
    let micros: i64 = trimmed
        .parse()
        .map_err(|_| AppError::PreconditionRequired)?;
    DateTime::<Utc>::from_timestamp_micros(micros).ok_or(AppError::PreconditionRequired)
}

fn can_view(row: &db::DeckRow, viewer: Uuid) -> bool {
    if row.owner_id == viewer {
        return true;
    }
    matches!(row.visibility.as_str(), "public" | "unlisted")
}

fn deck_summary_from_row(row: db::DeckRow) -> DeckSummary {
    DeckSummary {
        id: row.id,
        name: row.name,
        format: format_id_from_str(&row.format),
        hero: PrintingId::new(row.hero_printing_id),
        visibility: Visibility::from_str(&row.visibility),
        tags: row.tags,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

async fn build_deck_response(state: &AppState, id: Uuid) -> Result<DeckResponse, AppError> {
    let row = db::fetch_deck(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound {
            resource: "deck",
            id: id.to_string(),
        })?;
    build_deck_response_from_row(state, row).await
}

async fn build_deck_response_from_row(
    state: &AppState,
    row: db::DeckRow,
) -> Result<DeckResponse, AppError> {
    let pool = db::fetch_pool_entries(&state.pool, row.id).await?;
    let loadouts = db::fetch_loadouts(&state.pool, row.id).await?;
    let loadout_ids: Vec<Uuid> = loadouts.iter().map(|l| l.id).collect();
    let entries = db::fetch_loadout_entries(&state.pool, &loadout_ids).await?;
    let equipment = db::fetch_loadout_equipment(&state.pool, &loadout_ids).await?;

    let mut entries_by_loadout: HashMap<Uuid, Vec<LoadoutEntry>> = HashMap::new();
    for e in entries {
        entries_by_loadout
            .entry(e.loadout_id)
            .or_default()
            .push(LoadoutEntry {
                printing: PrintingId::new(e.printing_id),
                quantity: e.quantity as u8,
            });
    }

    let mut equipment_by_loadout: HashMap<Uuid, EquipmentLoadout> = HashMap::new();
    for eq in equipment {
        let entry = equipment_by_loadout.entry(eq.loadout_id).or_default();
        let printing_id = PrintingId::new(eq.printing_id);
        match eq.slot.as_str() {
            "head" => entry.head = Some(printing_id),
            "chest" => entry.chest = Some(printing_id),
            "arms" => entry.arms = Some(printing_id),
            "legs" => entry.legs = Some(printing_id),
            "main_hand" => entry.main_hand = Some(printing_id),
            "off_hand" => entry.off_hand = Some(printing_id),
            // Unexpected slot string — should be impossible due to the
            // CHECK constraint, but if it happens we silently drop rather
            // than panic.
            _ => {}
        }
    }

    let domain_loadouts: Vec<Loadout> = loadouts
        .into_iter()
        .map(|l| Loadout {
            name: l.name,
            deck_cards: entries_by_loadout.remove(&l.id).unwrap_or_default(),
            equipment: equipment_by_loadout.remove(&l.id).unwrap_or_default(),
        })
        .collect();

    let deck = Deck {
        format: format_id_from_str(&row.format),
        hero: PrintingId::new(row.hero_printing_id),
        pool: pool
            .into_iter()
            .map(|p| PoolEntry {
                printing: PrintingId::new(p.printing_id),
                quantity: p.quantity as u8,
            })
            .collect(),
        loadouts: domain_loadouts,
    };

    Ok(DeckResponse {
        id: row.id,
        owner_id: row.owner_id,
        name: row.name,
        description: row.description,
        visibility: Visibility::from_str(&row.visibility),
        tags: row.tags,
        deck,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn format_id_to_str(id: FormatId) -> &'static str {
    match id {
        FormatId::ClassicConstructed => "classic_constructed",
        FormatId::Blitz => "blitz",
        FormatId::Commoner => "commoner",
        FormatId::SilverAge => "silver_age",
        FormatId::LivingLegend => "living_legend",
        FormatId::UltimatePitFight => "ultimate_pit_fight",
    }
}

fn format_id_from_str(s: &str) -> FormatId {
    match s {
        "blitz" => FormatId::Blitz,
        "commoner" => FormatId::Commoner,
        "silver_age" => FormatId::SilverAge,
        "living_legend" => FormatId::LivingLegend,
        "ultimate_pit_fight" => FormatId::UltimatePitFight,
        _ => FormatId::ClassicConstructed,
    }
}

/// Owned-string variant of `db::NewLoadout` so we can build it from a
/// borrowed domain `Loadout` and pass borrowed slices into the DB layer
/// without lifetime gymnastics on the request body.
struct OwnedLoadout {
    name: String,
    notes: Option<String>,
    ordinal: i16,
    deck_cards: Vec<(String, i16)>,
    equipment: Vec<(&'static str, String)>,
}

impl OwnedLoadout {
    fn from_domain(l: &Loadout, ordinal: i16) -> Self {
        let mut equipment: Vec<(&'static str, String)> = Vec::new();
        let eq = &l.equipment;
        if let Some(p) = &eq.head {
            equipment.push(("head", p.as_str().to_string()));
        }
        if let Some(p) = &eq.chest {
            equipment.push(("chest", p.as_str().to_string()));
        }
        if let Some(p) = &eq.arms {
            equipment.push(("arms", p.as_str().to_string()));
        }
        if let Some(p) = &eq.legs {
            equipment.push(("legs", p.as_str().to_string()));
        }
        if let Some(p) = &eq.main_hand {
            equipment.push(("main_hand", p.as_str().to_string()));
        }
        if let Some(p) = &eq.off_hand {
            equipment.push(("off_hand", p.as_str().to_string()));
        }
        Self {
            name: l.name.clone(),
            notes: None,
            ordinal,
            deck_cards: l
                .deck_cards
                .iter()
                .map(|e| (e.printing.as_str().to_string(), e.quantity as i16))
                .collect(),
            equipment,
        }
    }

    fn borrow(&self) -> db::NewLoadout<'_> {
        db::NewLoadout {
            name: &self.name,
            notes: self.notes.as_deref(),
            ordinal: self.ordinal,
            deck_cards: &self.deck_cards,
            equipment: &self.equipment,
        }
    }
}
