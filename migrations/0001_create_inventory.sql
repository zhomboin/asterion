CREATE TABLE hosts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    kind TEXT NOT NULL CHECK (kind IN ('linux', 'windows')),
    notes TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE nodes (
    id TEXT PRIMARY KEY,
    host_id TEXT REFERENCES hosts (id) ON DELETE SET NULL,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    kind TEXT NOT NULL CHECK (kind IN ('linux', 'wsl')),
    os TEXT,
    arch TEXT,
    wsl_distro TEXT CHECK (wsl_distro IS NULL OR kind = 'wsl'),
    agent_version TEXT,
    tags TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags) AND json_type(tags) = 'array'),
    notes TEXT NOT NULL DEFAULT '',
    last_seen_at INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX nodes_host_id_idx ON nodes (host_id);

CREATE TABLE runtimes (
    id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('docker', 'systemd')),
    identity TEXT NOT NULL CHECK (length(trim(identity)) > 0),
    version TEXT,
    last_seen_at INTEGER,
    UNIQUE (node_id, kind, identity)
) STRICT;

CREATE TABLE service_assets (
    id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE RESTRICT,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    management TEXT NOT NULL CHECK (management IN ('docker', 'compose', 'systemd', 'process', 'other')),
    description TEXT NOT NULL DEFAULT '',
    repo_url TEXT,
    deploy_path TEXT,
    config_path TEXT,
    tags TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags) AND json_type(tags) = 'array'),
    notes TEXT NOT NULL DEFAULT '',
    verified_at INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX service_assets_node_id_idx ON service_assets (node_id);
