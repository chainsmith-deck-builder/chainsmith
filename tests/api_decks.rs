//! HTTP-level integration tests for `/decks`.
//!
//! Builds the full Axum router with a test `AppState` (real Postgres pool
//! from `sqlx::test`, in-memory empty catalog, dev-secret auth) and drives
//! requests through `tower::ServiceExt::oneshot`. Verifies wire shapes,
//! status codes, and auth behavior — complements the engine-level
//! validator tests and the DB-layer tests in `db_deck.rs`.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chainsmith::auth::{AuthContext, AuthMode};
use chainsmith::domain::catalog::Catalog;
use chainsmith::domain::format::classic_constructed::ClassicConstructed;
use chainsmith::state::AppState;
use chainsmith::{api, db};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const TEST_ISSUER: &str = "https://test.example/auth";
const TEST_AUDIENCE: &str = "authenticated";
const TEST_SECRET: &str = "test-secret-do-not-ship";

fn build_state(pool: PgPool) -> AppState {
    AppState {
        pool,
        catalog: Arc::new(Catalog::new()),
        cc_format: Arc::new(ClassicConstructed::empty()),
        auth: Arc::new(AuthContext {
            mode: AuthMode::DevSecret {
                secret: Arc::new(TEST_SECRET.into()),
                issuer: TEST_ISSUER.into(),
                audience: TEST_AUDIENCE.into(),
            },
        }),
    }
}

fn make_token(sub: Uuid) -> String {
    let exp = (Utc::now() + Duration::hours(1)).timestamp() as usize;
    let claims = json!({
        "sub": sub.to_string(),
        "iss": TEST_ISSUER,
        "aud": TEST_AUDIENCE,
        "exp": exp,
    });
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
    )
    .unwrap()
}

async fn body_to_json(body: Body) -> Value {
    let bytes = to_bytes(body, usize::MAX).await.unwrap();
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(&bytes).unwrap()
}

fn deck_payload() -> Value {
    json!({
        "name": "My Test Deck",
        "deck": {
            "format": "classic_constructed",
            "hero": "hero_printing_id",
            "pool": [
                {"printing": "p1", "quantity": 3},
                {"printing": "p2", "quantity": 1},
            ],
            "loadouts": [
                {
                    "name": "Main",
                    "deckCards": [{"printing": "p1", "quantity": 3}],
                    "equipment": {
                        "mainHand": "weapon_p"
                    }
                }
            ],
        },
        "tags": ["aggro", "wip"]
    })
}

#[sqlx::test]
async fn it_creates_a_deck_and_returns_201_with_full_body(pool: PgPool) {
    let user_id = Uuid::new_v4();
    let token = make_token(user_id);
    let app = api::router(build_state(pool));

    let request = Request::builder()
        .uri("/decks")
        .method("POST")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&deck_payload()).unwrap()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["name"], "My Test Deck");
    assert_eq!(json["ownerId"], user_id.to_string());
    assert_eq!(json["visibility"], "private");
    assert_eq!(json["tags"].as_array().unwrap().len(), 2);
    assert_eq!(json["deck"]["format"], "classic_constructed");
    assert_eq!(json["deck"]["hero"], "hero_printing_id");
    assert_eq!(json["deck"]["pool"].as_array().unwrap().len(), 2);
    assert_eq!(json["deck"]["loadouts"].as_array().unwrap().len(), 1);
    assert_eq!(json["deck"]["loadouts"][0]["name"], "Main");
    assert_eq!(
        json["deck"]["loadouts"][0]["equipment"]["mainHand"],
        "weapon_p"
    );
}

#[sqlx::test]
async fn it_lists_only_authenticated_users_decks(pool: PgPool) {
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    // Insert one deck for Bob directly via the DB layer so we don't rely on
    // his JWT.
    db::deck::create_deck(
        &pool,
        db::deck::NewDeck {
            owner_id: bob,
            format: "classic_constructed",
            hero_printing_id: "h",
            name: "Bob's deck",
            description: None,
            visibility: "private",
            tags: &[],
            pool: &[],
            loadouts: &[],
        },
    )
    .await
    .unwrap();

    let app = api::router(build_state(pool));

    // Alice creates a deck via the API.
    let alice_token = make_token(alice);
    let request = Request::builder()
        .uri("/decks")
        .method("POST")
        .header("authorization", format!("Bearer {alice_token}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&deck_payload()).unwrap()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Alice lists — only her deck.
    let request = Request::builder()
        .uri("/decks")
        .method("GET")
        .header("authorization", format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response.into_body()).await;
    let items = json["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "My Test Deck");
}

#[sqlx::test]
async fn it_rejects_request_without_authorization_header(pool: PgPool) {
    let app = api::router(build_state(pool));
    let request = Request::builder()
        .uri("/decks")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"]["code"], "unauthorized");
}

#[sqlx::test]
async fn it_rejects_request_with_invalid_token(pool: PgPool) {
    let app = api::router(build_state(pool));
    let request = Request::builder()
        .uri("/decks")
        .method("GET")
        .header("authorization", "Bearer not-a-real-jwt")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn it_returns_404_when_fetching_other_users_private_deck(pool: PgPool) {
    let owner = Uuid::new_v4();
    let other_user = Uuid::new_v4();

    let id = db::deck::create_deck(
        &pool,
        db::deck::NewDeck {
            owner_id: owner,
            format: "classic_constructed",
            hero_printing_id: "h",
            name: "Private",
            description: None,
            visibility: "private",
            tags: &[],
            pool: &[],
            loadouts: &[],
        },
    )
    .await
    .unwrap();

    let app = api::router(build_state(pool));
    let token = make_token(other_user);
    let request = Request::builder()
        .uri(format!("/decks/{id}"))
        .method("GET")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn it_allows_other_users_to_view_public_decks(pool: PgPool) {
    let owner = Uuid::new_v4();
    let viewer = Uuid::new_v4();

    let id = db::deck::create_deck(
        &pool,
        db::deck::NewDeck {
            owner_id: owner,
            format: "classic_constructed",
            hero_printing_id: "h",
            name: "Shared",
            description: None,
            visibility: "public",
            tags: &[],
            pool: &[],
            loadouts: &[],
        },
    )
    .await
    .unwrap();

    let app = api::router(build_state(pool));
    let token = make_token(viewer);
    let request = Request::builder()
        .uri(format!("/decks/{id}"))
        .method("GET")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[sqlx::test]
async fn it_soft_deletes_owners_deck_and_returns_204(pool: PgPool) {
    let owner = Uuid::new_v4();
    let id = db::deck::create_deck(
        &pool,
        db::deck::NewDeck {
            owner_id: owner,
            format: "classic_constructed",
            hero_printing_id: "h",
            name: "To Delete",
            description: None,
            visibility: "private",
            tags: &[],
            pool: &[],
            loadouts: &[],
        },
    )
    .await
    .unwrap();

    let token = make_token(owner);
    let app = api::router(build_state(pool));
    let request = Request::builder()
        .uri(format!("/decks/{id}"))
        .method("DELETE")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Subsequent GET returns 404.
    let request = Request::builder()
        .uri(format!("/decks/{id}"))
        .method("GET")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn it_returns_404_when_deleting_other_users_deck(pool: PgPool) {
    let owner = Uuid::new_v4();
    let attacker = Uuid::new_v4();

    let id = db::deck::create_deck(
        &pool,
        db::deck::NewDeck {
            owner_id: owner,
            format: "classic_constructed",
            hero_printing_id: "h",
            name: "Mine",
            description: None,
            visibility: "private",
            tags: &[],
            pool: &[],
            loadouts: &[],
        },
    )
    .await
    .unwrap();

    let token = make_token(attacker);
    let app = api::router(build_state(pool));
    let request = Request::builder()
        .uri(format!("/decks/{id}"))
        .method("DELETE")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
