//! `GET /cards` and `GET /cards/{id}` — catalog browsing.
//!
//! Backed by the in-memory `Catalog` populated by sync at startup. List
//! supports a small set of filter axes (text, class/talent/type, pitch, cost
//! range, format, hero-eligibility) and cursor pagination. Detail returns
//! the full `Card` plus all of its `Printing`s.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::domain::card::{Card, CardType, Class, LegalitySummary, Printing, Rarity, Talent};
use crate::domain::format::{supertypes_match_hero, FormatId};
use crate::domain::ids::{CardId, PrintingId, SetCode};
use crate::error::{AppError, ErrorBody};
use crate::state::AppState;

const DEFAULT_LIMIT: u16 = 50;
const MAX_LIMIT: u16 = 200;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_cards))
        .routes(routes!(get_card))
}

// ---- request / response shapes ----

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct CardSearchQuery {
    /// Case-insensitive substring match on `name`.
    pub text: Option<String>,
    /// Comma-separated list of class names. A card matches if it has any
    /// listed class. Generic cards (no class) match only when the filter is
    /// omitted.
    pub classes: Option<String>,
    /// Comma-separated list of talent names. A card matches if it has any
    /// listed talent.
    pub talents: Option<String>,
    /// Comma-separated list of card type names. A card matches if it has
    /// any listed type.
    pub types: Option<String>,
    pub pitch: Option<u8>,
    pub cost_min: Option<u8>,
    pub cost_max: Option<u8>,
    /// Filter to cards that are currently `Legal` or `Restricted` in the
    /// given format (snapshot-based, not date-aware — for date-aware
    /// validation use `POST /validate`).
    pub format: Option<FormatId>,
    /// Card id of a hero. Restricts results to cards whose supertypes are a
    /// subset of the hero's effective supertypes (CR 1.1.3).
    pub legal_for_hero: Option<CardId>,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CardSummary {
    pub id: CardId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defense: Option<u8>,
    pub types: Vec<CardType>,
    pub classes: Vec<Class>,
    pub talents: Vec<Talent>,
    pub legality_summary: LegalitySummary,
    /// A representative printing for catalog tile rendering. The first one
    /// by collector number; `None` when the card has no printings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_printing: Option<PrintingSummary>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrintingSummary {
    pub id: PrintingId,
    pub set: SetCode,
    pub rarity: Rarity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CardListResponse {
    pub items: Vec<CardSummary>,
    /// Opaque cursor; pass back as `cursor` to fetch the next page. `null`
    /// when there are no more results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CardDetail {
    pub card: Card,
    pub printings: Vec<Printing>,
}

// ---- handlers ----

#[utoipa::path(
    get,
    path = "/cards",
    operation_id = "listCards",
    tags = ["Catalog"],
    params(CardSearchQuery),
    responses(
        (status = 200, description = "Cards matching the filter, paginated", body = CardListResponse),
        (status = 400, description = "Malformed query", body = ErrorBody),
    )
)]
async fn list_cards(
    State(state): State<AppState>,
    Query(query): Query<CardSearchQuery>,
) -> Result<Json<CardListResponse>, AppError> {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let offset = parse_cursor(query.cursor.as_deref()).unwrap_or(0);

    let classes = parse_csv::<Class>(query.classes.as_deref());
    let talents = parse_csv::<Talent>(query.talents.as_deref());
    let types = parse_csv::<CardType>(query.types.as_deref());

    let hero = query
        .legal_for_hero
        .as_ref()
        .and_then(|id| state.catalog.card(id));

    // Collect matching cards, sort by name for deterministic ordering.
    let mut matches: Vec<&Card> = state
        .catalog
        .cards()
        .filter(|c| matches_filters(c, &query, &classes, &talents, &types, hero))
        .collect();
    matches.sort_by(|a, b| a.name.cmp(&b.name));

    let total = matches.len();
    let page: Vec<&Card> = matches.into_iter().skip(offset).take(limit).collect();

    let items: Vec<CardSummary> = page.into_iter().map(|c| build_summary(c, &state)).collect();

    let next_offset = offset + items.len();
    let next_cursor = if next_offset < total {
        Some(next_offset.to_string())
    } else {
        None
    };

    Ok(Json(CardListResponse { items, next_cursor }))
}

#[utoipa::path(
    get,
    path = "/cards/{id}",
    operation_id = "getCard",
    tags = ["Catalog"],
    params(("id" = String, Path, description = "Card unique id")),
    responses(
        (status = 200, description = "Card with all printings", body = CardDetail),
        (status = 404, description = "Card not found", body = ErrorBody),
    )
)]
async fn get_card(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CardDetail>, AppError> {
    let card_id = CardId::new(id.clone());
    let card = state
        .catalog
        .card(&card_id)
        .ok_or_else(|| AppError::NotFound {
            resource: "card",
            id: id.clone(),
        })?
        .clone();

    let mut printings: Vec<Printing> = state
        .catalog
        .printings_for_card(&card_id)
        .cloned()
        .collect();
    printings.sort_by(|a, b| a.collector_number.cmp(&b.collector_number));

    Ok(Json(CardDetail { card, printings }))
}

// ---- helpers ----

fn matches_filters(
    card: &Card,
    query: &CardSearchQuery,
    classes: &[Class],
    talents: &[Talent],
    types: &[CardType],
    hero: Option<&Card>,
) -> bool {
    if let Some(text) = &query.text {
        let needle = text.to_lowercase();
        if !card.name.to_lowercase().contains(&needle) {
            return false;
        }
    }
    if !classes.is_empty() && !classes.iter().any(|c| card.classes.contains(c)) {
        return false;
    }
    if !talents.is_empty() && !talents.iter().any(|t| card.talents.contains(t)) {
        return false;
    }
    if !types.is_empty() && !types.iter().any(|t| card.types.contains(t)) {
        return false;
    }
    if let Some(p) = query.pitch {
        if card.pitch != Some(p) {
            return false;
        }
    }
    if let Some(min) = query.cost_min {
        match card.cost {
            Some(c) if c >= min => {}
            _ => return false,
        }
    }
    if let Some(max) = query.cost_max {
        match card.cost {
            Some(c) if c <= max => {}
            _ => return false,
        }
    }
    if let Some(format) = query.format {
        let status = format_status(&card.legality_summary, format);
        // Snapshot filter: include cards that are currently legal or
        // restricted (LL only). Banned/suspended/retired/ineligible drop out.
        use crate::domain::card::FormatStatus::*;
        if !matches!(status, Legal | Restricted) {
            return false;
        }
    }
    if let Some(hero_card) = hero {
        if !supertypes_match_hero(card, hero_card) {
            return false;
        }
    }
    true
}

fn format_status(summary: &LegalitySummary, format: FormatId) -> crate::domain::card::FormatStatus {
    match format {
        FormatId::ClassicConstructed => summary.cc,
        FormatId::Blitz => summary.blitz,
        FormatId::Commoner => summary.commoner,
        FormatId::SilverAge => summary.silver_age,
        FormatId::LivingLegend => summary.living_legend,
        FormatId::UltimatePitFight => summary.upf,
    }
}

fn parse_csv<T: serde::de::DeserializeOwned>(s: Option<&str>) -> Vec<T> {
    let Some(s) = s else {
        return Vec::new();
    };
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter_map(|t| {
            // Re-encode as JSON string then deserialize via the enum's
            // serde derive. Avoids hand-maintaining a parser per enum.
            serde_json::from_value(serde_json::Value::String(t.to_lowercase())).ok()
        })
        .collect()
}

fn parse_cursor(s: Option<&str>) -> Option<usize> {
    s.and_then(|s| s.parse().ok())
}

fn build_summary(card: &Card, state: &AppState) -> CardSummary {
    let default_printing = pick_default_printing(state, &card.id);
    CardSummary {
        id: card.id.clone(),
        name: card.name.clone(),
        pitch: card.pitch,
        cost: card.cost,
        power: card.power,
        defense: card.defense,
        types: card.types.clone(),
        classes: card.classes.clone(),
        talents: card.talents.clone(),
        legality_summary: card.legality_summary.clone(),
        default_printing,
    }
}

fn pick_default_printing(state: &AppState, card_id: &CardId) -> Option<PrintingSummary> {
    let mut printings: Vec<&Printing> = state.catalog.printings_for_card(card_id).collect();
    if printings.is_empty() {
        return None;
    }
    printings.sort_by(|a, b| a.collector_number.cmp(&b.collector_number));
    let first = printings[0];
    Some(PrintingSummary {
        id: first.id.clone(),
        set: first.set.clone(),
        rarity: first.rarity,
        image_url: first.image_url.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::api;
    use crate::domain::format::classic_constructed::ClassicConstructed;
    use crate::domain::test_support::*;

    use super::*;

    fn test_state() -> AppState {
        // Six diverse cards for filter coverage.
        let bravo = make_adult_hero(
            "hero_bravo",
            "Bravo, Star of the Show",
            vec![Class::Guardian],
            vec![],
        );
        let katsu = make_adult_hero(
            "hero_katsu",
            "Katsu, the Wanderer",
            vec![Class::Ninja],
            vec![],
        );
        let crippling = make_action(
            "crippling",
            "Crippling Crush",
            1,
            vec![Class::Guardian],
            vec![],
        );
        let sledge = make_action("sledge", "Sledge", 2, vec![Class::Guardian], vec![]);
        let bolt = make_action(
            "bolt",
            "Lightning Bolt",
            3,
            vec![Class::Wizard],
            vec![Talent::Lightning],
        );
        let generic = make_generic_action("strike", "Strike", 1);

        let catalog = catalog_with(
            vec![bravo, katsu, crippling, sledge, bolt, generic],
            vec![
                make_printing("p_bravo", "hero_bravo"),
                make_printing("p_katsu", "hero_katsu"),
                make_printing("p_crippling", "crippling"),
                make_printing("p_sledge", "sledge"),
                make_printing("p_bolt", "bolt"),
                make_printing("p_strike", "strike"),
            ],
        );
        let pool = sqlx::PgPool::connect_lazy("postgres://test:test@localhost/test")
            .expect("lazy pool URL parses");
        AppState {
            pool,
            catalog: Arc::new(catalog),
            cc_format: Arc::new(ClassicConstructed::empty()),
            auth: Arc::new(crate::auth::AuthContext {
                mode: crate::auth::AuthMode::Disabled,
            }),
        }
    }

    async fn json_response(state: AppState, uri: &str) -> (StatusCode, Value) {
        let app = api::router(state);
        let request = Request::builder()
            .uri(uri)
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    #[tokio::test]
    async fn it_lists_all_cards_when_no_filters() {
        let (status, json) = json_response(test_state(), "/cards").await;
        assert_eq!(status, StatusCode::OK);
        let items = json["items"].as_array().unwrap();
        assert_eq!(items.len(), 6);
        // Sorted by name; first should be "Bravo, Star of the Show".
        assert_eq!(items[0]["name"], "Bravo, Star of the Show");
    }

    #[tokio::test]
    async fn it_filters_by_text_substring_case_insensitive() {
        let (status, json) = json_response(test_state(), "/cards?text=cRiPP").await;
        assert_eq!(status, StatusCode::OK);
        let items = json["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "Crippling Crush");
    }

    #[tokio::test]
    async fn it_filters_by_classes() {
        let (_, json) = json_response(test_state(), "/cards?classes=guardian").await;
        let items = json["items"].as_array().unwrap();
        // Bravo (hero), Crippling Crush, Sledge — Strike is generic, Bolt is Wizard.
        assert_eq!(items.len(), 3);
        let names: Vec<_> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"Bravo, Star of the Show"));
        assert!(names.contains(&"Crippling Crush"));
        assert!(names.contains(&"Sledge"));
    }

    #[tokio::test]
    async fn it_filters_by_talent() {
        let (_, json) = json_response(test_state(), "/cards?talents=lightning").await;
        let items = json["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "Lightning Bolt");
    }

    #[tokio::test]
    async fn it_filters_by_pitch() {
        let (_, json) = json_response(test_state(), "/cards?pitch=2").await;
        let items = json["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "Sledge");
    }

    #[tokio::test]
    async fn it_filters_by_legal_for_hero() {
        let (_, json) = json_response(test_state(), "/cards?legalForHero=hero_katsu").await;
        let items = json["items"].as_array().unwrap();
        // Katsu is Ninja, so Guardian/Wizard cards are excluded; Strike (generic) and Katsu (the hero) match.
        let names: Vec<_> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"Strike"));
        assert!(!names.contains(&"Crippling Crush"));
        assert!(!names.contains(&"Lightning Bolt"));
    }

    #[tokio::test]
    async fn it_paginates_with_limit_and_cursor() {
        // First page: 2 of 6.
        let (_, page1) = json_response(test_state(), "/cards?limit=2").await;
        let items1 = page1["items"].as_array().unwrap();
        assert_eq!(items1.len(), 2);
        let cursor = page1["nextCursor"].as_str().unwrap();

        // Second page: 2 more.
        let url2 = format!("/cards?limit=2&cursor={cursor}");
        let (_, page2) = json_response(test_state(), &url2).await;
        let items2 = page2["items"].as_array().unwrap();
        assert_eq!(items2.len(), 2);
        // No overlap between pages.
        let names1: Vec<_> = items1.iter().map(|i| i["name"].as_str().unwrap()).collect();
        let names2: Vec<_> = items2.iter().map(|i| i["name"].as_str().unwrap()).collect();
        for n in &names1 {
            assert!(!names2.contains(n), "page2 contains {n} from page1");
        }
    }

    #[tokio::test]
    async fn it_omits_next_cursor_on_last_page() {
        let (_, page) = json_response(test_state(), "/cards?limit=10").await;
        // 6 total cards, fits in one page → no nextCursor.
        assert!(page.get("nextCursor").is_none() || page["nextCursor"].is_null());
    }

    #[tokio::test]
    async fn it_includes_default_printing_in_summary() {
        let (_, json) = json_response(test_state(), "/cards?text=Strike").await;
        let item = &json["items"][0];
        assert!(item["defaultPrinting"].is_object());
        assert_eq!(item["defaultPrinting"]["id"], "p_strike");
        assert_eq!(item["defaultPrinting"]["set"], "WTR");
    }

    #[tokio::test]
    async fn it_returns_card_detail_with_printings() {
        let (status, json) = json_response(test_state(), "/cards/crippling").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["card"]["name"], "Crippling Crush");
        assert!(!json["printings"].as_array().unwrap().is_empty());
        assert_eq!(json["printings"][0]["id"], "p_crippling");
    }

    #[tokio::test]
    async fn it_returns_404_for_unknown_card() {
        let (status, json) = json_response(test_state(), "/cards/does_not_exist").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"]["code"], "not_found");
    }
}
