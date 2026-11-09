package dev.fetchx.core.download

/**
 * The lifecycle state of a download.
 * Maps to the Rust DownloadState enum in crates/models (Phase 20B will bridge these via UniFFI).
 */
sealed class DownloadState {
    /** Waiting to be started by the queue manager. */
    data object Queued : DownloadState()

    /** A torrent engine is resolving a magnet before the file list is available. */
    data class MetadataAcquiring(
        val downloadedBytes: Long,
        val totalBytes: Long?,
    ) : DownloadState()

    /** Actively downloading (one or more segments in progress). */
    data class Active(
        val downloadedBytes: Long,
        val totalBytes: Long?,
        val speedBytesPerSec: Long,
        val etaSeconds: Long?,
        val segmentCount: Int
    ) : DownloadState()

    /** Paused by the user or scheduler. */
    data class Paused(
        val downloadedBytes: Long = 0L,
        val totalBytes: Long? = null,
    ) : DownloadState()

    /** The payload is complete and the torrent engine is still seeding. */
    data class Seeding(
        val downloadedBytes: Long,
        val totalBytes: Long?,
    ) : DownloadState()

    /** The shared Rust queue is reconciling an interrupted task. */
    data object Recovering : DownloadState()

    /** Download finished successfully. */
    data class Completed(
        val totalBytes: Long,
        val destinationPath: String
    ) : DownloadState()

    /** Rust completed a private staging file, but Android public publication has not succeeded yet. */
    data class PublicationPending(
        val totalBytes: Long,
        val destinationPath: String,
    ) : DownloadState()

    /** Download failed. */
    data class Failed(
        val reason: String,
        val retryable: Boolean
    ) : DownloadState()

    /** Cancelled by the user. */
    data object Cancelled : DownloadState()

    /** Seeding was explicitly stopped; completed files remain available. */
    data object Stopped : DownloadState()
}
