use fetchx_models::{QueueDefinition, Timestamp};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use crate::{QueueDefinitionRepository, StorageError};

/// SQLite-backed implementation of [`QueueDefinitionRepository`].
#[derive(Clone)]
pub struct SqliteQueueDefinitionRepository {
    pool: SqlitePool,
}

impl SqliteQueueDefinitionRepository {
    pub(crate) const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl QueueDefinitionRepository for SqliteQueueDefinitionRepository {
    async fn list_queue_definitions(&self) -> Result<Vec<QueueDefinition>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, name, max_active, bandwidth_limit_bps, enabled
             FROM queue_definitions
             ORDER BY name, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_def).collect()
    }

    async fn get_queue_definition(
        &self,
        id: &str,
    ) -> Result<Option<QueueDefinition>, StorageError> {
        let row = sqlx::query(
            "SELECT id, name, max_active, bandwidth_limit_bps, enabled
             FROM queue_definitions
             WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_def).transpose()
    }

    async fn create_queue_definition(&self, def: &QueueDefinition) -> Result<(), StorageError> {
        let now = Timestamp::now()
            .map_err(|e| StorageError::invalid_data("created_at", e.to_string()))?
            .as_unix_millis() as i64;
        let max_active = def.max_active.map(|v| v as i64);
        let bandwidth = def.bandwidth_limit_bps.map(|v| v as i64);
        sqlx::query(
            "INSERT INTO queue_definitions (id, name, max_active, bandwidth_limit_bps, enabled, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&def.id)
        .bind(&def.name)
        .bind(max_active)
        .bind(bandwidth)
        .bind(i64::from(def.enabled))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_queue_definition(&self, def: &QueueDefinition) -> Result<bool, StorageError> {
        let now = Timestamp::now()
            .map_err(|e| StorageError::invalid_data("updated_at", e.to_string()))?
            .as_unix_millis() as i64;
        let max_active = def.max_active.map(|v| v as i64);
        let bandwidth = def.bandwidth_limit_bps.map(|v| v as i64);
        let result = sqlx::query(
            "UPDATE queue_definitions
             SET name = ?, max_active = ?, bandwidth_limit_bps = ?, enabled = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&def.name)
        .bind(max_active)
        .bind(bandwidth)
        .bind(i64::from(def.enabled))
        .bind(now)
        .bind(&def.id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn delete_queue_definition(&self, id: &str) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM queue_definitions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn count_downloads_in_queue(&self, queue_id: &str) -> Result<u64, StorageError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM downloads WHERE queue_id = ?")
            .bind(queue_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(count.max(0) as u64)
    }

    async fn count_all_queue_downloads(&self) -> Result<Vec<(Option<String>, u64)>, StorageError> {
        let rows = sqlx::query("SELECT queue_id, COUNT(*) as cnt FROM downloads GROUP BY queue_id")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                let queue_id: Option<String> = row.try_get("queue_id")?;
                let cnt: i64 = row.try_get("cnt")?;
                Ok((queue_id, cnt.max(0) as u64))
            })
            .collect()
    }
}

fn row_to_def(row: SqliteRow) -> Result<QueueDefinition, StorageError> {
    let id: String = row.try_get("id")?;
    let name: String = row.try_get("name")?;
    let max_active: Option<i64> = row.try_get("max_active")?;
    let bandwidth: Option<i64> = row.try_get("bandwidth_limit_bps")?;
    let enabled_i: i64 = row.try_get("enabled")?;
    Ok(QueueDefinition {
        id,
        name,
        max_active: max_active.map(|v| v.max(0) as u32),
        bandwidth_limit_bps: bandwidth.map(|v| v.max(0) as u64),
        enabled: enabled_i != 0,
    })
}
