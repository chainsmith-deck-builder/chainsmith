-- Decks and their normalized child tables.
--
-- Catalog data (cards, printings) lives in-memory only for now. Hero and
-- pool/loadout printings are stored as opaque text — referential integrity
-- is enforced at the handler layer against the in-memory catalog. When the
-- catalog moves to Postgres (tracker#2) we will add FKs in a follow-up
-- migration.

CREATE TABLE decks (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id           uuid NOT NULL,
    format             text NOT NULL,
    hero_printing_id   text NOT NULL,
    name               text NOT NULL,
    description        text,
    visibility         text NOT NULL DEFAULT 'private'
                       CHECK (visibility IN ('private', 'unlisted', 'public')),
    tags               text[] NOT NULL DEFAULT '{}',
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now(),
    deleted_at         timestamptz
);

-- Active-decks-by-owner is the only common list query.
CREATE INDEX decks_owner_active_idx
    ON decks (owner_id)
    WHERE deleted_at IS NULL;

CREATE TABLE deck_pool_entries (
    deck_id      uuid NOT NULL REFERENCES decks (id) ON DELETE CASCADE,
    printing_id  text NOT NULL,
    quantity     smallint NOT NULL CHECK (quantity > 0),
    PRIMARY KEY (deck_id, printing_id)
);

CREATE TABLE deck_loadouts (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    deck_id     uuid NOT NULL REFERENCES decks (id) ON DELETE CASCADE,
    name        text NOT NULL,
    notes       text,
    -- Stable ordering when listing loadouts for a deck.
    ordinal     smallint NOT NULL DEFAULT 0,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX deck_loadouts_deck_idx ON deck_loadouts (deck_id);

CREATE TABLE deck_loadout_entries (
    loadout_id   uuid NOT NULL REFERENCES deck_loadouts (id) ON DELETE CASCADE,
    printing_id  text NOT NULL,
    quantity     smallint NOT NULL CHECK (quantity > 0),
    PRIMARY KEY (loadout_id, printing_id)
);

CREATE TABLE deck_loadout_equipment (
    loadout_id   uuid NOT NULL REFERENCES deck_loadouts (id) ON DELETE CASCADE,
    slot         text NOT NULL
                 CHECK (slot IN ('head', 'chest', 'arms', 'legs', 'main_hand', 'off_hand')),
    printing_id  text NOT NULL,
    PRIMARY KEY (loadout_id, slot)
);
