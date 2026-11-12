//! `fetchx-media-extractor`
//!
//! Owns the single responsibility of "turn a URL into a structured list of
//! downloadable formats." This crate performs no downloading. It defines the
//! [`MediaExtractor`] trait, the [`MediaInfo`]/[`MediaFormat`] model pair, an
//! [`ExtractorRegistry`] for runtime dispatch, native YouTube and Generic
//! extractors, a [`YtDlpExtractor`] that wraps the existing yt-dlp process
//! runner, an [`UnsupportedExtractor`] stub, format-to-engine mapping, and URL
//! safety validation.
//!
//! # Android note
//! [`YtDlpExtractor`] spawns a subprocess and is a **desktop/CLI feature only**.
//! On Android, process spawning is blocked by the SELinux sandbox. The
//! [`GenericExtractor`] is Rust-native and can handle its limited HTML slice
//! on Android. The Android-side yt-dlp fallback still uses Chaquopy
//! (Python in-process) via `AndroidYtDlpRuntime`, which implements the UniFFI
//! `YtDlpRuntime` interface.

#![forbid(unsafe_code)]

mod brightcove;
mod cbc;
mod engine;
mod extractor;
mod format_selector;
mod generic;
mod globalplayer;
mod jwplatform;
mod kaltura;
mod models;
mod ndr;
mod polskieradio;
mod provider_http;
mod radiofrance;
mod registry;
pub mod security;
mod tvp;
mod unsupported;
mod validation;
mod vimeo;
mod youtube;
mod youtube_js;
pub mod ytdlp;

#[cfg(test)]
mod tests;

pub use brightcove::BrightcoveExtractor;
pub use cbc::CbcExtractor;
pub use engine::format_to_engine_kind;
pub use extractor::{
    ExtractOptions, ExtractionError, MediaExtractor, YoutubePlayerClient, YoutubePoToken,
    YoutubePoTokenContext,
};
pub use format_selector::{FormatPreference, FormatSelector, SelectionCriteria, select_format};
pub use generic::GenericExtractor;
pub use globalplayer::GlobalPlayerExtractor;
pub use jwplatform::JwPlatformExtractor;
pub use kaltura::KalturaExtractor;
pub use models::{MediaFormat, MediaInfo, MediaProtocol};
pub use ndr::NdrExtractor;
pub use polskieradio::PolskieRadioExtractor;
pub use radiofrance::RadioFranceExtractor;
pub use registry::ExtractorRegistry;
pub use security::{CookieTempFile, filter_safe_headers};
pub use tvp::TvpExtractor;
pub use unsupported::UnsupportedExtractor;
pub use validation::{is_safe_media_url, sanitise_title_for_filename};
pub use vimeo::VimeoExtractor;
pub use youtube::YoutubeExtractor;
pub use ytdlp::YtDlpExtractor;
