-- Enables pgcrypto for gen_random_uuid() in subsequent migrations.
-- Supabase enables this by default; local Postgres does not.
CREATE EXTENSION IF NOT EXISTS pgcrypto;
