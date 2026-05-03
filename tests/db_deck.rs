//! Integration tests for the deck DB layer.
//!
//! Each test gets a fresh database via `#[sqlx::test]` (sqlx creates the DB,
//! runs migrations from `./migrations`, hands us a `PgPool`, and drops the
//! DB after). Tests run in parallel safely because every test has its own
//! database.
//!
//! Requires `DATABASE_URL` to point at a Postgres cluster where the
//! configured user can `CREATE DATABASE`. Locally that's the Postgres
//! container set up in CLAUDE.md; in CI it's the `postgres` service in
//! `.github/workflows/rust.yml`.

use chainsmith::db::deck::{self, NewDeck, NewLoadout};
use sqlx::PgPool;
use uuid::Uuid;

fn new_deck<'a>(
    owner: Uuid,
    pool: &'a [(String, i16)],
    loadouts: &'a [NewLoadout<'a>],
) -> NewDeck<'a> {
    NewDeck {
        owner_id: owner,
        format: "classic_constructed",
        hero_printing_id: "hero_p1",
        name: "Test Deck",
        description: None,
        visibility: "private",
        tags: &[],
        pool,
        loadouts,
    }
}

#[sqlx::test]
async fn it_creates_and_fetches_a_minimal_deck(pool: PgPool) {
    let owner = Uuid::new_v4();
    let id = deck::create_deck(&pool, new_deck(owner, &[], &[]))
        .await
        .unwrap();

    let row = deck::fetch_deck(&pool, id)
        .await
        .unwrap()
        .expect("deck exists");
    assert_eq!(row.id, id);
    assert_eq!(row.owner_id, owner);
    assert_eq!(row.name, "Test Deck");
    assert_eq!(row.format, "classic_constructed");
    assert_eq!(row.visibility, "private");
    assert!(row.tags.is_empty());
    assert!(row.deleted_at.is_none());
    assert!(row.created_at <= row.updated_at);
}

#[sqlx::test]
async fn it_persists_pool_entries(pool: PgPool) {
    let owner = Uuid::new_v4();
    let pool_entries = vec![
        ("printing_a".to_string(), 3i16),
        ("printing_b".to_string(), 1i16),
    ];
    let id = deck::create_deck(&pool, new_deck(owner, &pool_entries, &[]))
        .await
        .unwrap();

    let mut entries = deck::fetch_pool_entries(&pool, id).await.unwrap();
    entries.sort_by(|a, b| a.printing_id.cmp(&b.printing_id));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].printing_id, "printing_a");
    assert_eq!(entries[0].quantity, 3);
    assert_eq!(entries[1].printing_id, "printing_b");
    assert_eq!(entries[1].quantity, 1);
}

#[sqlx::test]
async fn it_persists_loadouts_with_entries_and_equipment(pool: PgPool) {
    let owner = Uuid::new_v4();
    let deck_cards = vec![("printing_a".to_string(), 3i16)];
    let equipment = vec![
        ("head", "head_p".to_string()),
        ("main_hand", "weapon_p".to_string()),
    ];
    let loadouts = vec![NewLoadout {
        name: "Main",
        notes: Some("vs aggro"),
        ordinal: 0,
        deck_cards: &deck_cards,
        equipment: &equipment,
    }];

    let id = deck::create_deck(&pool, new_deck(owner, &[], &loadouts))
        .await
        .unwrap();

    let loadout_rows = deck::fetch_loadouts(&pool, id).await.unwrap();
    assert_eq!(loadout_rows.len(), 1);
    assert_eq!(loadout_rows[0].name, "Main");
    assert_eq!(loadout_rows[0].notes.as_deref(), Some("vs aggro"));

    let loadout_ids: Vec<Uuid> = loadout_rows.iter().map(|l| l.id).collect();

    let entries = deck::fetch_loadout_entries(&pool, &loadout_ids)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].printing_id, "printing_a");
    assert_eq!(entries[0].quantity, 3);

    let mut equipment_rows = deck::fetch_loadout_equipment(&pool, &loadout_ids)
        .await
        .unwrap();
    equipment_rows.sort_by(|a, b| a.slot.cmp(&b.slot));
    assert_eq!(equipment_rows.len(), 2);
    assert_eq!(equipment_rows[0].slot, "head");
    assert_eq!(equipment_rows[0].printing_id, "head_p");
    assert_eq!(equipment_rows[1].slot, "main_hand");
    assert_eq!(equipment_rows[1].printing_id, "weapon_p");
}

#[sqlx::test]
async fn it_lists_only_owners_decks(pool: PgPool) {
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    deck::create_deck(&pool, new_deck(alice, &[], &[]))
        .await
        .unwrap();
    deck::create_deck(&pool, new_deck(alice, &[], &[]))
        .await
        .unwrap();
    deck::create_deck(&pool, new_deck(bob, &[], &[]))
        .await
        .unwrap();

    let alice_decks = deck::list_decks_for_owner(&pool, alice).await.unwrap();
    let bob_decks = deck::list_decks_for_owner(&pool, bob).await.unwrap();

    assert_eq!(alice_decks.len(), 2);
    assert_eq!(bob_decks.len(), 1);
    assert!(alice_decks.iter().all(|d| d.owner_id == alice));
    assert!(bob_decks.iter().all(|d| d.owner_id == bob));
}

#[sqlx::test]
async fn it_returns_empty_list_for_owner_with_no_decks(pool: PgPool) {
    let owner = Uuid::new_v4();
    let decks = deck::list_decks_for_owner(&pool, owner).await.unwrap();
    assert!(decks.is_empty());
}

#[sqlx::test]
async fn it_returns_none_when_fetching_missing_deck(pool: PgPool) {
    let result = deck::fetch_deck(&pool, Uuid::new_v4()).await.unwrap();
    assert!(result.is_none());
}

#[sqlx::test]
async fn it_soft_deletes_owned_deck_and_excludes_from_listing(pool: PgPool) {
    let owner = Uuid::new_v4();
    let id = deck::create_deck(&pool, new_deck(owner, &[], &[]))
        .await
        .unwrap();

    let deleted = deck::soft_delete_deck(&pool, id, owner).await.unwrap();
    assert!(deleted);

    let row = deck::fetch_deck(&pool, id).await.unwrap();
    assert!(row.is_none(), "soft-deleted deck should not be returned");

    let decks = deck::list_decks_for_owner(&pool, owner).await.unwrap();
    assert!(decks.is_empty(), "soft-deleted deck should not be listed");
}

#[sqlx::test]
async fn it_does_not_delete_deck_owned_by_another_user(pool: PgPool) {
    let owner = Uuid::new_v4();
    let attacker = Uuid::new_v4();
    let id = deck::create_deck(&pool, new_deck(owner, &[], &[]))
        .await
        .unwrap();

    let deleted = deck::soft_delete_deck(&pool, id, attacker).await.unwrap();
    assert!(!deleted, "delete by non-owner should be a no-op");

    let row = deck::fetch_deck(&pool, id).await.unwrap();
    assert!(row.is_some(), "deck should still exist");
}

#[sqlx::test]
async fn it_returns_false_when_soft_deleting_missing_deck(pool: PgPool) {
    let owner = Uuid::new_v4();
    let deleted = deck::soft_delete_deck(&pool, Uuid::new_v4(), owner)
        .await
        .unwrap();
    assert!(!deleted);
}

#[sqlx::test]
async fn it_cascades_pool_entries_on_owner_delete(pool: PgPool) {
    // Sanity: a hard DELETE on decks (e.g. admin cleanup) should cascade to
    // child rows. We don't expose a hard-delete API but the FK constraint
    // is documented in the schema; this test makes the behavior explicit.
    let owner = Uuid::new_v4();
    let pool_entries = vec![("p1".to_string(), 1i16)];
    let id = deck::create_deck(&pool, new_deck(owner, &pool_entries, &[]))
        .await
        .unwrap();

    sqlx::query("DELETE FROM decks WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();

    let entries = deck::fetch_pool_entries(&pool, id).await.unwrap();
    assert!(entries.is_empty(), "pool entries should cascade");
}
