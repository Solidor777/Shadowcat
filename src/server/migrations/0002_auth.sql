ALTER TABLE users ADD COLUMN password_hash TEXT;

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
