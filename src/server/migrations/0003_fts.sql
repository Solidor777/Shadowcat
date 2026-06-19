-- Standalone FTS5 index over document text. `content` is the only indexed
-- column (extracted leaf text + doc_type, computed in Rust); doc_id/world_id are
-- stored UNINDEXED for retrieval and world-scoping. Synced inside the document
-- write transaction (see SqliteRepository::upsert_document / delete_document_fts).
CREATE VIRTUAL TABLE documents_fts USING fts5(
  content,
  doc_id UNINDEXED,
  world_id UNINDEXED,
  tokenize = 'unicode61'
);
