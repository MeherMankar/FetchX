ALTER TABLE downloads
ADD COLUMN yt_dlp_audio_format TEXT
CHECK (
    yt_dlp_audio_format IS NULL
    OR (
        yt_dlp_audio_format IN ('mp3', 'm4a')
        AND engine_kind = 'yt_dlp'
        AND yt_dlp_format IS NULL
    )
);
