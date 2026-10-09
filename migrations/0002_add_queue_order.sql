ALTER TABLE downloads
ADD COLUMN queue_order INTEGER NOT NULL DEFAULT 0 CHECK (queue_order >= 0);

WITH ordered_downloads AS (
    SELECT id, ROW_NUMBER() OVER (ORDER BY created_at, id) AS position
    FROM downloads
)
UPDATE downloads
SET queue_order = (
    SELECT position
    FROM ordered_downloads
    WHERE ordered_downloads.id = downloads.id
);

CREATE INDEX downloads_queue_order ON downloads (queue_order, id);
