-- World invites: a GM mints a single-use bearer code, the invited user redeems
-- it from their own session. The code itself is never stored — only an Argon2
-- PHC hash of its secret half, exactly like `users.password_hash`.
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
