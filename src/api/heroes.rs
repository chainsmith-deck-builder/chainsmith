//! `GET /heroes` — hero-typed cards with hero-specific summary fields.
//!
//! Specialization of `/cards`: filters to `CardType::Hero` and surfaces
//! life/intellect/arcane/kind/essence-grants in the response so the deck-
//! builder's hero-picker UI doesn't need an N+1 round-trip to `/cards/{id}`.
//! Hero detail still goes through `GET /cards/{id}`.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::api::cards::PrintingSummary;
use crate::domain::card::{
    Card, CardType, Class, EssenceGrant, FormatStatus, HeroFacts, HeroKind, LegalitySummary,
    Printing, Talent,
};
use crate::domain::format::FormatId;
use crate::domain::ids::{CardId, PrintingId, SetCode};
use crate::error::{AppError, ErrorBody};
use crate::state::AppState;

const DEFAULT_LIMIT: u16 = 50;
const MAX_LIMIT: u16 = 200;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list_heroes))
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct HeroSearchQuery {
    /// Case-insensitive substring match on `name`.
    pub text: Option<String>,
    /// Comma-separated class names (any-match).
    pub classes: Option<String>,
    /// Comma-separated talent names (any-match).
    pub talents: Option<String>,
    /// `adult`, `young`, or `pit_fighter`. Multiple values not supported —
    /// most queries want exactly one.
    pub kind: Option<HeroKind>,
    /// Filter to heroes currently `Legal` in the given format. CC-legal
    /// heroes that have been retired to Living Legend will *not* match
    /// `format=classic_constructed` (their `cc` legality is
    /// `living_legend_retired`); they will match `format=living_legend`.
    pub format: Option<FormatId>,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeroSummary {
    pub id: CardId,
    pub name: String,
    pub kind: HeroKind,
    pub classes: Vec<Class>,
    pub talents: Vec<Talent>,
    pub life: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intellect: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arcane: Option<u8>,
    pub essence_grants: Vec<EssenceGrant>,
    pub legality_summary: LegalitySummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_printing: Option<PrintingSummary>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeroListResponse {
    pub items: Vec<HeroSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[utoipa::path(
    get,
    path = "/heroes",
    operation_id = "listHeroes",
    params(HeroSearchQuery),
    responses(
        (status = 200, description = "Heroes matching the filter, paginated", body = HeroListResponse),
        (status = 400, description = "Malformed query", body = ErrorBody),
    )
)]
async fn list_heroes(
    State(state): State<AppState>,
    Query(query): Query<HeroSearchQuery>,
) -> Result<Json<HeroListResponse>, AppError> {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let offset = query
        .cursor
        .as_deref()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    let classes = parse_csv::<Class>(query.classes.as_deref());
    let talents = parse_csv::<Talent>(query.talents.as_deref());

    let mut matches: Vec<(&Card, &HeroFacts)> = state
        .catalog
        .cards()
        .filter(|c| c.types.contains(&CardType::Hero))
        .filter_map(|c| c.hero.as_ref().map(|h| (c, h)))
        .filter(|(c, h)| matches_filters(c, h, &query, &classes, &talents))
        .collect();
    matches.sort_by(|(a, _), (b, _)| a.name.cmp(&b.name));

    let total = matches.len();
    let page: Vec<(&Card, &HeroFacts)> = matches.into_iter().skip(offset).take(limit).collect();

    let items: Vec<HeroSummary> = page
        .into_iter()
        .map(|(card, hero)| build_summary(card, hero, &state))
        .collect();

    let next_offset = offset + items.len();
    let next_cursor = if next_offset < total {
        Some(next_offset.to_string())
    } else {
        None
    };

    Ok(Json(HeroListResponse { items, next_cursor }))
}

// ---- helpers ----

fn matches_filters(
    card: &Card,
    hero: &HeroFacts,
    query: &HeroSearchQuery,
    classes: &[Class],
    talents: &[Talent],
) -> bool {
    if let Some(text) = &query.text {
        if !card.name.to_lowercase().contains(&text.to_lowercase()) {
            return false;
        }
    }
    if !classes.is_empty() && !classes.iter().any(|c| card.classes.contains(c)) {
        return false;
    }
    if !talents.is_empty() && !talents.iter().any(|t| card.talents.contains(t)) {
        return false;
    }
    if let Some(kind) = query.kind {
        if hero.kind != kind {
            return false;
        }
    }
    if let Some(format) = query.format {
        // Only `Legal` here — a retired hero's `cc` is `LivingLegendRetired`,
        // which is NOT a CC match. The same hero's `living_legend` field is
        // `Legal`, so they correctly match `format=living_legend`.
        if format_status(&card.legality_summary, format) != FormatStatus::Legal {
            return false;
        }
    }
    true
}

fn format_status(summary: &LegalitySummary, format: FormatId) -> FormatStatus {
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
        .filter_map(|t| serde_json::from_value(serde_json::Value::String(t.to_lowercase())).ok())
        .collect()
}

fn build_summary(card: &Card, hero: &HeroFacts, state: &AppState) -> HeroSummary {
    HeroSummary {
        id: card.id.clone(),
        name: card.name.clone(),
        kind: hero.kind,
        classes: card.classes.clone(),
        talents: card.talents.clone(),
        life: hero.life,
        intellect: hero.intellect,
        arcane: hero.arcane,
        essence_grants: hero.essence_grants.clone(),
        legality_summary: card.legality_summary.clone(),
        default_printing: pick_default_printing(state, &card.id),
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
        id: PrintingId::new(first.id.as_str()),
        set: SetCode::new(first.set.as_str()),
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
        let bravo = make_adult_hero(
            "hero_bravo",
            "Bravo, Star of the Show",
            vec![Class::Guardian],
            vec![],
        );
        let katsu_young = make_young_hero("hero_katsu_young", "Katsu", vec![Class::Ninja], vec![]);
        let boltyn = make_adult_hero(
            "hero_boltyn",
            "Boltyn, Breaking Dawn",
            vec![Class::Warrior],
            vec![Talent::Light],
        );
        let bolfar = make_hero(
            "hero_bolfar",
            "Bolfar, Bear Hands",
            HeroKind::PitFighter,
            vec![Class::Guardian],
            vec![],
        );
        // Non-hero card to confirm it is excluded.
        let action = make_action("c1", "Crippling Crush", 1, vec![Class::Guardian], vec![]);

        let catalog = catalog_with(
            vec![bravo, katsu_young, boltyn, bolfar, action],
            vec![
                make_printing("p_bravo", "hero_bravo"),
                make_printing("p_katsu_y", "hero_katsu_young"),
                make_printing("p_boltyn", "hero_boltyn"),
                make_printing("p_bolfar", "hero_bolfar"),
                make_printing("p_cc", "c1"),
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
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn it_lists_only_heroes_excluding_other_card_types() {
        let (status, json) = json_response(test_state(), "/heroes").await;
        assert_eq!(status, StatusCode::OK);
        let items = json["items"].as_array().unwrap();
        // 4 heroes; the action card is excluded.
        assert_eq!(items.len(), 4);
        for item in items {
            assert!(item["kind"].is_string());
            assert!(item["life"].is_number());
        }
    }

    #[tokio::test]
    async fn it_filters_by_kind_adult() {
        let (_, json) = json_response(test_state(), "/heroes?kind=adult").await;
        let items = json["items"].as_array().unwrap();
        // Bravo + Boltyn — Katsu Young and Bolfar (PitFighter) excluded.
        assert_eq!(items.len(), 2);
        let names: Vec<_> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"Bravo, Star of the Show"));
        assert!(names.contains(&"Boltyn, Breaking Dawn"));
    }

    #[tokio::test]
    async fn it_filters_by_kind_young() {
        let (_, json) = json_response(test_state(), "/heroes?kind=young").await;
        let items = json["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "Katsu");
        assert_eq!(items[0]["kind"], "young");
    }

    #[tokio::test]
    async fn it_filters_by_kind_pit_fighter() {
        let (_, json) = json_response(test_state(), "/heroes?kind=pit_fighter").await;
        let items = json["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "Bolfar, Bear Hands");
    }

    #[tokio::test]
    async fn it_filters_by_class() {
        let (_, json) = json_response(test_state(), "/heroes?classes=guardian").await;
        let items = json["items"].as_array().unwrap();
        // Bravo + Bolfar are Guardian.
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn it_filters_by_talent() {
        let (_, json) = json_response(test_state(), "/heroes?talents=light").await;
        let items = json["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "Boltyn, Breaking Dawn");
    }

    #[tokio::test]
    async fn it_includes_hero_facts_in_summary() {
        let (_, json) = json_response(test_state(), "/heroes?text=Bravo").await;
        let item = &json["items"][0];
        assert_eq!(item["name"], "Bravo, Star of the Show");
        assert_eq!(item["kind"], "adult");
        assert_eq!(item["life"], 40);
        assert_eq!(item["intellect"], 4);
        assert!(item["essenceGrants"].is_array());
        assert!(item["defaultPrinting"].is_object());
    }

    #[tokio::test]
    async fn it_paginates_with_limit_and_cursor() {
        let (_, page1) = json_response(test_state(), "/heroes?limit=2").await;
        assert_eq!(page1["items"].as_array().unwrap().len(), 2);
        let cursor = page1["nextCursor"].as_str().unwrap();
        let (_, page2) =
            json_response(test_state(), &format!("/heroes?limit=2&cursor={cursor}")).await;
        assert_eq!(page2["items"].as_array().unwrap().len(), 2);
    }
}
