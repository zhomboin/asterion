CREATE TABLE panel_admin (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    username TEXT NOT NULL CHECK (length(username) BETWEEN 1 AND 64),
    password_hash TEXT NOT NULL CHECK (length(password_hash) > 0),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE panel_sessions (
    token_hash BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
    admin_id INTEGER NOT NULL REFERENCES panel_admin (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at)
) STRICT;

CREATE INDEX panel_sessions_expiry_idx ON panel_sessions (expires_at);
