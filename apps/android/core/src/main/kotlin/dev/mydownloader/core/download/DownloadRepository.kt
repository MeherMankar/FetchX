package dev.fetchx.core.download

import kotlinx.coroutines.flow.Flow

data class YtDlpFormat(
    val formatId: String,
    val label: String,
    val extension: String?,
    val title: String? = null,
    val platform: String? = null,
)

data class DownloadBatchItem(
    val url: String,
    val fileName: String,
    val destinationPath: String,
    val isMedia: Boolean = false,
    val formatId: String? = null,
    val extension: String? = null,
    val audioFormat: String? = null,
)

enum class DownloadBatchError {
    DUPLICATE_URL,
    INVALID_URL,
    INVALID_PACKAGE_NAME,
    INVALID_FORMAT,
    INVALID_CHECKSUM,
    INVALID_DESTINATION,
    UNAVAILABLE_ENGINE,
    DESTINATION,
    QUEUE,
    STORAGE,
}

data class DownloadBatchOutcome(
    val inputUrl: String,
    val downloadId: String?,
    val error: DownloadBatchError?,
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
    val trackers: List<String> = emptyList(),
    val pieceLength: Long = 0,
    val pieceCount: Long = 0,
)

data class TorrentDownloadMetadata(
    val metadataState: String,
    val metadata: TorrentMetadata?,
    val selectedFileIndexes: List<Int>,
    val selectedTotalBytes: Long,
    val selectionConfirmed: Boolean,
)

enum class Aria2RecoveryState {
    NOT_ARIA2,
    NOT_REQUIRED,
    RUNTIME_UNAVAILABLE,
    READY_TO_START,
    JOB_AVAILABLE,
    MISSING_TORRENT_METADATA,
    MISSING_GID,
    STALE_GID,
    MISSING_CONTROL_FILE,
    CONTROL_FILE_PRESENT,
    UNKNOWN,
}

data class Aria2RecoveryAssessment(
    val state: Aria2RecoveryState,
    val explanation: String,
    val nextStep: String,
)

interface Aria2RecoveryRepository {
    suspend fun assessAria2Recovery(downloadId: String): Aria2RecoveryAssessment?
}

interface TorrentDownloadRepository : DownloadRepository {
    suspend fun addMagnet(magnetUri: String, priority: DownloadPriority): String
    suspend fun inspectTorrentFile(torrentPath: String): TorrentMetadata
    suspend fun addTorrentFile(
        torrentPath: String,
        selectedFileIndexes: List<Int>,
        destinationDirectory: String,
        priority: DownloadPriority,
    ): String
    suspend fun torrentTelemetry(downloadId: String): TorrentTelemetry?
    suspend fun torrentDownloadMetadata(downloadId: String): TorrentDownloadMetadata?
    suspend fun selectTorrentFiles(downloadId: String, fileIndexes: List<Int>)
    suspend fun setTorrentFileSelected(downloadId: String, fileIndex: Int, selected: Boolean)
    suspend fun setTorrentSelectionAll(downloadId: String, selected: Boolean)
    suspend fun setTorrentDirectorySelected(downloadId: String, directory: String, selected: Boolean)
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

    /**
     * Wait until the repository has loaded its persisted initial download snapshot.
     * Repositories without asynchronous startup loading can keep the default no-op.
     */
    suspend fun awaitInitialSnapshot() = Unit

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

    suspend fun addDownloadWithSha256(
        url: String,
        fileName: String,
        destinationPath: String,
        expectedSha256: String,
        priority: DownloadPriority = DownloadPriority.DEFAULT,
    ): String = addDownload(url, fileName, destinationPath, priority)

    /** Add a webpage URL to the media extractor instead of the direct-file engine. */
    suspend fun addMediaDownload(
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority = DownloadPriority.DEFAULT,
        formatId: String? = null,
        audioFormat: String? = null,
    ): String {
        if (formatId != null || audioFormat != null) {
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

    /** Move a queued download earlier among downloads with the same priority. */
    suspend fun moveUpInQueue(downloadId: String)

    /** Move a queued download later among downloads with the same priority. */
    suspend fun moveDownInQueue(downloadId: String)

    /** Hold new queue dispatch without interrupting active transfers. */
    suspend fun pauseQueueDispatch() = Unit

    /** Resume starting eligible queued downloads. */
    suspend fun resumeQueueDispatch() = Unit

    /** Read whether new queue dispatch is currently held. */
    suspend fun isQueueDispatchPaused(): Boolean = false

    /** Schedule a download to start at a specific wall-clock time (epoch millis). */
    suspend fun scheduleDownload(downloadId: String, startAtMillis: Long)

    /** Retrieve a single download by ID, or null if not found. */
    suspend fun getDownload(downloadId: String): DownloadItem?
}

/**
 * Optional repository capability for checking persisted URL duplicates.
 *
 * Kept separate from [DownloadRepository] so hosts without shared-core
 * duplicate lookup do not need to provide a synthetic result.
 */
interface DuplicateDownloadRepository {
    /** Returns the existing download filename, or null when no match exists. */
    suspend fun findDuplicateDownload(url: String): String?
}

/**
 * Optional shared-core batch admission capability.
 *
 * Platform adapters may supply explicit paths after applying OS storage policy;
 * admission, collision handling, and per-input results remain in Rust.
 */
interface BatchDownloadRepository {
    suspend fun addDownloadBatch(
        items: List<DownloadBatchItem>,
        priority: DownloadPriority,
        expectedSha256: String? = null,
    ): List<DownloadBatchOutcome>
}
