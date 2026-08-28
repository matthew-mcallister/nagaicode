DROP INDEX IF EXISTS idx_item_session_seqno;
ALTER TABLE item DROP COLUMN completed;
ALTER TABLE item DROP COLUMN seqno;
