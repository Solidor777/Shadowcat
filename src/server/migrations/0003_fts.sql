-- Standalone FTS5 index over document text, split by visibility so a non-GM
-- search can never match or snippet GM-only field text:
--   content      — leaf text a non-GM may read (GM-only properties stripped)
--   content_all  — all leaf text (GM/admin search)
-- Both computed in Rust and kept in lockstep with the row inside the document
-- write transaction (see SqliteRepository::upsert_document / delete_document_fts).
-- doc_id/world_id are UNINDEXED (retrieval + world-scoping, not tokenized).
CREATE VIRTUAL TABLE documents_fts USING fts5(
  content,
  content_all,
  doc_id UNINDEXED,
  world_id UNINDEXED,
  tokenize = 'unicode61'
);
