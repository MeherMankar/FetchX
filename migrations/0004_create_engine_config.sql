-- Application-level key-value configuration store.
-- Entries are identified by a namespaced text key and store a JSON value.
-- This table is used for persisting engine configuration and other
-- application-level settings that do not belong in download records.
CREATE TABLE IF NOT EXISTS app_config (
    key   TEXT PRIMARY KEY NOT NULL CHECK (length(trim(key)) > 0),
    value TEXT NOT NULL
);
