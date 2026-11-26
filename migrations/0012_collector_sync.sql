CREATE TABLE collector_sync (
    node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    stream TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1,
    seq INTEGER NOT NULL,
    newest_seq INTEGER NOT NULL,
    interval_secs INTEGER NOT NULL,
    missed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (node_id, stream)
) STRICT, WITHOUT ROWID;

CREATE UNIQUE INDEX collector_active_node ON collector_sync(node_id) WHERE active=1;
