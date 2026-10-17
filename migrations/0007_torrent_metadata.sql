-- Phase 20S.2: durable torrent metadata + widened download status CHECK.
--
-- Additive-only migration following the 0006 table-rebuild pattern so existing
-- FetchX installations keep their rows: a new torrent_metadata table is created
-- and the downloads table is rebuilt with a wider status CHECK that recognises
-- the torrent-specific lifecycle states.

CREATE TABLE torrent_metadata (
    download_id TEXT PRIMARY KEY NOT NULL,
    source_type TEXT NOT NULL CHECK (source_type IN ('magnet', 'torrent_file')),
    magnet_uri TEXT,
    local_torrent_path TEXT,
    info_hash TEXT NOT NULL,
    torrent_name TEXT NOT NULL CHECK (length(trim(torrent_name)) > 0),
    destination TEXT NOT NULL,
    metadata_state TEXT NOT NULL DEFAULT 'pending',
    selected_file_indexes TEXT NOT NULL DEFAULT '[]',
    selected_total_bytes INTEGER NOT NULL DEFAULT 0 CHECK (selected_total_bytes >= 0),
    aria2_gid TEXT,
    seed_state TEXT NOT NULL DEFAULT 'downloading'
        CHECK (seed_state IN ('downloading', 'seeding', 'stopped')),
    uploaded_bytes INTEGER NOT NULL DEFAULT 0 CHECK (uploaded_bytes >= 0),
    upload_speed_bps INTEGER NOT NULL DEFAULT 0 CHECK (upload_speed_bps >= 0),
    peer_count INTEGER NOT NULL DEFAULT 0 CHECK (peer_count >= 0),
    seed_count INTEGER NOT NULL DEFAULT 0 CHECK (seed_count >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    FOREIGN KEY (download_id) REFERENCES downloads(id) ON DELETE CASCADE
);

CREATE INDEX torrent_metadata_gid ON torrent_metadata (aria2_gid);
CREATE INDEX torrent_metadata_seed_state ON torrent_metadata (seed_state);

-- Rebuild downloads with a status CHECK that recognises torrent lifecycle
-- states: metadata_acquiring, seeding, recovering, stopped.
CREATE TABLE downloads_new (
    id TEXT PRIMARY KEY NOT NULL,
    source_url TEXT NOT NULL CHECK (length(trim(source_url)) > 0),
    engine_kind TEXT NOT NULL CHECK (engine_kind IN ('direct_http', 'aria2', 'yt_dlp', 'hls')),
    filename TEXT NOT NULL CHECK (length(trim(filename)) > 0),
    destination TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN (
            'queued', 'preparing', 'downloading', 'paused', 'completed',
            'failed', 'cancelled', 'metadata_acquiring', 'seeding', 'recovering',
            'stopped'
        )
    ),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'normal', 'high')),
    downloaded_bytes INTEGER NOT NULL CHECK (downloaded_bytes >= 0),
    total_bytes INTEGER CHECK (total_bytes IS NULL OR total_bytes >= 0),
    speed_bytes_per_second INTEGER CHECK (
        speed_bytes_per_second IS NULL OR speed_bytes_per_second >= 0
    ),
    estimated_remaining_seconds INTEGER CHECK (
        estimated_remaining_seconds IS NULL OR estimated_remaining_seconds >= 0
    ),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    queue_order INTEGER NOT NULL DEFAULT 0 CHECK (queue_order >= 0),
    error_code TEXT CHECK (
        error_code IS NULL OR error_code IN ('source', 'destination', 'engine', 'internal')
    ),
    error_message TEXT,
    yt_dlp_format TEXT CHECK (
        yt_dlp_format IS NULL
        OR (
            engine_kind = 'yt_dlp'
            AND length(yt_dlp_format) BETWEEN 1 AND 128
            AND yt_dlp_format NOT GLOB '*[^A-Za-z0-9._-]*'
        )
    ),
    CHECK (
        (error_code IS NULL AND error_message IS NULL)
        OR (
            error_code IS NOT NULL
            AND error_message IS NOT NULL
            AND length(trim(error_message)) > 0
        )
    ),
    CHECK (total_bytes IS NULL OR downloaded_bytes <= total_bytes)
);

INSERT INTO downloads_new (
    id, source_url, engine_kind, filename, destination, status, priority,
    downloaded_bytes, total_bytes, speed_bytes_per_second,
    estimated_remaining_seconds, created_at, updated_at, error_code,
    error_message, yt_dlp_format, queue_order
)
SELECT
    id, source_url, engine_kind, filename, destination, status, priority,
    downloaded_bytes, total_bytes, speed_bytes_per_second,
    estimated_remaining_seconds, created_at, updated_at, error_code,
    error_message, yt_dlp_format, queue_order
FROM downloads;

DROP TABLE downloads;
ALTER TABLE downloads_new RENAME TO downloads;

CREATE INDEX downloads_created_at_id ON downloads (created_at, id);
CREATE INDEX downloads_queue_order ON downloads (queue_order, id);
