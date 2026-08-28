-- Rebuild item with seqno and completed; backfill seqno from id.
CREATE TABLE item_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
  session_id INTEGER NOT NULL,
  turn_id INTEGER NOT NULL,
  response_id INTEGER,
  provider_id INTEGER,
  type TEXT NOT NULL,
  upstream_id TEXT,
  upstream_type TEXT,
  upstream_call_id TEXT,
  text TEXT,
  summary TEXT,
  encrypted_text TEXT,
  json TEXT,
  raw_data TEXT,
  seqno BIGINT NOT NULL,
  completed BOOLEAN NOT NULL DEFAULT 1,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (session_id) REFERENCES session (id) ON DELETE CASCADE,
  FOREIGN KEY (turn_id) REFERENCES turn (id) ON DELETE CASCADE,
  FOREIGN KEY (response_id) REFERENCES response (id) ON DELETE CASCADE
);

INSERT INTO item_new (
  id, session_id, turn_id, response_id, provider_id, type,
  upstream_id, upstream_type, upstream_call_id, text, summary,
  encrypted_text, json, raw_data, seqno, completed, created_at, updated_at
)
SELECT
  id, session_id, turn_id, response_id, provider_id, type,
  upstream_id, upstream_type, upstream_call_id, text, summary,
  encrypted_text, json, raw_data, id, 1, created_at, updated_at
FROM item;

DROP TABLE item;
ALTER TABLE item_new RENAME TO item;

CREATE INDEX idx_item_session_id ON item (session_id);
CREATE INDEX idx_item_response_id ON item (response_id);
CREATE UNIQUE INDEX idx_item_session_seqno ON item (session_id, seqno);
