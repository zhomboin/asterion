CREATE TABLE managed_resources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('process', 'docker')),
    target TEXT NOT NULL CHECK (length(target) BETWEEN 1 AND 256),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 256),
    engine_id TEXT NOT NULL DEFAULT '',
    CHECK ((kind = 'process' AND engine_id = '') OR (kind = 'docker' AND engine_id <> '')),
    UNIQUE (node_id, kind, engine_id, target)
) STRICT;

CREATE INDEX managed_resources_list_idx ON managed_resources (node_id, kind, id);
