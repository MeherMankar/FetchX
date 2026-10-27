use sqlx::SqlitePool;

use crate::StorageError;

/// SQLite-backed key-value store for application configuration.
///
/// Keys are namespaced strings; values are arbitrary JSON text.
/// Callers are responsible for serialising/deserialising the JSON payload.
#[derive(Clone)]
pub struct SqliteConfigRepository {
    pool: SqlitePool,
}

impl SqliteConfigRepository {
    pub(crate) const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns the stored JSON value for `key`, or `None` if absent.
    pub async fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM app_config WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(v,)| v))
    }

    /// Inserts or replaces the JSON value for `key`.
    pub async fn set(&self, key: &str, value: &str) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO app_config (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Inserts `value` only when `key` is absent, returning whether it was inserted.
    pub async fn set_if_absent(&self, key: &str, value: &str) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "INSERT INTO app_config (key, value) VALUES (?, ?) ON CONFLICT(key) DO NOTHING",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Replaces `value` only when `key` exists, returning whether it was replaced.
    pub async fn replace_if_present(&self, key: &str, value: &str) -> Result<bool, StorageError> {
        let result = sqlx::query("UPDATE app_config SET value = ? WHERE key = ?")
            .bind(value)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Removes the entry for `key`. Removing a missing key is idempotent.
    pub async fn delete(&self, key: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM app_config WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
