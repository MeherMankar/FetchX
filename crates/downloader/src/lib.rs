#![forbid(unsafe_code)]

mod adaptive;
mod aria2;
mod dispatcher;
mod engine;
mod error;
mod http;
mod segments;

pub use adaptive::AdaptiveConfig;
pub use aria2::{
    Aria2Client, Aria2ClientConfig, Aria2ClientError, Aria2DownloadStatus, Aria2Engine, Aria2Gid,
    Aria2Status, Aria2Version,
};
pub use dispatcher::EngineDispatcher;
pub use engine::{DownloadEngine, ManagedDownloadControl, ManagedDownloadEngine, ResourceDemand};
pub use error::DownloadEngineError;
pub use http::{DirectHttpEngine, HttpEngineConfig};

#[cfg(test)]
mod tests;
