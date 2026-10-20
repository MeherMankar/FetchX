//! `fetchx-media-extractor`
//!
//! Owns the single responsibility of "turn a URL into a structured list of
//! downloadable formats." This crate performs no downloading. It defines the
//! [`MediaExtractor`] trait, the [`MediaInfo`]/[`MediaFormat`] model pair, an
//! [`ExtractorRegistry`] for runtime dispatch, a [`YtDlpExtractor`] stub that
//! wraps the existing yt-dlp process runner (see note below), an
//! [`UnsupportedExtractor`] always-fail stub, format-to-engine mapping, and
//! URL safety validation.
//!
//! # Android note
//! [`YtDlpExtractor`] spawns a subprocess and is a **desktop/CLI feature only**.
//! On Android, process spawning is blocked by the SELinux sandbox. The
//! Android-side runtime uses Chaquopy (Python in-process) via the
//! `AndroidYtDlpRuntime` Kotlin class, which implements the UniFFI
//! `YtDlpRuntime` interface. A future native extractor (e.g. wrapping
//! `rusty_ytdl`) would be required for a fully Android-safe extraction path.

#![forbid(unsafe_code)]

mod engine;
mod extractor;
mod format_selector;
mod models;
mod registry;
pub mod security;
mod unsupported;
mod validation;
pub mod ytdlp;

#[cfg(test)]
mod tests;

pub use engine::format_to_engine_kind;
pub use extractor::{ExtractOptions, ExtractionError, MediaExtractor};
pub use format_selector::{FormatPreference, FormatSelector, SelectionCriteria, select_format};
pub use models::{MediaFormat, MediaInfo, MediaProtocol};
pub use registry::ExtractorRegistry;
pub use security::{CookieTempFile, filter_safe_headers};
pub use unsupported::UnsupportedExtractor;
pub use validation::{is_safe_media_url, sanitise_title_for_filename};
pub use ytdlp::YtDlpExtractor;
