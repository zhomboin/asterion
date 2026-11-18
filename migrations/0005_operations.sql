CREATE TABLE operation_jobs (
    id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL,
    node_name TEXT NOT NULL,
    actor TEXT NOT NULL,
    session_hash BLOB NOT NULL CHECK (length(session_hash) = 32),
    challenge_hash BLOB CHECK (challenge_hash IS NULL OR length(challenge_hash) = 32),
    action TEXT NOT NULL CHECK (json_valid(action) AND json_type(action) = 'object'),
    step_up_required INTEGER NOT NULL CHECK (step_up_required IN (0, 1)),
    state TEXT NOT NULL CHECK (state IN (
        'prepared', 'queued', 'running', 'succeeded', 'failed', 'timed_out',
        'disconnected', 'cancelled', 'interrupted', 'expired'
    )),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    expires_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    cancel_requested_at INTEGER,
    exit_code INTEGER,
    output TEXT CHECK (output IS NULL OR length(CAST(output AS BLOB)) <= 32768),
    truncated INTEGER NOT NULL DEFAULT 0 CHECK (truncated IN (0, 1)),
    duration_ms INTEGER,
    error_code TEXT,
    CHECK ((state = 'prepared') = (challenge_hash IS NOT NULL))
) STRICT;

CREATE INDEX operation_jobs_recency_idx ON operation_jobs (created_at DESC, id);
CREATE INDEX operation_jobs_node_idx ON operation_jobs (node_id, state, created_at DESC, id);

CREATE TABLE operation_audit_events (
    id INTEGER PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES operation_jobs (id),
    actor TEXT,
    action TEXT NOT NULL CHECK (action IN (
        'prepared', 'confirmed', 'dispatched', 'cancel_requested', 'succeeded',
        'failed', 'timed_out', 'disconnected', 'cancelled', 'interrupted'
    )),
    observed_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX operation_audit_task_idx ON operation_audit_events (task_id, id);
