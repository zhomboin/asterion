CREATE TABLE node_ssh_auth (
    node_id TEXT PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
    method TEXT NOT NULL CHECK (method IN ('key', 'password')),
    identity_file TEXT,
    password TEXT,
    revision TEXT NOT NULL UNIQUE,
    paused INTEGER NOT NULL DEFAULT 0,
    CHECK ((method = 'key' AND identity_file IS NOT NULL AND password IS NULL)
        OR (method = 'password' AND identity_file IS NULL AND password IS NOT NULL))
);
