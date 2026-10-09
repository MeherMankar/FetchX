use mydownloader_models::{DownloadId, DownloadItem};
use tokio::sync::watch;

use crate::DownloadEngineError;

/// Replaceable async engine interface shared by concrete download backends.
#[allow(async_fn_in_trait)]
pub trait DownloadEngine: Send + Sync {
    /// Executes the download identified by `id`, observing cooperative cancellation.
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError>;
}
