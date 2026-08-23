-- Baseline schema. CONSTRAINT: pre-customer builds carry no upgrade path, so
-- schema changes edit this baseline in place instead of adding incremental
-- migration files; the sqlx migration machinery stays so real migrations can
-- begin once a release milestone declares live databases. A dev database
-- predating a baseline edit fails the sqlx checksum check — delete the dev DB
-- file and restart.

CREATE TABLE users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  server_role TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  password_hash TEXT,
  ui_state TEXT
);

CREATE TABLE worlds (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  seq INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- Server-wide key/value settings (session signing key, world schema
-- declarations, ...).
CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE documents (
  id TEXT PRIMARY KEY,
  scope_kind TEXT NOT NULL,
  world_id TEXT REFERENCES worlds(id) ON DELETE CASCADE,
  pack TEXT,
  doc_type TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  source_id TEXT,
  source_pack TEXT,
  source_version INTEGER,
  owner_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  seq INTEGER NOT NULL DEFAULT 0,
  created_seq INTEGER NOT NULL DEFAULT 0,
  json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  parent_id TEXT REFERENCES documents(id) ON DELETE CASCADE
);

CREATE INDEX idx_documents_scope      ON documents(scope_kind, pack);
CREATE INDEX idx_documents_source     ON documents(source_pack, source_id);
CREATE INDEX idx_documents_world_type ON documents(world_id, doc_type);
CREATE INDEX idx_documents_parent     ON documents(parent_id);

CREATE TABLE world_events (
  world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  author_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  ts INTEGER NOT NULL,
  command_json TEXT NOT NULL,
  PRIMARY KEY (world_id, seq)
);

CREATE TABLE world_members (
  world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  user_id  TEXT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
  role     TEXT NOT NULL,
  PRIMARY KEY (world_id, user_id)
);

CREATE TABLE world_invites (
  id          TEXT PRIMARY KEY,
  world_id    TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  -- PHC string over the code's secret half. The plaintext secret exists only
  -- in the mint response.
  secret_hash TEXT NOT NULL,
  -- WorldRole ('gm'/'player'/'spectator'). No server tier is representable.
  role        TEXT NOT NULL,
  created_by  TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at  INTEGER NOT NULL,
  expires_at  INTEGER NOT NULL,
  -- Single-use + revocable: the redemption UPDATE requires both to be NULL,
  -- so consumption and revocation are enforced by one guarded statement.
  revoked_at  INTEGER,
  consumed_at INTEGER,
  consumed_by TEXT REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX idx_world_invites_world ON world_invites(world_id);

CREATE TABLE assets (
  id            TEXT PRIMARY KEY,
  world_id      TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  storage_key   TEXT NOT NULL,
  original_name TEXT NOT NULL,
  content_type  TEXT NOT NULL,
  byte_size     INTEGER NOT NULL,
  created_by    TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at    INTEGER NOT NULL,
  version       INTEGER NOT NULL
);

CREATE INDEX idx_assets_world ON assets(world_id);

-- Persisted link-preview cache: the DB-backed tier behind chat::LinkPreviewCache's
-- in-memory fast path. No world_id — same process-global, URL-keyed scope the
-- in-memory cache already has, now durable across restarts. title/description
-- both NULL together = a cached negative outcome (the fetch failed or found no
-- content), mirroring LinkPreview's own None-outcome convention.
CREATE TABLE link_preview_cache (
    url TEXT PRIMARY KEY,
    title TEXT,
    description TEXT,
    image_asset_id TEXT REFERENCES assets(id) ON DELETE SET NULL,
    fetched_at TEXT NOT NULL
);

-- Per-(scene, user) explored-fog memory. world_id denormalized for the
-- world-scoped sweep query.
CREATE TABLE explored_fog (
  world_id  TEXT NOT NULL,
  scene_id  TEXT NOT NULL,
  user_id   TEXT NOT NULL,
  cells     BLOB NOT NULL,
  PRIMARY KEY (scene_id, user_id)
);

CREATE INDEX idx_explored_fog_world ON explored_fog(world_id);

-- Visibility-partitioned full-text search: physically separate PUBLIC and GM
-- indexes (redacting a shared index's results would still leak via
-- snippet/score). The virtual tables auto-create their fts5 shadow tables.
CREATE VIRTUAL TABLE documents_fts_public USING fts5(
  content,
  doc_id UNINDEXED,
  world_id UNINDEXED,
  tokenize = 'unicode61'
);

CREATE VIRTUAL TABLE documents_fts_gm USING fts5(
  content_all,
  doc_id UNINDEXED,
  world_id UNINDEXED,
  tokenize = 'unicode61'
);

CREATE TRIGGER documents_fts_public_delete AFTER DELETE ON documents BEGIN
  DELETE FROM documents_fts_public WHERE doc_id = old.id;
END;

CREATE TRIGGER documents_fts_gm_delete AFTER DELETE ON documents BEGIN
  DELETE FROM documents_fts_gm WHERE doc_id = old.id;
END;
