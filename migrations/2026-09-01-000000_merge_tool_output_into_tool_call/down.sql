-- Split merged tool_call items back into tool_call and tool_output pairs.

ALTER TABLE item RENAME COLUMN tool_args TO json;

-- Prevent seqno collisions
DROP INDEX idx_item_session_seqno;
UPDATE item SET seqno = seqno * 2;

INSERT INTO item (
  session_id, turn_id, response_id, provider_id, type,
  upstream_id, upstream_type, upstream_call_id, text, summary,
  encrypted_text, json, raw_data, seqno, created_at, updated_at
)
SELECT
  session_id, turn_id, NULL, provider_id, 'tool_output',
  NULL, 'function_call_output', upstream_call_id, NULL, NULL,
  NULL, tool_output, NULL, seqno + 1, created_at, updated_at
FROM item
WHERE type = 'tool_call';

CREATE UNIQUE INDEX idx_item_session_seqno ON item (session_id, seqno);

ALTER TABLE item ADD COLUMN completed BOOLEAN NOT NULL DEFAULT 0;
UPDATE item
SET completed = (type != 'tool_output' OR json IS NOT NULL);

ALTER TABLE item DROP COLUMN tool_output;
