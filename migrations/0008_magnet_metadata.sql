ALTER TABLE torrent_metadata
    ADD COLUMN torrent_metadata_json TEXT;

ALTER TABLE torrent_metadata
    ADD COLUMN selection_confirmed INTEGER NOT NULL DEFAULT 1
        CHECK (selection_confirmed IN (0, 1));

UPDATE torrent_metadata
SET selection_confirmed = CASE
    WHEN source_type = 'magnet' AND metadata_state != 'ready' THEN 0
    ELSE 1
END;
