-- Renumber seqno to make room, then split each completed tool call's output
-- into its own tool_output item.

CREATE TEMPORARY TABLE wind AS
SELECT
  id,
  seqno,
  min(seqno) OVER (PARTITION BY coalesce(response_id, -id)) AS min_seqno,
  max(seqno) OVER (PARTITION BY coalesce(response_id, -id)) AS max_seqno
FROM item;

DROP INDEX idx_item_session_seqno;

UPDATE item
SET seqno = 2 * wind.min_seqno + (wind.seqno - wind.min_seqno)
FROM wind
WHERE wind.id = item.id;

INSERT INTO item (
  session_id, turn_id, response_id, provider_id, type, upstream_id,
  upstream_type, upstream_call_id, text, summary, encrypted_text,
  tool_args, raw_data, seqno, tool_output
)
SELECT
  item.session_id, item.turn_id, NULL, NULL, 'tool_output', null,
  NULL, item.upstream_call_id, item.text, NULL, NULL,
  item.tool_args, NULL,
  2 * wind.min_seqno + (wind.max_seqno - wind.min_seqno + 1)
    + (wind.seqno - wind.min_seqno),
  item.tool_output
FROM item
JOIN wind ON wind.id = item.id
WHERE item.type = 'tool_call' AND item.tool_output IS NOT NULL;

CREATE UNIQUE INDEX idx_item_session_seqno ON item (session_id, seqno);

DROP TABLE wind;
