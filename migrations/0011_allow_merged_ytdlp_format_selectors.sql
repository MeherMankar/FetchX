-- Allow the merged yt-dlp selector format accepted by DownloadSource while
-- keeping format IDs restricted to the same safe grammar.
CREATE TABLE downloads_new (
    id TEXT PRIMARY KEY NOT NULL,
    source_url TEXT NOT NULL CHECK (length(trim(source_url)) > 0),
    engine_kind TEXT NOT NULL CHECK (
        engine_kind IN (
            'direct_http', 'aria2', 'yt_dlp', 'hls', 'native_torrent',
            'ftp', 'ftps', 'sftp', 'metalink'
        )
    ),
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
            AND yt_dlp_format NOT GLOB '*[^A-Za-z0-9._+/-]*'
            AND (
                (
                    yt_dlp_format NOT GLOB '*[+/]*'
                    AND substr(yt_dlp_format, 1, 1) GLOB '[A-Za-z0-9]'
                )
                OR (
                    substr(yt_dlp_format, 1, 1) GLOB '[A-Za-z0-9]'
                    AND instr(yt_dlp_format, '+bestaudio/') > 1
                    AND length(yt_dlp_format)
                        - length(replace(yt_dlp_format, '+bestaudio/', '')) = 11
                    AND length(yt_dlp_format)
                        - length(replace(yt_dlp_format, '+', '')) = 1
                    AND length(yt_dlp_format)
                        - length(replace(yt_dlp_format, '/', '')) = 1
                    AND substr(
                        yt_dlp_format,
                        1,
                        instr(yt_dlp_format, '+bestaudio/') - 1
                    ) = substr(
                        yt_dlp_format,
                        instr(yt_dlp_format, '+bestaudio/') + 11
                    )
                )
            )
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
