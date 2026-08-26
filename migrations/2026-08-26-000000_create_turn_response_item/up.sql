DROP TABLE IF EXISTS content;
DROP TABLE IF EXISTS item;
DROP TABLE IF EXISTS chain;

CREATE TABLE turn (
  id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
  type TEXT NOT NULL,
  session_id INTEGER NOT NULL,
  provider_id INTEGER,
  provider_name TEXT,
  model_id TEXT,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (session_id) REFERENCES session (id) ON DELETE CASCADE
);

CREATE TABLE response (
  id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
  session_id INTEGER NOT NULL,
  turn_id INTEGER NOT NULL,
  upstream_id TEXT,
  upstream_status TEXT,
  input_tokens BIGINT,
  cached_input_tokens BIGINT,
  output_tokens BIGINT,
  reasoning_tokens BIGINT,
  total_tokens BIGINT,
  raw_request TEXT,
  raw_response TEXT,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (session_id) REFERENCES session (id) ON DELETE CASCADE,
  FOREIGN KEY (turn_id) REFERENCES turn (id) ON DELETE CASCADE
);

CREATE TABLE item (
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
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (session_id) REFERENCES session (id) ON DELETE CASCADE,
  FOREIGN KEY (turn_id) REFERENCES turn (id) ON DELETE CASCADE,
  FOREIGN KEY (response_id) REFERENCES response (id) ON DELETE CASCADE
);
