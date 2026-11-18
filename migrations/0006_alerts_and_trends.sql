CREATE TABLE metric_rollups (
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    bucket_at INTEGER NOT NULL,
    sample_count INTEGER NOT NULL DEFAULT 0,
    cpu_sum REAL NOT NULL DEFAULT 0,
    cpu_count INTEGER NOT NULL DEFAULT 0,
    memory_sum REAL NOT NULL DEFAULT 0,
    memory_count INTEGER NOT NULL DEFAULT 0,
    disk_sum REAL NOT NULL DEFAULT 0,
    disk_count INTEGER NOT NULL DEFAULT 0,
    rx_sum REAL NOT NULL DEFAULT 0,
    rx_count INTEGER NOT NULL DEFAULT 0,
    tx_sum REAL NOT NULL DEFAULT 0,
    tx_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (node_id, bucket_at)
) STRICT, WITHOUT ROWID;

CREATE INDEX metric_rollups_retention_idx ON metric_rollups (bucket_at);

WITH readings AS (
    SELECT node_id, (observed_at / 300) * 300 AS bucket_at,
           CAST(json_extract(sample, '$.cpu_percent') AS REAL) AS cpu,
           json_extract(sample, '$.memory.used_bytes') * 100.0
               / NULLIF(json_extract(sample, '$.memory.total_bytes'), 0) AS memory,
           (SELECT MAX(
                (json_extract(value, '$.total_bytes') - json_extract(value, '$.available_bytes'))
                    * 100.0 / NULLIF(json_extract(value, '$.total_bytes'), 0))
            FROM json_each(sample, '$.filesystems')) AS disk,
           (SELECT SUM(json_extract(value, '$.rx_bytes_per_second'))
            FROM json_each(sample, '$.interfaces')) AS rx,
           (SELECT SUM(json_extract(value, '$.tx_bytes_per_second'))
            FROM json_each(sample, '$.interfaces')) AS tx
    FROM metric_samples
)
INSERT INTO metric_rollups (node_id, bucket_at, sample_count, cpu_sum, cpu_count,
    memory_sum, memory_count, disk_sum, disk_count, rx_sum, rx_count, tx_sum, tx_count)
SELECT node_id, bucket_at, COUNT(*), COALESCE(SUM(cpu), 0), COUNT(cpu),
    COALESCE(SUM(memory), 0), COUNT(memory), COALESCE(SUM(disk), 0), COUNT(disk),
    COALESCE(SUM(rx), 0), COUNT(rx), COALESCE(SUM(tx), 0), COUNT(tx)
FROM readings GROUP BY node_id, bucket_at;

CREATE TABLE alert_rules (
    id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    metric TEXT NOT NULL CHECK (metric IN ('offline', 'cpu', 'memory', 'disk')),
    threshold REAL,
    for_seconds INTEGER NOT NULL CHECK (for_seconds BETWEEN 0 AND 3600),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    breach_since INTEGER,
    last_sample_at INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    CHECK (CASE WHEN metric = 'offline' THEN threshold IS NULL
                ELSE threshold IS NOT NULL AND threshold BETWEEN 0 AND 100 END),
    UNIQUE (node_id, metric)
) STRICT;

CREATE TABLE alert_events (
    id TEXT PRIMARY KEY,
    rule_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    node_name TEXT NOT NULL,
    metric TEXT NOT NULL CHECK (metric IN ('offline', 'cpu', 'memory', 'disk')),
    threshold REAL,
    value REAL,
    opened_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    resolved_at INTEGER
) STRICT;

CREATE UNIQUE INDEX alert_events_active_rule_idx
    ON alert_events (rule_id) WHERE resolved_at IS NULL;

CREATE INDEX alert_events_recency_idx ON alert_events (opened_at DESC, id);
CREATE INDEX alert_events_node_recency_idx ON alert_events (node_id, opened_at DESC, id);

CREATE TRIGGER alert_rule_removed AFTER DELETE ON alert_rules
BEGIN
    UPDATE alert_events SET resolved_at = unixepoch(), updated_at = unixepoch()
    WHERE rule_id = OLD.id AND resolved_at IS NULL;
END;
