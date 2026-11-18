CREATE TABLE service_monitors (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    unit TEXT NOT NULL CHECK (length(unit) BETWEEN 1 AND 255),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    generation INTEGER NOT NULL DEFAULT 1 CHECK (generation > 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'healthy', 'down', 'unknown')),
    active TEXT,
    sub TEXT,
    reason TEXT,
    checked_at INTEGER,
    changed_at INTEGER,
    next_check_at INTEGER NOT NULL DEFAULT (unixepoch()),
    disabled_at INTEGER,
    CHECK ((enabled = 1 AND disabled_at IS NULL) OR (enabled = 0 AND disabled_at IS NOT NULL)),
    UNIQUE (node_id, unit)
) STRICT;

CREATE INDEX service_monitors_due_idx ON service_monitors (next_check_at, node_id, id)
    WHERE enabled = 1;

CREATE TABLE service_monitor_history (
    id INTEGER PRIMARY KEY,
    monitor_id TEXT NOT NULL REFERENCES service_monitors (id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('healthy', 'down', 'unknown')),
    active TEXT,
    sub TEXT,
    reason TEXT,
    observed_at INTEGER NOT NULL
) STRICT;

CREATE INDEX service_monitor_history_monitor_idx
    ON service_monitor_history (monitor_id, id DESC);

CREATE INDEX service_monitor_history_retention_idx ON service_monitor_history (observed_at);

CREATE TABLE service_alert_events (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    monitor_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    node_name TEXT NOT NULL,
    unit TEXT NOT NULL,
    opened_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    resolved_at INTEGER,
    resolution TEXT,
    CHECK ((resolved_at IS NULL AND resolution IS NULL) OR
           (resolved_at IS NOT NULL AND resolution IS NOT NULL AND resolution IN ('recovered', 'disabled')))
) STRICT;

CREATE UNIQUE INDEX service_alert_events_active_idx
    ON service_alert_events (monitor_id) WHERE resolved_at IS NULL;

CREATE INDEX service_alert_events_recency_idx ON service_alert_events (opened_at DESC, id);

CREATE INDEX service_alert_events_node_recency_idx
    ON service_alert_events (node_id, opened_at DESC, id);

CREATE INDEX service_alert_events_retention_idx ON service_alert_events (resolved_at)
    WHERE resolved_at IS NOT NULL;

CREATE TRIGGER service_monitor_removed AFTER DELETE ON service_monitors
BEGIN
    UPDATE service_alert_events
    SET resolved_at = unixepoch(), updated_at = unixepoch(), resolution = 'disabled'
    WHERE monitor_id = OLD.id AND resolved_at IS NULL;
END;
