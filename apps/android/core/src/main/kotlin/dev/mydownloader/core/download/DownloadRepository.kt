package dev.fetchx.core.download

import kotlinx.coroutines.flow.Flow

data class YtDlpFormat(
    val formatId: String,
    val label: String,
    val extension: String?,
    val title: String? = null,
    val platform: String? = null,
)

data class TorrentTelemetry(
    val downloadSpeedBytesPerSecond: Long?,
    val uploadSpeedBytesPerSecond: Long?,
    val downloadedBytes: Long?,
    val uploadedBytes: Long?,
    val peerCount: Long?,
    val completedPeerCount: Long?,
    val seedCount: Long?,
    val shareRatio: Double?,
    val seeding: Boolean?,
    val trackerUris: List<String>?,
)

data class TorrentFile(
    val index: Int,
    val path: String,
    val sizeBytes: Long,
)

data class TorrentMetadata(
    val name: String,
    val infoHash: String,
    val totalSizeBytes: Long,
    val files: List<TorrentFile>,
)

interface TorrentDownloadRepository : DownloadRepository {
    suspend fun inspectTorrentFile(torrentPath: String): TorrentMetadata
    suspend fun addTorrentFile(
        torrentPath: String,
        selectedFileIndexes: List<Int>,
        destinationDirectory: String,
        priority: DownloadPriority,
    ): String
    suspend fun torrentTelemetry(downloadId: String): TorrentTelemetry?
    suspend fun stopTorrentSeeding(downloadId: String)
}

/**
 * Repository interface for download operations.
 *
 * In Phase 20B/20C this will be implemented by a Rust-bridge-backed class that delegates
 * to the UniFFI-generated DownloadService bindings.
 *
 * The interface is defined here (in :core) so that the :downloads UI module and future
 * modules depend only on this contract, never on the Rust bridge directly.
 */
interface DownloadRepository {

    /** Observe all downloads as a reactive stream. Emits the full list on every change. */
    fun observeDownloads(): Flow<List<DownloadItem>>

    /** Observe events from the download engine. */
    fun observeEvents(): Flow<DownloadEvent>

    /** Add a new download to the queue. Returns the assigned download ID. */
    suspend fun addDownload(
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority = DownloadPriority.DEFAULT,
        mimeType: String? = null,
        totalBytes: Long? = null,
        referrer: String? = null
    ): String

    /** Add a webpage URL to the media extractor instead of the direct-file engine. */
    suspend fun addMediaDownload(
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority = DownloadPriority.DEFAULT,
        formatId: String? = null,
    ): String {
        if (formatId != null) {
            throw UnsupportedOperationException("This repository cannot apply a yt-dlp format")
        }
        return addDownload(url, fileName, destinationPath, priority)
    }

    /** Retrieve selectable video formats from the configured yt-dlp runtime. */
    suspend fun discoverMediaFormats(url: String): List<YtDlpFormat> =
        throw UnsupportedOperationException("Media format discovery is unavailable")

    /** Pause an active download. */
    suspend fun pauseDownload(downloadId: String)

    /** Resume a paused download. */
    suspend fun resumeDownload(downloadId: String)

    /** Cancel and optionally delete the partial file. */
    suspend fun cancelDownload(downloadId: String, deleteFile: Boolean = false)

    /** Remove a completed/failed/cancelled download from the list. */
    suspend fun removeDownload(downloadId: String, deleteFile: Boolean = false)

    /** Retry a failed download. */
    suspend fun retryDownload(downloadId: String): String

    /** Set the queue priority of a download. */
    suspend fun setPriority(downloadId: String, priority: DownloadPriority)

    /** Schedule a download to start at a specific wall-clock time (epoch millis). */
    suspend fun scheduleDownload(downloadId: String, startAtMillis: Long)

    /** Retrieve a single download by ID, or null if not found. */
    suspend fun getDownload(downloadId: String): DownloadItem?
}
