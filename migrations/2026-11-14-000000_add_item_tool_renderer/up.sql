ALTER TABLE item ADD COLUMN tool_renderer TEXT;

UPDATE item
SET tool_renderer = text
WHERE type IN ('tool_call', 'tool_output');
