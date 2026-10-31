use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::Write,
    path::{Component, Path, PathBuf},
};

use fetchx_models::{
    DiagnosticComponent, DiagnosticCounter, DiagnosticEvent, DiagnosticLevel, DownloadItem,
    DownloadSchedule, DownloadSource, DownloadStatus, Timestamp, global_diagnostics,
};
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};
use url::Url;

use crate::{
    Database, DownloadRepository, StorageError, TorrentMetadataRepository, TorrentMetadataRow,
    TorrentSeedState, TorrentSourceType, maintenance::CURRENT_SCHEMA_VERSION,
};

pub const EXPORT_FORMAT_VERSION: u32 = 1;
const MAX_EXPORT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXPORTED_DOWNLOADS: usize = 50_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseExport {
    pub export_format_version: u32,
    pub created_at: Timestamp,
    pub application_version: String,
    pub schema_version: i64,
    pub downloads: Vec<DownloadItem>,
    pub schedules: Vec<DownloadSchedule>,
    pub torrent_metadata: Vec<TorrentMetadataRow>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExportResult {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub downloads: usize,
    pub schedules: usize,
    pub torrent_metadata: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ImportResult {
    pub recovery_backup: PathBuf,
    pub downloads: usize,
    pub schedules: usize,
    pub torrent_metadata: usize,
    pub interrupted_downloads_requeued: usize,
}

impl Database {
    /// Writes a versioned export without secrets or unvalidated filesystem paths.
    pub async fn export_to(
        &self,
        destination: impl AsRef<Path>,
    ) -> Result<ExportResult, StorageError> {
        let result = self.export_to_inner(destination.as_ref()).await;
        if result.is_err() {
            global_diagnostics().increment(DiagnosticCounter::StorageExportFailures);
            global_diagnostics().record(
                DiagnosticEvent::new(
                    DiagnosticComponent::Storage,
                    "export_failed",
                    DiagnosticLevel::Error,
                )
                .with_error_code("STORAGE_EXPORT_FAILED"),
            );
            tracing::warn!(
                error_code = "STORAGE_EXPORT_FAILED",
                "database export failed"
            );
        } else {
            global_diagnostics().record(DiagnosticEvent::new(
                DiagnosticComponent::Storage,
                "export_completed",
                DiagnosticLevel::Info,
            ));
            tracing::info!("database export completed");
        }
        result
    }

    async fn export_to_inner(&self, destination: &Path) -> Result<ExportResult, StorageError> {
        if destination.exists() {
            return Err(StorageError::Maintenance(
                "export destination already exists".to_owned(),
            ));
        }
        let health = self.check().await?;
        if !health.is_healthy() {
            return Err(StorageError::Maintenance(
                "refusing to export an unhealthy or unmigrated database".to_owned(),
            ));
        }
        let mut downloads = self.downloads().list().await?;
        let schedules = self.downloads().list_schedules().await?;
        let torrent_metadata = self.torrent_metadata().list_torrent_metadata().await?;
        if downloads.len() > MAX_EXPORTED_DOWNLOADS {
            return Err(StorageError::Maintenance(format!(
                "export contains more than {MAX_EXPORTED_DOWNLOADS} downloads"
            )));
        }
        for item in &downloads {
            validate_export_source(item.source())?;
        }
        for row in &torrent_metadata {
            validate_torrent_export_row(row)?;
        }
        let created_at =
            Timestamp::now().map_err(|error| StorageError::Maintenance(error.to_string()))?;
        let document = DatabaseExport {
            export_format_version: EXPORT_FORMAT_VERSION,
            created_at,
            application_version: env!("CARGO_PKG_VERSION").to_owned(),
            schema_version: CURRENT_SCHEMA_VERSION,
            downloads: std::mem::take(&mut downloads),
            schedules,
            torrent_metadata,
        };
        let bytes = serde_json::to_vec_pretty(&document)
            .map_err(|error| StorageError::Maintenance(error.to_string()))?;
        if bytes.len() as u64 > MAX_EXPORT_BYTES {
            return Err(StorageError::Maintenance(format!(
                "export exceeds the {} MiB size limit",
                MAX_EXPORT_BYTES / (1024 * 1024)
            )));
        }
        write_new_file(destination, &bytes)?;
        Ok(ExportResult {
            path: destination.to_path_buf(),
            size_bytes: bytes.len() as u64,
            downloads: document.downloads.len(),
            schedules: document.schedules.len(),
            torrent_metadata: document.torrent_metadata.len(),
        })
    }

    /// Validates and transactionally imports a versioned export.
    ///
    /// The caller must stop other FetchX processes before import. SQLite holds a
    /// write transaction during activation; a sibling snapshot is validated first.
    pub async fn import_from(
        &self,
        source: impl AsRef<Path>,
        allowed_download_root: impl AsRef<Path>,
    ) -> Result<ImportResult, StorageError> {
        let result = self
            .import_from_inner(source.as_ref(), allowed_download_root.as_ref())
            .await;
        if result.is_err() {
            global_diagnostics().increment(DiagnosticCounter::StorageImportFailures);
            global_diagnostics().record(
                DiagnosticEvent::new(
                    DiagnosticComponent::Storage,
                    "import_failed",
                    DiagnosticLevel::Error,
                )
                .with_error_code("STORAGE_IMPORT_FAILED"),
            );
            tracing::warn!(
                error_code = "STORAGE_IMPORT_FAILED",
                "database import failed"
            );
        } else {
            global_diagnostics().record(DiagnosticEvent::new(
                DiagnosticComponent::Storage,
                "import_completed",
                DiagnosticLevel::Info,
            ));
            tracing::info!("database import completed");
        }
        result
    }

    async fn import_from_inner(
        &self,
        source: &Path,
        allowed_download_root: &Path,
    ) -> Result<ImportResult, StorageError> {
        let file_size = std::fs::metadata(source)
            .map_err(|error| StorageError::Maintenance(error.to_string()))?
            .len();
        if file_size > MAX_EXPORT_BYTES {
            return Err(StorageError::Maintenance(format!(
                "import exceeds the {} MiB size limit",
                MAX_EXPORT_BYTES / (1024 * 1024)
            )));
        }
        let bytes =
            std::fs::read(source).map_err(|error| StorageError::Maintenance(error.to_string()))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| StorageError::Maintenance(format!("invalid export: {error}")))?;
        validate_export_shape(&value)?;
        let mut document: DatabaseExport = serde_json::from_value(value)
            .map_err(|error| StorageError::Maintenance(format!("invalid export: {error}")))?;
        let interrupted_downloads_requeued = document
            .downloads
            .iter()
            .filter(|item| {
                matches!(
                    item.status(),
                    DownloadStatus::Preparing
                        | DownloadStatus::Downloading
                        | DownloadStatus::MetadataAcquiring
                        | DownloadStatus::Recovering
                )
            })
            .count();
        validate_export(&mut document, allowed_download_root)?;

        let health = self.check().await?;
        if !health.is_healthy() {
            return Err(StorageError::Maintenance(
                "refusing to import into an unhealthy or unmigrated database".to_owned(),
            ));
        }
        let destination = self
            .path
            .as_deref()
            .ok_or_else(|| StorageError::Maintenance("database path is unavailable".to_owned()))?;
        let backup_path = recovery_snapshot_path(destination)?;
        self.backup_to(&backup_path).await?;

        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let import_result = activate_export(&mut transaction, &document).await;
        if let Err(error) = import_result {
            let rollback_result = transaction.rollback().await;
            if let Err(rollback_error) = rollback_result {
                return Err(StorageError::Maintenance(format!(
                    "import failed: {error}; rollback failed: {rollback_error}; recovery backup: {}",
                    backup_path.display()
                )));
            }
            return Err(StorageError::Maintenance(format!(
                "import failed and was rolled back: {error}; recovery backup: {}",
                backup_path.display()
            )));
        }
        transaction.commit().await.map_err(|error| {
            StorageError::Maintenance(format!(
                "import commit failed; recovery backup: {}: {error}",
                backup_path.display()
            ))
        })?;

        let post_import = self.check().await?;
        if !post_import.is_healthy() {
            return Err(StorageError::Maintenance(format!(
                "import committed but post-import validation failed; recovery backup: {}",
                backup_path.display()
            )));
        }
        Ok(ImportResult {
            recovery_backup: backup_path,
            downloads: document.downloads.len(),
            schedules: document.schedules.len(),
            torrent_metadata: document.torrent_metadata.len(),
            interrupted_downloads_requeued,
        })
    }
}

fn validate_export_shape(value: &serde_json::Value) -> Result<(), StorageError> {
    object_has_only(
        value,
        &[
            "export_format_version",
            "created_at",
            "application_version",
            "schema_version",
            "downloads",
            "schedules",
            "torrent_metadata",
        ],
    )?;
    for item in array_field(value, "downloads")? {
        object_has_only(
            item,
            &[
                "id",
                "source",
                "filename",
                "destination",
                "status",
                "priority",
                "progress",
                "created_at",
                "updated_at",
                "error",
            ],
        )?;
        let source = item
            .get("source")
            .ok_or_else(|| invalid_export_shape("download source is missing"))?;
        object_has_only(source, &["url", "engine", "yt_dlp_format"])?;
        let progress = item
            .get("progress")
            .ok_or_else(|| invalid_export_shape("download progress is missing"))?;
        object_has_only(
            progress,
            &[
                "downloaded_bytes",
                "total_bytes",
                "speed_bytes_per_second",
                "estimated_remaining_seconds",
            ],
        )?;
        if let Some(error) = item.get("error").filter(|error| !error.is_null()) {
            object_has_only(error, &["code", "message"])?;
        }
    }
    for schedule in array_field(value, "schedules")? {
        object_has_only(
            schedule,
            &[
                "download_id",
                "scheduled_at",
                "enabled",
                "created_at",
                "updated_at",
            ],
        )?;
    }
    for row in array_field(value, "torrent_metadata")? {
        object_has_only(
            row,
            &[
                "download_id",
                "source_type",
                "magnet_uri",
                "local_torrent_path",
                "info_hash",
                "torrent_name",
                "torrent_metadata",
                "destination",
                "metadata_state",
                "selected_file_indexes",
                "selected_total_bytes",
                "selection_confirmed",
                "aria2_gid",
                "seed_state",
                "uploaded_bytes",
                "upload_speed_bps",
                "peer_count",
                "seed_count",
                "created_at",
                "updated_at",
            ],
        )?;
        if let Some(metadata) = row
            .get("torrent_metadata")
            .filter(|metadata| !metadata.is_null())
        {
            object_has_only(
                metadata,
                &[
                    "info_hash",
                    "name",
                    "total_size",
                    "files",
                    "trackers",
                    "piece_length",
                    "piece_count",
                ],
            )?;
            for file in array_field(metadata, "files")? {
                object_has_only(file, &["path", "size"])?;
            }
        }
    }
    Ok(())
}

fn object_has_only(value: &serde_json::Value, allowed: &[&str]) -> Result<(), StorageError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid_export_shape("expected an object"))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid_export_shape("export contains an unknown field"));
    }
    Ok(())
}

fn array_field<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a [serde_json::Value], StorageError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| invalid_export_shape("expected an array"))
}

fn invalid_export_shape(reason: &str) -> StorageError {
    StorageError::Maintenance(format!("invalid export structure: {reason}"))
}

fn validate_export(
    document: &mut DatabaseExport,
    allowed_download_root: &Path,
) -> Result<(), StorageError> {
    if document.export_format_version != EXPORT_FORMAT_VERSION {
        return Err(StorageError::Maintenance(format!(
            "unsupported export format version {}",
            document.export_format_version
        )));
    }
    if document.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(StorageError::Maintenance(format!(
            "unsupported export schema version {}",
            document.schema_version
        )));
    }
    if document.application_version.trim().is_empty() || document.application_version.len() > 128 {
        return Err(StorageError::Maintenance(
            "invalid export application version".to_owned(),
        ));
    }
    validate_timestamp(document.created_at)?;
    if document.downloads.len() > MAX_EXPORTED_DOWNLOADS {
        return Err(StorageError::Maintenance(format!(
            "export contains more than {MAX_EXPORTED_DOWNLOADS} downloads"
        )));
    }
    let download_ids = document
        .downloads
        .iter()
        .map(|item| item.id())
        .collect::<HashSet<_>>();
    if download_ids.len() != document.downloads.len() {
        return Err(StorageError::Maintenance(
            "export contains duplicate download IDs".to_owned(),
        ));
    }
    for item in &document.downloads {
        validate_import_path(item.destination(), allowed_download_root)?;
        validate_export_source(item.source())?;
        if let Some(torrent_path) = item.source().torrent_file_path() {
            validate_import_path(&torrent_path, allowed_download_root)?;
        }
        validate_item_sqlite_ranges(item)?;
    }
    let mut schedule_ids = HashSet::new();
    for schedule in &document.schedules {
        if !schedule_ids.insert(schedule.download_id()) {
            return Err(StorageError::Maintenance(
                "export contains duplicate schedules".to_owned(),
            ));
        }
        if !download_ids.contains(&schedule.download_id()) {
            return Err(StorageError::Maintenance(
                "schedule references a missing download".to_owned(),
            ));
        }
        DownloadSchedule::from_parts(
            schedule.download_id(),
            schedule.scheduled_at(),
            schedule.enabled(),
            schedule.created_at(),
            schedule.updated_at(),
        )
        .map_err(|error| StorageError::Maintenance(error.to_string()))?;
        validate_timestamp(schedule.scheduled_at())?;
        validate_timestamp(schedule.created_at())?;
        validate_timestamp(schedule.updated_at())?;
    }
    let mut torrent_ids = HashSet::new();
    for row in &document.torrent_metadata {
        if !torrent_ids.insert(row.download_id) || !download_ids.contains(&row.download_id) {
            return Err(StorageError::Maintenance(
                "torrent metadata has a duplicate or missing download reference".to_owned(),
            ));
        }
        let item = document
            .downloads
            .iter()
            .find(|item| item.id() == row.download_id)
            .expect("validated download metadata relationship");
        if !matches!(
            item.source().engine(),
            fetchx_models::EngineKind::Aria2 | fetchx_models::EngineKind::NativeTorrent
        ) {
            return Err(StorageError::Maintenance(
                "torrent metadata references a non-torrent download".to_owned(),
            ));
        }
        match row.source_type {
            TorrentSourceType::Magnet
                if !item.source().is_magnet()
                    || row.magnet_uri.as_deref() != Some(item.source().url()) =>
            {
                return Err(StorageError::Maintenance(
                    "magnet metadata does not match the download source".to_owned(),
                ));
            }
            TorrentSourceType::TorrentFile
                if !item.source().is_torrent_file()
                    || item.source().torrent_file_path() != row.local_torrent_path =>
            {
                return Err(StorageError::Maintenance(
                    "torrent file metadata does not match the download source".to_owned(),
                ));
            }
            _ => {}
        }
        validate_import_path(&row.destination, allowed_download_root)?;
        if row.destination != item.destination() {
            return Err(StorageError::Maintenance(
                "torrent metadata destination differs from its download".to_owned(),
            ));
        }
        if let Some(path) = &row.local_torrent_path {
            validate_import_path(path, allowed_download_root)?;
        }
        validate_torrent_export_row(row)?;
        validate_timestamp(row.created_at)?;
        validate_timestamp(row.updated_at)?;
        validate_sqlite_integer("selected_total_bytes", row.selected_total_bytes)?;
        validate_sqlite_integer("uploaded_bytes", row.uploaded_bytes)?;
        validate_sqlite_integer("upload_speed_bps", row.upload_speed_bps)?;
        validate_sqlite_integer("peer_count", row.peer_count)?;
        validate_sqlite_integer("seed_count", row.seed_count)?;
    }
    for item in &mut document.downloads {
        if matches!(
            item.status(),
            DownloadStatus::Preparing
                | DownloadStatus::Downloading
                | DownloadStatus::MetadataAcquiring
                | DownloadStatus::Recovering
        ) {
            item.set_status_at(DownloadStatus::Queued, item.updated_at())
                .map_err(|error| StorageError::Maintenance(error.to_string()))?;
        }
    }
    Ok(())
}

fn validate_export_source(source: &DownloadSource) -> Result<(), StorageError> {
    let value = source.url();
    if contains_sensitive_marker(value) {
        return Err(StorageError::Maintenance(
            "export refused because a download source may contain credentials".to_owned(),
        ));
    }
    if source.is_magnet() {
        let parsed = Url::parse(value)
            .map_err(|error| StorageError::Maintenance(format!("invalid magnet URI: {error}")))?;
        if parsed.query_pairs().any(|(key, _)| key == "tr") {
            return Err(StorageError::Maintenance(
                "export refused because magnet tracker URLs may contain private credentials"
                    .to_owned(),
            ));
        }
        return Ok(());
    }
    if source.is_torrent_file() {
        let local_path = source.torrent_file_path().ok_or_else(|| {
            StorageError::Maintenance("invalid local torrent source path".to_owned())
        })?;
        if contains_sensitive_marker(&local_path.to_string_lossy()) {
            return Err(StorageError::Maintenance(
                "export refused because a local torrent path may contain sensitive data".to_owned(),
            ));
        }
        return Ok(());
    }
    let parsed = Url::parse(value)
        .map_err(|error| StorageError::Maintenance(format!("invalid download URL: {error}")))?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(StorageError::Maintenance(
            "export refused for a download URL containing user info, query parameters, or a fragment"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_torrent_export_row(row: &TorrentMetadataRow) -> Result<(), StorageError> {
    let valid_info_hash = (row.info_hash.len() == 40
        && row.info_hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || (row.info_hash.len() == 32
            && row.info_hash.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() && !matches!(byte, b'0' | b'1' | b'8' | b'9')
            }));
    if !valid_info_hash
        || row.torrent_name.trim().is_empty()
        || row.torrent_name.len() > 1024
        || row.metadata_state.is_empty()
        || row.metadata_state.len() > 64
        || row.selected_file_indexes.len() > 100_000
        || row.updated_at < row.created_at
    {
        return Err(StorageError::Maintenance(
            "invalid torrent metadata record".to_owned(),
        ));
    }
    let selected = row
        .selected_file_indexes
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    if selected.len() != row.selected_file_indexes.len() {
        return Err(StorageError::Maintenance(
            "torrent metadata contains duplicate selected file indexes".to_owned(),
        ));
    }
    if let Some(metadata) = &row.torrent_metadata {
        if metadata.info_hash != row.info_hash
            || metadata.files.len() > 100_000
            || row
                .selected_file_indexes
                .iter()
                .any(|index| *index as usize >= metadata.files.len())
        {
            return Err(StorageError::Maintenance(
                "torrent metadata selection or info hash is inconsistent".to_owned(),
            ));
        }
        let selected_total = row
            .selected_file_indexes
            .iter()
            .try_fold(0_u64, |total, index| {
                total.checked_add(metadata.files[*index as usize].size)
            })
            .ok_or_else(|| {
                StorageError::Maintenance("torrent selection size overflow".to_owned())
            })?;
        if row.selection_confirmed && selected_total != row.selected_total_bytes {
            return Err(StorageError::Maintenance(
                "torrent selected byte count does not match selected files".to_owned(),
            ));
        }
        for tracker in &metadata.trackers {
            let parsed = Url::parse(tracker).map_err(|error| {
                StorageError::Maintenance(format!("invalid torrent tracker URL: {error}"))
            })?;
            if !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || contains_sensitive_marker(tracker)
            {
                return Err(StorageError::Maintenance(
                    "export refused because torrent metadata may contain private tracker data"
                        .to_owned(),
                ));
            }
        }
    }
    if row.source_type == TorrentSourceType::Magnet {
        let magnet_uri = row.magnet_uri.as_deref().ok_or_else(|| {
            StorageError::Maintenance("magnet metadata is missing its source URI".to_owned())
        })?;
        let parsed = Url::parse(magnet_uri)
            .map_err(|error| StorageError::Maintenance(format!("invalid magnet URI: {error}")))?;
        if contains_sensitive_marker(magnet_uri) || parsed.query_pairs().any(|(key, _)| key == "tr")
        {
            return Err(StorageError::Maintenance(
                "export refused because magnet metadata may contain private tracker data"
                    .to_owned(),
            ));
        }
    }
    if row.seed_state == TorrentSeedState::Seeding
        && row.aria2_gid.as_deref().is_some_and(str::is_empty)
    {
        return Err(StorageError::Maintenance(
            "torrent seeding state has an invalid engine identifier".to_owned(),
        ));
    }
    Ok(())
}

fn contains_sensitive_marker(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "session",
        "auth",
        "api_key",
        "apikey",
        "credential",
        "cookie",
        "signature",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

fn validate_import_path(path: &Path, allowed_root: &Path) -> Result<(), StorageError> {
    if path.as_os_str().is_empty()
        || path.to_string_lossy().contains('\0')
        || !path.is_absolute()
        || !allowed_root.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(StorageError::Maintenance(
            "import path must be absolute and contain no traversal components".to_owned(),
        ));
    }
    let root = normalize_path(allowed_root)?;
    let target = normalize_path(path)?;
    if !target.starts_with(&root) {
        return Err(StorageError::Maintenance(
            "import path is outside the configured downloads directory".to_owned(),
        ));
    }
    let canonical_root = std::fs::canonicalize(&root).map_err(|error| {
        StorageError::Maintenance(format!(
            "could not resolve configured downloads directory: {error}"
        ))
    })?;
    let mut existing = target.as_path();
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing.parent().ok_or_else(|| {
                    StorageError::Maintenance("could not resolve import path".to_owned())
                })?;
            }
            Err(error) => {
                return Err(StorageError::Maintenance(format!(
                    "could not inspect import path: {error}"
                )));
            }
        }
    }
    let canonical_existing = std::fs::canonicalize(existing).map_err(|error| {
        StorageError::Maintenance(format!("could not resolve import path: {error}"))
    })?;
    let mut resolved_target = canonical_existing;
    for component in target
        .strip_prefix(existing)
        .map_err(|_| StorageError::Maintenance("invalid import path".to_owned()))?
        .components()
    {
        match component {
            Component::Normal(value) => resolved_target.push(value),
            Component::CurDir => {}
            _ => {
                return Err(StorageError::Maintenance(
                    "invalid import path component".to_owned(),
                ));
            }
        }
    }
    if !resolved_target.starts_with(&canonical_root) {
        return Err(StorageError::Maintenance(
            "import path resolves outside the configured downloads directory".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_path(path: &Path) -> Result<PathBuf, StorageError> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => result.push(prefix.as_os_str()),
            Component::RootDir => result.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(value) => result.push(value),
            Component::ParentDir => {
                return Err(StorageError::Maintenance(
                    "path contains a parent traversal component".to_owned(),
                ));
            }
        }
    }
    Ok(result)
}

fn validate_item_sqlite_ranges(item: &DownloadItem) -> Result<(), StorageError> {
    validate_timestamp(item.created_at())?;
    validate_timestamp(item.updated_at())?;
    validate_sqlite_integer("downloaded_bytes", item.progress().downloaded_bytes())?;
    if let Some(value) = item.progress().total_bytes() {
        validate_sqlite_integer("total_bytes", value)?;
    }
    if let Some(value) = item.progress().speed_bytes_per_second() {
        validate_sqlite_integer("speed_bytes_per_second", value)?;
    }
    if let Some(value) = item.progress().estimated_remaining_seconds() {
        validate_sqlite_integer("estimated_remaining_seconds", value)?;
    }
    Ok(())
}

fn validate_timestamp(timestamp: Timestamp) -> Result<(), StorageError> {
    validate_sqlite_integer("timestamp", timestamp.as_unix_millis())
}

fn validate_sqlite_integer(field: &'static str, value: u64) -> Result<(), StorageError> {
    i64::try_from(value)
        .map(|_| ())
        .map_err(|_| StorageError::IntegerOutOfRange { field })
}

fn write_new_file(destination: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| StorageError::Maintenance(error.to_string()))?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let cleanup_error = std::fs::remove_file(destination).err();
        return Err(StorageError::Maintenance(match cleanup_error {
            Some(cleanup_error) => {
                format!("{error}; could not remove incomplete output: {cleanup_error}")
            }
            None => error.to_string(),
        }));
    }
    Ok(())
}

async fn activate_export(
    transaction: &mut Transaction<'_, Sqlite>,
    document: &DatabaseExport,
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM downloads")
        .execute(&mut **transaction)
        .await?;
    for (index, item) in document.downloads.iter().enumerate() {
        let destination = item
            .destination()
            .to_str()
            .ok_or(StorageError::NonUtf8Destination)?;
        let progress = item.progress();
        let error = item.error();
        sqlx::query(
            "INSERT INTO downloads (
                id, source_url, engine_kind, yt_dlp_format, filename, destination, status, priority,
                downloaded_bytes, total_bytes, speed_bytes_per_second,
                estimated_remaining_seconds, created_at, updated_at, error_code, error_message,
                queue_order
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(item.id().to_string())
        .bind(item.source().url())
        .bind(enum_text(item.source().engine())?)
        .bind(item.source().yt_dlp_format())
        .bind(item.filename().as_str())
        .bind(destination)
        .bind(enum_text(item.status())?)
        .bind(enum_text(item.priority())?)
        .bind(sqlite_integer(
            "downloaded_bytes",
            progress.downloaded_bytes(),
        )?)
        .bind(optional_sqlite_integer(
            "total_bytes",
            progress.total_bytes(),
        )?)
        .bind(optional_sqlite_integer(
            "speed_bytes_per_second",
            progress.speed_bytes_per_second(),
        )?)
        .bind(optional_sqlite_integer(
            "estimated_remaining_seconds",
            progress.estimated_remaining_seconds(),
        )?)
        .bind(sqlite_integer(
            "created_at",
            item.created_at().as_unix_millis(),
        )?)
        .bind(sqlite_integer(
            "updated_at",
            item.updated_at().as_unix_millis(),
        )?)
        .bind(error.map(|value| enum_text(value.code())).transpose()?)
        .bind(error.map(|value| value.message()))
        .bind(
            i64::try_from(index + 1).map_err(|_| StorageError::IntegerOutOfRange {
                field: "queue_order",
            })?,
        )
        .execute(&mut **transaction)
        .await?;
    }
    for schedule in &document.schedules {
        sqlx::query(
            "INSERT INTO download_schedules (
                download_id, scheduled_at_ms, enabled, created_at_ms, updated_at_ms
            ) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(schedule.download_id().to_string())
        .bind(sqlite_integer(
            "scheduled_at_ms",
            schedule.scheduled_at().as_unix_millis(),
        )?)
        .bind(i64::from(schedule.enabled()))
        .bind(sqlite_integer(
            "created_at_ms",
            schedule.created_at().as_unix_millis(),
        )?)
        .bind(sqlite_integer(
            "updated_at_ms",
            schedule.updated_at().as_unix_millis(),
        )?)
        .execute(&mut **transaction)
        .await?;
    }
    for row in &document.torrent_metadata {
        let selected_indexes = serde_json::to_string(&row.selected_file_indexes)
            .map_err(|error| StorageError::Maintenance(error.to_string()))?;
        let metadata = row
            .torrent_metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| StorageError::Maintenance(error.to_string()))?;
        let local_path = match row.local_torrent_path.as_deref() {
            Some(path) => Some(path.to_str().ok_or(StorageError::NonUtf8Destination)?),
            None => None,
        };
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
        .bind(enum_text(row.source_type)?)
        .bind(row.magnet_uri.as_deref())
        .bind(local_path)
        .bind(&row.info_hash)
        .bind(&row.torrent_name)
        .bind(metadata)
        .bind(destination)
        .bind(&row.metadata_state)
        .bind(selected_indexes)
        .bind(sqlite_integer(
            "selected_total_bytes",
            row.selected_total_bytes,
        )?)
        .bind(i64::from(row.selection_confirmed))
        .bind(&row.aria2_gid)
        .bind(enum_text(row.seed_state)?)
        .bind(sqlite_integer("uploaded_bytes", row.uploaded_bytes)?)
        .bind(sqlite_integer("upload_speed_bps", row.upload_speed_bps)?)
        .bind(sqlite_integer("peer_count", row.peer_count)?)
        .bind(sqlite_integer("seed_count", row.seed_count)?)
        .bind(sqlite_integer(
            "created_at",
            row.created_at.as_unix_millis(),
        )?)
        .bind(sqlite_integer(
            "updated_at",
            row.updated_at.as_unix_millis(),
        )?)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

fn enum_text(value: impl Serialize) -> Result<String, StorageError> {
    serde_json::to_value(value)
        .map_err(|error| StorageError::Maintenance(error.to_string()))?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| StorageError::Maintenance("invalid enum value".to_owned()))
}

fn sqlite_integer(field: &'static str, value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::IntegerOutOfRange { field })
}

fn optional_sqlite_integer(
    field: &'static str,
    value: Option<u64>,
) -> Result<Option<i64>, StorageError> {
    value.map(|value| sqlite_integer(field, value)).transpose()
}

fn recovery_snapshot_path(database_path: &Path) -> Result<PathBuf, StorageError> {
    let filename = database_path
        .file_name()
        .ok_or_else(|| StorageError::Maintenance("database path has no filename".to_owned()))?;
    let timestamp = Timestamp::now()
        .map_err(|error| StorageError::Maintenance(error.to_string()))?
        .as_unix_millis();
    for sequence in 0..100_u32 {
        let mut name = filename.to_os_string();
        name.push(format!(".pre-import-{timestamp}-{sequence}.bak"));
        let candidate = database_path.with_file_name(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(StorageError::Maintenance(
        "could not choose a unique import recovery filename".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf};

    use fetchx_models::{
        DownloadId, DownloadItem, DownloadPriority, DownloadProgress, DownloadSchedule,
        DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
    };

    use super::validate_import_path;
    use super::{Database, EXPORT_FORMAT_VERSION};
    use crate::DownloadRepository;

    fn item(
        destination: PathBuf,
        source: &str,
        status: DownloadStatus,
    ) -> Result<DownloadItem, Box<dyn Error>> {
        let timestamp = Timestamp::from_unix_millis(1_725_000_000_123);
        Ok(DownloadItem::from_parts(
            DownloadId::new(),
            DownloadSource::new(source, EngineKind::DirectHttp)?,
            FileName::new("payload.bin")?,
            destination,
            status,
            DownloadPriority::Normal,
            DownloadProgress::default(),
            timestamp,
            timestamp,
            None,
        )?)
    }

    async fn close(database: Database) {
        let pool = database.pool().clone();
        drop(database);
        pool.close().await;
    }

    #[tokio::test]
    async fn export_import_round_trip_requeues_interrupted_work_and_keeps_backup()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let downloads = directory.path().join("downloads");
        std::fs::create_dir(&downloads)?;
        let database_path = directory.path().join("source.db");
        let export_path = directory.path().join("export.json");
        let source_database = Database::connect_path(&database_path).await?;
        source_database
            .config()
            .set("remote.auth.token", "bearer-secret-fixture")
            .await?;
        let active = item(
            downloads.join("active.bin"),
            "https://example.test/payload.bin",
            DownloadStatus::Downloading,
        )?;
        source_database.downloads().create(&active).await?;
        source_database
            .downloads()
            .save_schedule(&DownloadSchedule::new(
                active.id(),
                Timestamp::from_unix_millis(1_725_000_100_000),
                true,
                active.created_at(),
            ))
            .await?;
        let export_result = source_database.export_to(&export_path).await?;
        assert_eq!(export_result.downloads, 1);
        let exported_bytes = std::fs::read(&export_path)?;
        let exported_text = String::from_utf8(exported_bytes.clone())?;
        assert!(!exported_text.contains("bearer-secret-fixture"));
        assert!(!exported_text.contains("remote.auth.token"));
        let exported_json: serde_json::Value = serde_json::from_slice(&exported_bytes)?;
        assert_eq!(
            exported_json["export_format_version"],
            serde_json::json!(EXPORT_FORMAT_VERSION)
        );
        assert_eq!(
            exported_json["schema_version"],
            serde_json::json!(crate::maintenance::CURRENT_SCHEMA_VERSION)
        );
        close(source_database).await;

        let target_path = directory.path().join("target.db");
        let target_database = Database::connect_path(&target_path).await?;
        let stale = item(
            downloads.join("stale.bin"),
            "https://example.test/stale.bin",
            DownloadStatus::Queued,
        )?;
        target_database.downloads().create(&stale).await?;
        let import_result = target_database
            .import_from(&export_path, &downloads)
            .await?;

        assert_eq!(import_result.downloads, 1);
        assert_eq!(import_result.schedules, 1);
        assert_eq!(import_result.interrupted_downloads_requeued, 1);
        assert!(import_result.recovery_backup.exists());
        let imported = target_database
            .downloads()
            .get(active.id())
            .await?
            .ok_or("imported download is missing")?;
        assert_eq!(imported.status(), DownloadStatus::Queued);
        assert!(target_database.downloads().get(stale.id()).await?.is_none());
        assert_eq!(
            target_database
                .downloads()
                .get_schedule(active.id())
                .await?,
            Some(DownloadSchedule::new(
                active.id(),
                Timestamp::from_unix_millis(1_725_000_100_000),
                true,
                active.created_at(),
            ))
        );
        close(target_database).await;
        Ok(())
    }

    #[tokio::test]
    async fn export_refuses_urls_that_may_contain_credentials() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let downloads = directory.path().join("downloads");
        std::fs::create_dir(&downloads)?;
        let database = Database::connect_path(directory.path().join("db.sqlite")).await?;
        let item = item(
            downloads.join("payload.bin"),
            "https://example.test/payload.bin?token=private",
            DownloadStatus::Queued,
        )?;
        database.downloads().create(&item).await?;

        let output = directory.path().join("should-not-exist.json");
        let error = database.export_to(&output).await.unwrap_err();
        assert!(error.to_string().contains("credentials"));
        assert!(!output.exists());
        close(database).await;
        Ok(())
    }

    #[tokio::test]
    async fn import_rejects_unsupported_versions_and_paths_without_changing_database()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let downloads = directory.path().join("downloads");
        std::fs::create_dir(&downloads)?;
        let source_database = Database::connect_path(directory.path().join("source.db")).await?;
        let source_item = item(
            downloads.join("payload.bin"),
            "https://example.test/payload.bin",
            DownloadStatus::Queued,
        )?;
        source_database.downloads().create(&source_item).await?;
        let export_path = directory.path().join("export.json");
        source_database.export_to(&export_path).await?;
        close(source_database).await;
        let original: serde_json::Value = serde_json::from_slice(&std::fs::read(&export_path)?)?;

        let target_database = Database::connect_path(directory.path().join("target.db")).await?;
        let existing = item(
            downloads.join("existing.bin"),
            "https://example.test/existing.bin",
            DownloadStatus::Queued,
        )?;
        target_database.downloads().create(&existing).await?;

        let mut unsupported = original.clone();
        unsupported["export_format_version"] = serde_json::json!(EXPORT_FORMAT_VERSION + 1);
        let unsupported_path = directory.path().join("unsupported.json");
        std::fs::write(&unsupported_path, serde_json::to_vec(&unsupported)?)?;
        let error = target_database
            .import_from(&unsupported_path, &downloads)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unsupported export format"));

        let malformed_path = directory.path().join("malformed.json");
        std::fs::write(&malformed_path, br#"{"export_format_version": "#)?;
        assert!(
            target_database
                .import_from(&malformed_path, &downloads)
                .await
                .is_err()
        );

        let mut invalid_id = original.clone();
        invalid_id["downloads"][0]["id"] = serde_json::json!("not-a-uuid");
        let invalid_id_path = directory.path().join("invalid-id.json");
        std::fs::write(&invalid_id_path, serde_json::to_vec(&invalid_id)?)?;
        assert!(
            target_database
                .import_from(&invalid_id_path, &downloads)
                .await
                .is_err()
        );

        let mut invalid_timestamp = original.clone();
        invalid_timestamp["downloads"][0]["created_at"] = serde_json::json!(1_900_000_000_000_u64);
        let invalid_timestamp_path = directory.path().join("invalid-timestamp.json");
        std::fs::write(
            &invalid_timestamp_path,
            serde_json::to_vec(&invalid_timestamp)?,
        )?;
        assert!(
            target_database
                .import_from(&invalid_timestamp_path, &downloads)
                .await
                .is_err()
        );

        let mut duplicates = original.clone();
        let first_download = duplicates["downloads"][0].clone();
        duplicates["downloads"]
            .as_array_mut()
            .ok_or("downloads should be an array")?
            .push(first_download);
        let duplicates_path = directory.path().join("duplicates.json");
        std::fs::write(&duplicates_path, serde_json::to_vec(&duplicates)?)?;
        let error = target_database
            .import_from(&duplicates_path, &downloads)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("duplicate download IDs"));

        let mut escaped = original.clone();
        escaped["downloads"][0]["destination"] = serde_json::json!(
            directory
                .path()
                .join("outside.bin")
                .to_string_lossy()
                .to_string()
        );
        let escaped_path = directory.path().join("escaped.json");
        std::fs::write(&escaped_path, serde_json::to_vec(&escaped)?)?;
        let error = target_database
            .import_from(&escaped_path, &downloads)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("outside"));
        assert!(
            target_database
                .downloads()
                .get(existing.id())
                .await?
                .is_some()
        );

        sqlx::query(
            "CREATE TRIGGER reject_imported_download
             BEFORE INSERT ON downloads
             WHEN NEW.source_url = 'https://example.test/payload.bin'
             BEGIN SELECT RAISE(ABORT, 'forced import failure'); END",
        )
        .execute(target_database.pool())
        .await?;
        let error = target_database
            .import_from(&export_path, &downloads)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("rolled back"));
        assert!(
            target_database
                .downloads()
                .get(existing.id())
                .await?
                .is_some()
        );
        close(target_database).await;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn import_path_validation_rejects_traversal_and_cross_platform_absolute_paths()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let downloads = directory.path().join("downloads");
        std::fs::create_dir(&downloads)?;
        let paths = [
            downloads.join("../outside.bin"),
            downloads.join("nested/../../outside.bin"),
            PathBuf::from("/outside/file.bin"),
            PathBuf::from(r"C:\outside\file.bin"),
            PathBuf::from(r"\\server\share\file.bin"),
            PathBuf::from(r"..\outside\file.bin"),
        ];
        for path in paths {
            assert!(
                validate_import_path(&path, &downloads).is_err(),
                "unexpectedly accepted {}",
                path.display()
            );
        }
        assert!(validate_import_path(&downloads.join("new/nested/file.bin"), &downloads).is_ok());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn import_path_validation_rejects_windows_and_mixed_separator_traversal()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let downloads = directory.path().join("downloads");
        std::fs::create_dir(&downloads)?;
        for suffix in [r"\..\outside.bin", r"\safe/../../outside.bin"] {
            let path = PathBuf::from(format!("{}{suffix}", downloads.display()));
            assert!(validate_import_path(&path, &downloads).is_err());
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn import_path_validation_rejects_symlink_escape() -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let downloads = directory.path().join("downloads");
        let outside = directory.path().join("outside");
        std::fs::create_dir(&downloads)?;
        std::fs::create_dir(&outside)?;
        let link = downloads.join("linked");
        symlink(&outside, &link)?;

        let error = validate_import_path(&link.join("payload.bin"), &downloads).unwrap_err();
        assert!(error.to_string().contains("resolves outside"));
        Ok(())
    }
}
