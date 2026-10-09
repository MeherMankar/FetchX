#![forbid(unsafe_code)]

mod adaptive;
mod engine;
mod error;
mod http;
mod segments;

pub use adaptive::AdaptiveConfig;
pub use engine::{DownloadEngine, ManagedDownloadControl, ManagedDownloadEngine, ResourceDemand};
pub use error::DownloadEngineError;
pub use http::{DirectHttpEngine, HttpEngineConfig};

#[cfg(test)]
mod tests;
