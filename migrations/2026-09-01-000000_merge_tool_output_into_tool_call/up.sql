-- Merge each tool_output item into its paired tool_call item.

ALTER TABLE item ADD COLUMN tool_output TEXT;

-- Text outputs are stored as error JSON.
UPDATE item
SET tool_output = json_object('error', text)
WHERE type = 'tool_output' AND text IS NOT NULL;

UPDATE item
SET tool_output = json
WHERE type = 'tool_output' AND text IS NULL;

ALTER TABLE item RENAME COLUMN json TO tool_args;
ALTER TABLE item DROP COLUMN completed;

-- Copy from tool_output to tool_call
UPDATE item AS call
SET tool_output = output.tool_output
FROM item AS output
WHERE call.type = 'tool_call'
  AND output.type = 'tool_output'
  AND call.upstream_call_id = output.upstream_call_id
  AND call.session_id = output.session_id;

DELETE FROM item WHERE type = 'tool_output';
