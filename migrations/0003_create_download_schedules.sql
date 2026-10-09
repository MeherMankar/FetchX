CREATE TABLE download_schedules (
    download_id TEXT PRIMARY KEY NOT NULL,
    scheduled_at_ms INTEGER NOT NULL CHECK (scheduled_at_ms >= 0),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (
        updated_at_ms >= created_at_ms
    ),
    FOREIGN KEY (download_id) REFERENCES downloads(id) ON DELETE CASCADE
);

CREATE INDEX download_schedules_enabled_time
ON download_schedules (enabled, scheduled_at_ms, download_id);
