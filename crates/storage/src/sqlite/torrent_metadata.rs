use std::path::PathBuf;

use fetchx_models::{DownloadId, Timestamp};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use crate::{
    StorageError, TorrentMetadataRepository, TorrentMetadataRow, TorrentSeedState,
    TorrentSourceType,
};

/// SQLite-backed implementation of [`TorrentMetadataRepository`].
#[derive(Clone)]
pub struct SqliteTorrentMetadataRepository {
    pool: SqlitePool,
}

impl SqliteTorrentMetadataRepository {
    pub(crate) const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl TorrentMetadataRepository for SqliteTorrentMetadataRepository {
    async fn create_torrent_metadata(&self, row: &TorrentMetadataRow) -> Result<(), StorageError> {
        let indexes = serde_json::to_string(&row.selected_file_indexes)
            .map_err(|_| StorageError::invalid_data("selected_file_indexes", "invalid JSON"))?;
        let torrent_metadata = row
            .torrent_metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| StorageError::invalid_data("torrent_metadata_json", "invalid JSON"))?;
        let magnet_uri = row.magnet_uri.as_deref();
        let local_path = row
            .local_torrent_path
            .as_ref()
            .and_then(|path| path.to_str())
            .map(str::to_owned);
        let destination = row
            .destination
            .to_str()
            .ok_or(StorageError::NonUtf8Destination)?;
        sqlx::query(
            "INSERT INTO torrent_metadata (
                download_id, source_type, magnet_uri, local_torrent_path, info_hash,
                torrent_name, torrent_metadata_json, destination, metadata_state,
                selected_file_indexes, selected_total_bytes, selection_confirmed,
                aria2_gid, seed_state, uploaded_bytes, upload_speed_bps, peer_count,
                seed_count, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row.download_id.to_string())
        .bind(source_type_to_str(row.source_type))
        .bind(magnet_uri)
        .bind(local_path)
        .bind(&row.info_hash)
        .bind(&row.torrent_name)
        .bind(torrent_metadata)
        .bind(destination)
        .bind(&row.metadata_state)
        .bind(&indexes)
        .bind(to_sqlite_integer(
            "selected_total_bytes",
            row.selected_total_bytes,
        )?)
        .bind(i64::from(row.selection_confirmed))
        .bind(&row.aria2_gid)
        .bind(seed_state_to_str(row.seed_state))
        .bind(to_sqlite_integer("uploaded_bytes", row.uploaded_bytes)?)
        .bind(to_sqlite_integer("upload_speed_bps", row.upload_speed_bps)?)
        .bind(to_sqlite_integer("peer_count", row.peer_count)?)
        .bind(to_sqlite_integer("seed_count", row.seed_count)?)
        .bind(to_sqlite_integer(
            "created_at",
            row.created_at.as_unix_millis(),
        )?)
        .bind(to_sqlite_integer(
            "updated_at",
            row.updated_at.as_unix_millis(),
        )?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_torrent_metadata(&self, row: &TorrentMetadataRow) -> Result<(), StorageError> {
        let indexes = serde_json::to_string(&row.selected_file_indexes)
            .map_err(|_| StorageError::invalid_data("selected_file_indexes", "invalid JSON"))?;
        let torrent_metadata = row
            .torrent_metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| StorageError::invalid_data("torrent_metadata_json", "invalid JSON"))?;
        let magnet_uri = row.magnet_uri.as_deref();
        let local_path = row
            .local_torrent_path
            .as_ref()
            .and_then(|path| path.to_str())
            .map(str::to_owned);
        let destination = row
            .destination
            .to_str()
            .ok_or(StorageError::NonUtf8Destination)?;
        let result = sqlx::query(
            "UPDATE torrent_metadata SET
                source_type = ?, magnet_uri = ?, local_torrent_path = ?, info_hash = ?,
                torrent_name = ?, torrent_metadata_json = ?, destination = ?, metadata_state = ?,
                selected_file_indexes = ?, selected_total_bytes = ?, selection_confirmed = ?,
                aria2_gid = ?, seed_state = ?, uploaded_bytes = ?,
                upload_speed_bps = ?, peer_count = ?, seed_count = ?, updated_at = ?
             WHERE download_id = ?",
        )
        .bind(source_type_to_str(row.source_type))
        .bind(magnet_uri)
        .bind(local_path)
        .bind(&row.info_hash)
        .bind(&row.torrent_name)
        .bind(torrent_metadata)
        .bind(destination)
        .bind(&row.metadata_state)
        .bind(&indexes)
        .bind(to_sqlite_integer(
            "selected_total_bytes",
            row.selected_total_bytes,
        )?)
        .bind(i64::from(row.selection_confirmed))
        .bind(&row.aria2_gid)
        .bind(seed_state_to_str(row.seed_state))
        .bind(to_sqlite_integer("uploaded_bytes", row.uploaded_bytes)?)
        .bind(to_sqlite_integer("upload_speed_bps", row.upload_speed_bps)?)
        .bind(to_sqlite_integer("peer_count", row.peer_count)?)
        .bind(to_sqlite_integer("seed_count", row.seed_count)?)
        .bind(to_sqlite_integer(
            "updated_at",
            row.updated_at.as_unix_millis(),
        )?)
        .bind(row.download_id.to_string())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(row.download_id));
        }
        Ok(())
    }

    async fn set_torrent_aria2_gid(
        &self,
        id: DownloadId,
        gid: Option<&str>,
        updated_at: Timestamp,
    ) -> Result<bool, StorageError> {
        let updated_at = to_sqlite_integer("updated_at", updated_at.as_unix_millis())?;
        let result = sqlx::query(
            "UPDATE torrent_metadata
             SET aria2_gid = ?, updated_at = MAX(updated_at, ?)
             WHERE download_id = ?",
        )
        .bind(gid)
        .bind(updated_at)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() != 0)
    }

    async fn get_torrent_metadata(
        &self,
        id: DownloadId,
    ) -> Result<Option<TorrentMetadataRow>, StorageError> {
        let row = sqlx::query(&format!("{SELECT_COLUMNS} WHERE download_id = ?"))
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_metadata).transpose()
    }

    async fn get_torrent_metadata_by_gid(
        &self,
        gid: &str,
    ) -> Result<Option<TorrentMetadataRow>, StorageError> {
        let row = sqlx::query(&format!("{SELECT_COLUMNS} WHERE aria2_gid = ?"))
            .bind(gid)
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_metadata).transpose()
    }

    async fn list_torrent_metadata(&self) -> Result<Vec<TorrentMetadataRow>, StorageError> {
        let rows = sqlx::query(&format!(
            "{SELECT_COLUMNS} ORDER BY created_at, download_id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_metadata).collect()
    }

    async fn list_active_torrent_downloads(&self) -> Result<Vec<TorrentMetadataRow>, StorageError> {
        let rows = sqlx::query(&format!(
            "{SELECT_COLUMNS}
             WHERE seed_state != 'stopped'
               AND download_id IN (
                   SELECT id FROM downloads
                   WHERE status IN (
                       'queued', 'preparing', 'downloading', 'paused',
                       'metadata_acquiring', 'seeding', 'recovering'
                   )
               )
             ORDER BY created_at, download_id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_metadata).collect()
    }

    async fn delete_torrent_metadata(&self, id: DownloadId) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM torrent_metadata WHERE download_id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() != 0)
    }
}

const SELECT_COLUMNS: &str =
    "SELECT download_id, source_type, magnet_uri, local_torrent_path, info_hash,
                    torrent_name, torrent_metadata_json, destination, metadata_state,
                    selected_file_indexes, selected_total_bytes, selection_confirmed,
                    aria2_gid, seed_state, uploaded_bytes, upload_speed_bps,
                    peer_count, seed_count, created_at, updated_at
             FROM torrent_metadata";

fn row_to_metadata(row: SqliteRow) -> Result<TorrentMetadataRow, StorageError> {
    let id_text: String = row.try_get("download_id")?;
    let download_id = DownloadId::parse(&id_text)
        .map_err(|error| StorageError::invalid_data("download_id", error.to_string()))?;
    if download_id.to_string() != id_text {
        return Err(StorageError::invalid_data(
            "download_id",
            "download ID is not in canonical string form",
        ));
    }

    let source_type = source_type_from_str(row.try_get("source_type")?)?;
    let magnet_uri: Option<String> = row.try_get("magnet_uri")?;
    let local_torrent_path: Option<String> = row.try_get("local_torrent_path")?;
    let info_hash: String = row.try_get("info_hash")?;
    let torrent_name: String = row.try_get("torrent_name")?;
    let torrent_metadata_json: Option<String> = row.try_get("torrent_metadata_json")?;
    let torrent_metadata: Option<fetchx_models::TorrentMetadata> = torrent_metadata_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| StorageError::invalid_data("torrent_metadata_json", "invalid JSON"))?;
    let destination: String = row.try_get("destination")?;
    let metadata_state: String = row.try_get("metadata_state")?;
    let indexes_text: String = row.try_get("selected_file_indexes")?;
    let selected_file_indexes: Vec<u32> = serde_json::from_str(&indexes_text)
        .map_err(|_| StorageError::invalid_data("selected_file_indexes", "invalid JSON"))?;
    let selected_total_bytes =
        from_sqlite_integer("selected_total_bytes", row.try_get("selected_total_bytes")?)?;
    let selection_confirmed: i64 = row.try_get("selection_confirmed")?;
    let selection_confirmed = match selection_confirmed {
        0 => false,
        1 => true,
        _ => {
            return Err(StorageError::invalid_data(
                "selection_confirmed",
                "invalid boolean value",
            ));
        }
    };
    let aria2_gid: Option<String> = row.try_get("aria2_gid")?;
    let seed_state = seed_state_from_str(row.try_get("seed_state")?)?;
    let uploaded_bytes = from_sqlite_integer("uploaded_bytes", row.try_get("uploaded_bytes")?)?;
    let upload_speed_bps =
        from_sqlite_integer("upload_speed_bps", row.try_get("upload_speed_bps")?)?;
    let peer_count = from_sqlite_integer("peer_count", row.try_get("peer_count")?)?;
    let seed_count = from_sqlite_integer("seed_count", row.try_get("seed_count")?)?;
    let created_at = Timestamp::from_unix_millis(from_sqlite_integer(
        "created_at",
        row.try_get("created_at")?,
    )?);
    let updated_at = Timestamp::from_unix_millis(from_sqlite_integer(
        "updated_at",
        row.try_get("updated_at")?,
    )?);

    Ok(TorrentMetadataRow {
        download_id,
        source_type,
        magnet_uri,
        local_torrent_path: local_torrent_path.map(PathBuf::from),
        info_hash,
        torrent_name,
        torrent_metadata,
        destination: PathBuf::from(destination),
        metadata_state,
        selected_file_indexes,
        selected_total_bytes,
        selection_confirmed,
        aria2_gid,
        seed_state,
        uploaded_bytes,
        upload_speed_bps,
        peer_count,
        seed_count,
        created_at,
        updated_at,
    })
}

fn source_type_to_str(value: TorrentSourceType) -> &'static str {
    match value {
        TorrentSourceType::Magnet => "magnet",
        TorrentSourceType::TorrentFile => "torrent_file",
    }
}

fn source_type_from_str(value: &str) -> Result<TorrentSourceType, StorageError> {
    match value {
        "magnet" => Ok(TorrentSourceType::Magnet),
        "torrent_file" => Ok(TorrentSourceType::TorrentFile),
        _ => Err(StorageError::invalid_data(
            "source_type",
            "unknown torrent source type",
        )),
    }
}

fn seed_state_to_str(value: TorrentSeedState) -> &'static str {
    match value {
        TorrentSeedState::Downloading => "downloading",
        TorrentSeedState::Seeding => "seeding",
        TorrentSeedState::Stopped => "stopped",
    }
}

fn seed_state_from_str(value: &str) -> Result<TorrentSeedState, StorageError> {
    match value {
        "downloading" => Ok(TorrentSeedState::Downloading),
        "seeding" => Ok(TorrentSeedState::Seeding),
        "stopped" => Ok(TorrentSeedState::Stopped),
        _ => Err(StorageError::invalid_data(
            "seed_state",
            "unknown torrent seed state",
        )),
    }
}

fn to_sqlite_integer(field: &'static str, value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::IntegerOutOfRange { field })
}

fn from_sqlite_integer(field: &'static str, value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::invalid_data(field, "negative integer"))
}
