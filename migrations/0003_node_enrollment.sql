CREATE TABLE node_enrollment_tokens (
    node_id TEXT PRIMARY KEY REFERENCES nodes (id) ON DELETE CASCADE,
    fingerprint BLOB NOT NULL UNIQUE CHECK (length(fingerprint) = 32),
    issued_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK (expires_at > issued_at),
    consumed_at INTEGER,
    revoked_at INTEGER
) STRICT;

CREATE TABLE agent_credentials (
    fingerprint BLOB PRIMARY KEY CHECK (length(fingerprint) = 32),
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER,
    connection_generation INTEGER NOT NULL CHECK (connection_generation > 0)
) STRICT;

CREATE UNIQUE INDEX agent_credentials_active_node_idx
    ON agent_credentials (node_id) WHERE revoked_at IS NULL;
CREATE INDEX agent_credentials_node_idx ON agent_credentials (node_id);

CREATE TABLE node_audit_events (
    id INTEGER PRIMARY KEY,
    actor TEXT NOT NULL,
    node_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('node_create', 'node_update', 'node_remove', 'enrollment_issue', 'credential_revoke')),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX node_audit_events_node_idx ON node_audit_events (node_id, created_at);
