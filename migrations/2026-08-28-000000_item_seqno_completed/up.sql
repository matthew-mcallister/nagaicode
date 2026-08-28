-- FIXME: Default 0 is not allowed. We must recreate the table from scratch
-- without the default
ALTER TABLE item ADD COLUMN seqno BIGINT NOT NULL DEFAULT 0;
UPDATE item SET seqno = id;
ALTER TABLE item ADD COLUMN completed BOOLEAN NOT NULL DEFAULT 1;
CREATE UNIQUE INDEX idx_item_session_seqno ON item (session_id, seqno);
