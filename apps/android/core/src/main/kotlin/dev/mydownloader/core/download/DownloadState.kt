package dev.mydownloader.core.download

/**
 * The lifecycle state of a download.
 * Maps to the Rust DownloadState enum in crates/models (Phase 20B will bridge these via UniFFI).
 */
sealed class DownloadState {
    /** Waiting to be started by the queue manager. */
    data object Queued : DownloadState()

    /** Actively downloading (one or more segments in progress). */
    data class Active(
        val downloadedBytes: Long,
        val totalBytes: Long?,
        val speedBytesPerSec: Long,
        val etaSeconds: Long?,
        val segmentCount: Int
    ) : DownloadState()

    /** Paused by the user or scheduler. */
    data object Paused : DownloadState()

    /** Download finished successfully. */
    data class Completed(
        val totalBytes: Long,
        val destinationPath: String
    ) : DownloadState()

    /** Download failed. */
    data class Failed(
        val reason: String,
        val retryable: Boolean
    ) : DownloadState()

    /** Cancelled by the user. */
    data object Cancelled : DownloadState()
}
