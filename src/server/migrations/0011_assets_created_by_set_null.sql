-- `created_by` is nullable with ON DELETE SET NULL so deleting a user account
-- never FK-fails on assets they uploaded: the asset row survives, attribution
-- becomes NULL. SQLite cannot alter a constraint in place; `assets` has no
-- child tables, so the plain rebuild is safe under foreign_keys=ON with no
-- PRAGMA toggling. idx_assets_world drops with the table and must be
-- recreated.
CREATE TABLE assets_new (
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
INSERT INTO assets_new
  SELECT id, world_id, storage_key, original_name, content_type, byte_size,
         created_by, created_at, version
  FROM assets;
DROP TABLE assets;
ALTER TABLE assets_new RENAME TO assets;
CREATE INDEX idx_assets_world ON assets(world_id);
