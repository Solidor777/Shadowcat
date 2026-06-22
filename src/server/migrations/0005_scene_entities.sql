-- Scene entities (tokens, walls, tiles, regions, lights, ...) are top-level
-- documents linked to their scene by parent_id. ON DELETE CASCADE is a DB
-- integrity backstop only: the authoritative delete path (apply_intent) expands
-- a parent delete into explicit, reversible Delete ops for every descendant, so
-- the event log and broadcasts stay complete. The trigger removes a deleted
-- document's FTS row, covering both the per-op delete and any cascade.
ALTER TABLE documents ADD COLUMN parent_id TEXT REFERENCES documents(id) ON DELETE CASCADE;
CREATE INDEX idx_documents_parent ON documents(parent_id);

CREATE TRIGGER documents_fts_delete AFTER DELETE ON documents BEGIN
  DELETE FROM documents_fts WHERE doc_id = old.id;
END;
