CREATE TABLE metric_samples (
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    observed_at INTEGER NOT NULL,
    sample TEXT NOT NULL CHECK (json_valid(sample) AND json_type(sample) = 'object' AND length(CAST(sample AS BLOB)) <= 49152),
    PRIMARY KEY (node_id, observed_at)
) STRICT, WITHOUT ROWID;

CREATE INDEX metric_samples_retention_idx ON metric_samples (observed_at);

CREATE TABLE discovered_resources (
    id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('docker', 'systemd', 'process')),
    resource_scope TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    name TEXT NOT NULL,
    metadata TEXT NOT NULL CHECK (json_valid(metadata) AND json_type(metadata) = 'object'),
    observed_at INTEGER NOT NULL,
    UNIQUE (node_id, kind, resource_scope, resource_id),
    UNIQUE (id, node_id)
) STRICT;

CREATE INDEX discovered_resources_recency_idx
    ON discovered_resources (node_id, observed_at DESC, id);

CREATE UNIQUE INDEX service_assets_identity_idx ON service_assets (id, node_id);

CREATE TABLE asset_bindings (
    asset_id TEXT NOT NULL,
    discovery_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    bound_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (asset_id, discovery_id),
    FOREIGN KEY (asset_id, node_id) REFERENCES service_assets (id, node_id)
        ON DELETE CASCADE,
    FOREIGN KEY (discovery_id, node_id) REFERENCES discovered_resources (id, node_id)
        ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE INDEX asset_bindings_discovery_idx ON asset_bindings (discovery_id, node_id);

CREATE TABLE asset_audit_events (
    id INTEGER PRIMARY KEY,
    actor TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('asset_create', 'asset_update', 'asset_remove', 'asset_bind', 'asset_unbind')),
    observed_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;
