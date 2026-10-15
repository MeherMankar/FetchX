ALTER TABLE downloads
ADD COLUMN yt_dlp_format TEXT
CHECK (
    yt_dlp_format IS NULL
    OR (
        engine_kind = 'yt_dlp'
        AND length(yt_dlp_format) BETWEEN 1 AND 128
        AND yt_dlp_format NOT GLOB '*[^A-Za-z0-9._-]*'
    )
);
