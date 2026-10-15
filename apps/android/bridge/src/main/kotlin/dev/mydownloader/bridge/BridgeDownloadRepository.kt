package dev.fetchx.bridge

import dev.fetchx.bridge.uniffi.DownloadEventDto
import dev.fetchx.bridge.uniffi.DownloadEventListener
import dev.fetchx.bridge.uniffi.DownloadInfoDto
import dev.fetchx.bridge.uniffi.DownloadPriorityDto
import dev.fetchx.bridge.uniffi.DownloadService
import dev.fetchx.bridge.uniffi.DownloadStatusDto
import dev.fetchx.core.download.DownloadEvent
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.DownloadRepository
import dev.fetchx.core.download.DownloadState
import dev.fetchx.core.download.YtDlpFormat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Implements [DownloadRepository] by delegating to the Rust [DownloadService]
 * via the UniFFI-generated bindings.
 *
 * Threading model:
 * - All UniFFI calls are dispatched to [Dispatchers.IO] via [withContext].
 *   The Rust bridge is synchronous (blocking); it must never run on the UI thread.
 * - Event delivery is driven by the Rust [DownloadEventListener] callback, which
 *   is invoked on a Rust Tokio thread. Events are pushed to thread-safe flows.
 * - [observeDownloads] returns a [StateFlow] that reflects the full download
 *   list, refreshed after each Rust event.
 *
 * Ownership:
 * - [service] is owned by [BridgeInitializer]; this class holds a shared reference.
 * - After [BridgeInitializer.shutdown], any call on the repository will receive
 *   a MobileApiError.Runtime indicating the service is shut down.
 *
 * Priority note:
 * - [DownloadPriority.CRITICAL] has no Rust equivalent. It is mapped to
 *   [DownloadPriorityDto.HIGH] at the bridge boundary.
 */
class BridgeDownloadRepository(
    private val service: DownloadService,
    private val externalScope: CoroutineScope =
        CoroutineScope(SupervisorJob() + Dispatchers.IO),
) : DownloadRepository {

    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    private val _events = MutableSharedFlow<DownloadEvent>(extraBufferCapacity = 64)

    init {
        // Register the Rust → Kotlin event listener. The callback is invoked on a
        // Rust Tokio thread, so all operations inside it must be thread-safe.
        service.addEventListener(object : DownloadEventListener {
            override fun onEvent(event: DownloadEventDto) {
                mapEvent(event)?.let { _events.tryEmit(it) }
                // Refresh the snapshot on every event so observeDownloads() stays current.
                externalScope.launch {
                    val items = withContext(Dispatchers.IO) {
                        service.listDownloads().map(::mapItem)
                    }
                    _downloads.value = items
                }
            }
        })

        // Load initial state.
        externalScope.launch {
            val items = withContext(Dispatchers.IO) {
                service.listDownloads().map(::mapItem)
            }
            _downloads.value = items
        }
    }

    // ── DownloadRepository interface ──────────────────────────────────────

    override fun observeDownloads(): Flow<List<DownloadItem>> = _downloads.asStateFlow()

    override fun observeEvents(): Flow<DownloadEvent> = _events.asSharedFlow()

    override suspend fun addDownload(
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority,
        mimeType: String?,
        totalBytes: Long?,
        referrer: String?,
    ): String = withContext(Dispatchers.IO) {
        if (isMediaUrl(url)) {
            val safeDestination = if (destinationPath.endsWith(".mp4", ignoreCase = true)) destinationPath else "$destinationPath.mp4"
            service.addMediaDownload(url, fileName, safeDestination, mapPriorityToDto(priority), null)
        } else {
            service.addDownload(url, fileName, destinationPath, mapPriorityToDto(priority))
        }
    }

    override suspend fun addMediaDownload(
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority,
        formatId: String?,
    ): String = withContext(Dispatchers.IO) {
        service.addMediaDownload(
            url,
            fileName,
            destinationPath,
            mapPriorityToDto(priority),
            formatId,
        )
    }

    override suspend fun discoverMediaFormats(url: String): List<YtDlpFormat> =
        withContext(Dispatchers.IO) {
            service.discoverMediaFormats(url).map { format ->
                YtDlpFormat(format.formatId, format.label, format.extension)
            }
    }

    override suspend fun pauseDownload(downloadId: String) =
        withContext(Dispatchers.IO) { service.pauseDownload(downloadId) }

    override suspend fun resumeDownload(downloadId: String) =
        withContext(Dispatchers.IO) { service.resumeDownload(downloadId) }

    override suspend fun cancelDownload(downloadId: String, deleteFile: Boolean) =
        withContext(Dispatchers.IO) { service.cancelDownload(downloadId) }

    override suspend fun removeDownload(downloadId: String, deleteFile: Boolean) =
        withContext(Dispatchers.IO) { service.removeDownload(downloadId) }

    override suspend fun retryDownload(downloadId: String): String =
        withContext(Dispatchers.IO) { service.retryDownload(downloadId) }

    override suspend fun setPriority(downloadId: String, priority: DownloadPriority) =
        withContext(Dispatchers.IO) {
            service.setPriority(downloadId, mapPriorityToDto(priority))
        }

    override suspend fun scheduleDownload(downloadId: String, startAtMillis: Long) =
        withContext(Dispatchers.IO) {
            service.scheduleDownload(downloadId, startAtMillis)
        }

    override suspend fun getDownload(downloadId: String): DownloadItem? =
        withContext(Dispatchers.IO) {
            service.getDownload(downloadId)?.let(::mapItem)
        }

    suspend fun hasRelevantDownloads(): Boolean = withContext(Dispatchers.IO) {
        service.listDownloads().map(::mapItem).any { item ->
            item.state is DownloadState.Active || item.state is DownloadState.Queued
        }
    }

    // ── Mapping helpers ───────────────────────────────────────────────────

    /**
     * Converts a [DownloadInfoDto] from Rust into the Android [DownloadItem].
     * Sentinel values (-1) for numeric optional fields are mapped to null.
     */
    internal fun mapItem(dto: DownloadInfoDto): DownloadItem = DownloadItem(
        id = dto.id,
        fileName = dto.filename,
        url = dto.url,
        destinationPath = dto.destination,
        state = mapState(dto),
        priority = mapPriorityFromDto(dto.priority),
        engine = dto.engine,
        mimeType = null,
        totalBytes = dto.progress.totalBytes.takeIf { it >= 0L },
        createdAt = dto.createdAtMillis,
    )

    private fun mapState(dto: DownloadInfoDto): DownloadState = when (dto.status) {
        DownloadStatusDto.QUEUED -> DownloadState.Queued
        DownloadStatusDto.PREPARING,
        DownloadStatusDto.DOWNLOADING -> DownloadState.Active(
            downloadedBytes = dto.progress.downloadedBytes.toLong(),
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L },
            speedBytesPerSec = dto.progress.speedBytesPerSecond.takeIf { it >= 0L } ?: 0L,
            etaSeconds = dto.progress.estimatedRemainingSeconds.takeIf { it >= 0L },
            segmentCount = 1,
        )
        DownloadStatusDto.PAUSED -> DownloadState.Paused
        DownloadStatusDto.COMPLETED -> DownloadState.Completed(
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L } ?: 0L,
            destinationPath = dto.destination,
        )
        DownloadStatusDto.FAILED -> DownloadState.Failed(
            reason = dto.errorMessage ?: "unknown error",
            retryable = true,
        )
        DownloadStatusDto.CANCELLED -> DownloadState.Cancelled
    }

    /**
     * Maps Rust priority DTO to Android priority.
     */
    internal fun mapPriorityFromDto(dto: DownloadPriorityDto): DownloadPriority = when (dto) {
        DownloadPriorityDto.LOW -> DownloadPriority.LOW
        DownloadPriorityDto.NORMAL -> DownloadPriority.NORMAL
        DownloadPriorityDto.HIGH -> DownloadPriority.HIGH
    }

    /**
     * Maps Android priority to Rust priority DTO.
     * [DownloadPriority.CRITICAL] maps to [DownloadPriorityDto.HIGH] since
     * the Rust domain model has no CRITICAL variant.
     */
    internal fun mapPriorityToDto(priority: DownloadPriority): DownloadPriorityDto = when (priority) {
        DownloadPriority.LOW -> DownloadPriorityDto.LOW
        DownloadPriority.NORMAL -> DownloadPriorityDto.NORMAL
        DownloadPriority.HIGH -> DownloadPriorityDto.HIGH
        DownloadPriority.CRITICAL -> DownloadPriorityDto.HIGH  // CRITICAL → High; documented limitation
    }

    /**
     * Maps a [DownloadEventDto] from Rust to a Kotlin [DownloadEvent].
     * Returns null for event types that have no Kotlin counterpart.
     */
    internal fun mapEvent(dto: DownloadEventDto): DownloadEvent? = when (dto) {
        is DownloadEventDto.Created -> {
            val item = mapItem(dto.item)
            if (item.state is DownloadState.Active) DownloadEvent.Started(item.id) else null
        }
        is DownloadEventDto.Updated -> {
            val item = mapItem(dto.item)
            when (val state = item.state) {
                is DownloadState.Active -> DownloadEvent.Progress(
                    downloadId = item.id,
                    downloadedBytes = state.downloadedBytes,
                    totalBytes = state.totalBytes,
                    speedBytesPerSec = state.speedBytesPerSec,
                    etaSeconds = state.etaSeconds,
                )
                is DownloadState.Paused -> DownloadEvent.Paused(item.id)
                is DownloadState.Completed -> DownloadEvent.Completed(
                    downloadId = item.id,
                    destinationPath = item.destinationPath,
                    totalBytes = state.totalBytes,
                )
                is DownloadState.Failed -> DownloadEvent.Failed(
                    downloadId = item.id,
                    reason = state.reason,
                    retryable = state.retryable,
                )
                is DownloadState.Cancelled -> DownloadEvent.Cancelled(item.id)
                else -> null
            }
        }
        is DownloadEventDto.Removed -> null
    }
}

internal fun isMediaUrl(url: String): Boolean {
    val host = try {
        java.net.URI(url).host?.lowercase() ?: ""
    } catch (_: Exception) {
        ""
    }
    return host.contains("youtube.com") || host == "youtu.be"
}
