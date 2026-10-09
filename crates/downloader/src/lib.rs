#![forbid(unsafe_code)]

mod engine;
mod error;
mod http;

pub use engine::DownloadEngine;
pub use error::DownloadEngineError;
pub use http::{DirectHttpEngine, HttpEngineConfig};

#[cfg(test)]
mod tests;
