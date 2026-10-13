package dev.fetchx.core.download

/**
 * Events emitted by the download engine.
 * In Phase 20B/20C these will be fed from Rust via UniFFI callbacks into a SharedFlow.
 */
sealed class DownloadEvent {
    abstract val downloadId: String

    data class Started(override val downloadId: String) : DownloadEvent()

    data class Progress(
        override val downloadId: String,
        val downloadedBytes: Long,
        val totalBytes: Long?,
        val speedBytesPerSec: Long,
        val etaSeconds: Long?
    ) : DownloadEvent()

    data class Paused(override val downloadId: String) : DownloadEvent()

    data class Resumed(override val downloadId: String) : DownloadEvent()

    data class Completed(
        override val downloadId: String,
        val destinationPath: String,
        val totalBytes: Long
    ) : DownloadEvent()

    data class Failed(
        override val downloadId: String,
        val reason: String,
        val retryable: Boolean
    ) : DownloadEvent()

    data class Cancelled(override val downloadId: String) : DownloadEvent()
}

