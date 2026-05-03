//! `POST /validate` — stateless deck validation.
//!
//! The caller posts a `Deck` plus an optional date; the handler resolves the
//! format, runs the engine, and returns the result. No persistent state is
//! consulted besides the in-memory `Catalog` populated by sync at startup.

use axum::{extract::State, Json};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::domain::deck::Deck;
use crate::domain::format::{validate, Format, FormatId};
use crate::domain::violation::Violation;
use crate::error::{AppError, ErrorBody};
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(validate_deck))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateRequest {
    pub deck: Deck,
    /// Validation date. Used to evaluate time-evolving rules (banned list,
    /// Living Legend retirement). Defaults to today UTC if omitted.
    #[serde(default)]
    pub date: Option<NaiveDate>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResponse {
    /// True iff `violations` is empty.
    pub valid: bool,
    pub format: FormatId,
    pub date: NaiveDate,
    pub violations: Vec<Violation>,
}

#[utoipa::path(
    post,
    path = "/validate",
    operation_id = "validateDeck",
    tags = ["Validation"],
    request_body = ValidateRequest,
    responses(
        (status = 200, description = "Validation completed; check `valid` and `violations`", body = ValidationResponse),
        (status = 400, description = "Malformed request or unsupported format", body = ErrorBody),
        (status = 500, description = "Internal server error", body = ErrorBody),
    )
)]
async fn validate_deck(
    State(state): State<AppState>,
    Json(req): Json<ValidateRequest>,
) -> Result<Json<ValidationResponse>, AppError> {
    let date = req.date.unwrap_or_else(|| Utc::now().date_naive());
    let format_id = req.deck.format;

    let format: &dyn Format = match format_id {
        FormatId::ClassicConstructed => state.cc_format.as_ref(),
        other => return Err(AppError::UnsupportedFormat(other)),
    };

    let violations = match validate(&req.deck, format, date, state.catalog.as_ref()) {
        Ok(()) => Vec::new(),
        Err(vs) => vs,
    };

    Ok(Json(ValidationResponse {
        valid: violations.is_empty(),
        format: format_id,
        date,
        violations,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::api;
    use crate::domain::card::Class;
    use crate::domain::format::classic_constructed::ClassicConstructed;
    use crate::domain::test_support::*;
    use crate::domain::violation::ViolationCode;

    use super::*;

    fn test_state() -> AppState {
        let hero = make_adult_hero("h", "Bravo", vec![Class::Guardian], vec![]);
        let action = make_action("c1", "Crippling Crush", 1, vec![Class::Guardian], vec![]);
        let off_class = make_action("c2", "Wizard Bolt", 1, vec![Class::Wizard], vec![]);
        let catalog = catalog_with(
            vec![hero, action, off_class],
            vec![
                make_printing("hero_p", "h"),
                make_printing("p1", "c1"),
                make_printing("p2", "c2"),
            ],
        );
        let cc_format = ClassicConstructed::empty();
        // Lazy pool — never connects because /validate doesn't query the DB.
        let pool = sqlx::PgPool::connect_lazy("postgres://test:test@localhost/test")
            .expect("lazy pool URL parses");
        AppState {
            pool,
            catalog: Arc::new(catalog),
            cc_format: Arc::new(cc_format),
            auth: Arc::new(crate::auth::AuthContext {
                mode: crate::auth::AuthMode::Disabled,
            }),
        }
    }

    fn deck_json(format: &str, hero: &str, pool: Vec<(&str, u8)>) -> Value {
        let pool_arr: Vec<Value> = pool
            .into_iter()
            .map(|(p, q)| json!({ "printing": p, "quantity": q }))
            .collect();
        json!({
            "format": format,
            "hero": hero,
            "pool": pool_arr,
            "loadouts": [],
        })
    }

    async fn body_to_json(body: Body) -> Value {
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn it_returns_valid_true_for_legal_deck() {
        let state = test_state();
        let req = ValidateRequest {
            deck: serde_json::from_value(deck_json(
                "classic_constructed",
                "hero_p",
                vec![("p1", 1)],
            ))
            .unwrap(),
            date: Some(release_on(2026, 5, 2)),
        };
        let response = validate_deck(State(state), Json(req)).await.unwrap().0;
        assert!(response.valid);
        assert!(response.violations.is_empty());
        assert_eq!(response.format, FormatId::ClassicConstructed);
        assert_eq!(response.date, release_on(2026, 5, 2));
    }

    #[tokio::test]
    async fn it_returns_violations_for_off_class_card() {
        let state = test_state();
        // p2 is Wizard, hero is Guardian — supertype mismatch.
        let req = ValidateRequest {
            deck: serde_json::from_value(deck_json(
                "classic_constructed",
                "hero_p",
                vec![("p2", 1)],
            ))
            .unwrap(),
            date: None,
        };
        let response = validate_deck(State(state), Json(req)).await.unwrap().0;
        assert!(!response.valid);
        assert!(response
            .violations
            .iter()
            .any(|v| v.code == ViolationCode::SupertypeMismatch));
    }

    #[tokio::test]
    async fn it_returns_unsupported_format_for_blitz() {
        let state = test_state();
        let req = ValidateRequest {
            deck: serde_json::from_value(deck_json("blitz", "hero_p", vec![])).unwrap(),
            date: None,
        };
        let err = validate_deck(State(state), Json(req)).await.unwrap_err();
        match err {
            AppError::UnsupportedFormat(f) => assert_eq!(f, FormatId::Blitz),
            other => panic!("expected UnsupportedFormat, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn it_defaults_date_to_today_when_omitted() {
        let state = test_state();
        let req = ValidateRequest {
            deck: serde_json::from_value(deck_json(
                "classic_constructed",
                "hero_p",
                vec![("p1", 1)],
            ))
            .unwrap(),
            date: None,
        };
        let response = validate_deck(State(state), Json(req)).await.unwrap().0;
        // The date should be today (within a day for clock skew tolerance).
        let today = Utc::now().date_naive();
        let diff = (response.date - today).num_days().abs();
        assert!(
            diff <= 1,
            "expected today, got {} (diff {diff})",
            response.date
        );
    }

    // ---- end-to-end through the router ----

    #[tokio::test]
    async fn it_serves_validate_endpoint_with_camelcase_response_shape() {
        let state = test_state();
        let app = api::router(state);
        let body = serde_json::to_vec(&json!({
            "deck": deck_json("classic_constructed", "hero_p", vec![("p1", 1)]),
            "date": "2026-05-02",
        }))
        .unwrap();
        let request = Request::builder()
            .uri("/validate")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["valid"], true);
        assert_eq!(json["format"], "classic_constructed");
        assert_eq!(json["date"], "2026-05-02");
        assert!(json["violations"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn it_returns_400_with_error_body_for_unsupported_format() {
        let state = test_state();
        let app = api::router(state);
        let body = serde_json::to_vec(&json!({
            "deck": deck_json("blitz", "hero_p", vec![]),
        }))
        .unwrap();
        let request = Request::builder()
            .uri("/validate")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["error"]["code"], "unsupported_format");
    }
}
