CREATE TABLE worlds (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  seq INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  server_role TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE world_members (
  world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  user_id  TEXT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
  role     TEXT NOT NULL,
  PRIMARY KEY (world_id, user_id)
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
  json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_documents_world_type ON documents(world_id, doc_type);
CREATE INDEX idx_documents_source     ON documents(source_pack, source_id);
CREATE INDEX idx_documents_scope      ON documents(scope_kind, pack);

CREATE TABLE world_events (
  world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  author_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  ts INTEGER NOT NULL,
  command_json TEXT NOT NULL,
  PRIMARY KEY (world_id, seq)
);
