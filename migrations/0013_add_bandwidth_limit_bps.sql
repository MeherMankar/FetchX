ALTER TABLE downloads
ADD COLUMN bandwidth_limit_bps INTEGER
DEFAULT NULL
CHECK (
    bandwidth_limit_bps IS NULL
    OR (bandwidth_limit_bps > 0 AND bandwidth_limit_bps <= 1000000000000)
);
