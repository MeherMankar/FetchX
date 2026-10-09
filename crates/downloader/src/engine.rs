use std::future::Future;

use mydownloader_models::{DownloadId, DownloadItem, EngineKind};
use tokio::sync::{mpsc, watch};

use crate::DownloadEngineError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceDemand {
    pub download_id: DownloadId,
    pub desired_concurrency: usize,
    pub active_concurrency: usize,
}

pub struct ManagedDownloadControl {
    pub cancellation: watch::Receiver<bool>,
    pub pause: watch::Receiver<bool>,
    pub connection_limit: watch::Receiver<usize>,
    pub resource_demand: Option<mpsc::Sender<ResourceDemand>>,
}

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

/// Extension for engines that cooperate with queue pause and global connection budgets.
#[allow(async_fn_in_trait)]
pub trait ManagedDownloadEngine: DownloadEngine {
    fn download_managed(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
    ) -> impl Future<Output = Result<DownloadItem, DownloadEngineError>> + Send;

    /// Reports whether active work of this engine kind can be paused without
    /// terminating the underlying transfer.
    fn supports_active_pause(&self, _engine: EngineKind) -> bool {
        true
    }

    /// Stops backend work for an item that has no active queue worker, if any exists.
    ///
    /// Engines without independently persistent jobs may keep the default no-op.
    fn cancel_inactive(
        &self,
        _id: DownloadId,
    ) -> impl Future<Output = Result<(), DownloadEngineError>> + Send {
        async { Ok(()) }
    }
}
