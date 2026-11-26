-- Named download queues.
-- A NULL queue_id on a download means the default (global) queue.
ALTER TABLE downloads ADD COLUMN queue_id TEXT;

CREATE TABLE IF NOT EXISTS queue_definitions (
    id          TEXT    NOT NULL PRIMARY KEY,
    name        TEXT    NOT NULL,
    max_active  INTEGER,
    bandwidth_limit_bps INTEGER,
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
