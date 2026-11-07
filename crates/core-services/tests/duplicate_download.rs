use std::path::PathBuf;

use fetchx_core_services::{CoreServices, CoreServicesConfig};
use fetchx_models::{DownloadId, DownloadItem, DownloadSource, EngineKind, FileName, Timestamp};
use fetchx_storage::DownloadRepository;

#[tokio::test]
async fn duplicate_lookup_uses_persisted_downloads_and_normalized_urls()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let services = CoreServices::start(CoreServicesConfig {
        database_path: root.path().join("fetchx.db"),
        downloads_dir: root.path().join("downloads"),
        runtime_dir: root.path().join("runtime"),
        bundled_engines_dir: None,
        defer_queued_on_start: false,
    })
    .await?;
    let id = DownloadId::new();
    let item = DownloadItem::new(
        id,
        DownloadSource::new(
            "https://EXAMPLE.test/file.bin#download",
            EngineKind::DirectHttp,
        )?,
        FileName::new("file.bin")?,
        PathBuf::from("downloads/file.bin"),
        Timestamp::now()?,
    );
    services.repository.create(&item).await?;

    let duplicate = services
        .find_duplicate_download("https://example.test/file.bin#another-view")
        .await?
        .expect("normalized URL should match existing download");
    assert_eq!(*duplicate.id(), id);
    assert_eq!(duplicate.filename(), "file.bin");

    assert!(
        services
            .find_duplicate_download("https://example.test/other.bin")
            .await?
            .is_none()
    );
    services.shutdown().await?;
    Ok(())
}
