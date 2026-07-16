-- Split the single two-column `documents_fts` table into two single-column
-- FTS5 tables, one per visibility tier. SQLite FTS5's bm25() computes each
-- row's "document length" normalization term from the TOTAL token count of
-- the WHOLE ROW (all declared columns combined), not per matched column —
-- this is a documented FTS5 characteristic, not a bug in the query that
-- consumes it. In a shared two-column table, per-column bm25() weight
-- arguments zero out a column's term-frequency*IDF CONTRIBUTION but cannot
-- remove its token count from that shared row-length denominator, so a
-- non-GM searcher's score still shifts based on the sheer LENGTH of GM-only
-- text on the same row, even text that never matches their query. Two
-- separate single-column tables make each tier's bm25() row-length
-- computation genuinely isolated: a non-GM query's table has no GM-only
-- text in ANY column of ANY row, so nothing about it can influence score.
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

-- Best-effort backfill from the prior combined table so existing documents
-- remain searchable until their next write re-indexes them from the live
-- Rust extraction (`index_content`/`index_content_public`).
INSERT INTO documents_fts_public (content, doc_id, world_id)
  SELECT content, doc_id, world_id FROM documents_fts;
INSERT INTO documents_fts_gm (content_all, doc_id, world_id)
  SELECT content_all, doc_id, world_id FROM documents_fts;

-- The cascade-delete backstop (migrations/0005_scene_entities.sql) targeted
-- the single combined table; re-point it at both split tables before
-- dropping the old one.
DROP TRIGGER documents_fts_delete;
CREATE TRIGGER documents_fts_public_delete AFTER DELETE ON documents BEGIN
  DELETE FROM documents_fts_public WHERE doc_id = old.id;
END;
CREATE TRIGGER documents_fts_gm_delete AFTER DELETE ON documents BEGIN
  DELETE FROM documents_fts_gm WHERE doc_id = old.id;
END;

DROP TABLE documents_fts;
