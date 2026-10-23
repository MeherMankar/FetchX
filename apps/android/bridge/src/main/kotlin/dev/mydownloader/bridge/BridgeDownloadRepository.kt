package dev.fetchx.bridge

import android.util.Log
import dev.fetchx.bridge.uniffi.DownloadEventDto
import dev.fetchx.bridge.uniffi.DownloadEventListener
import dev.fetchx.bridge.uniffi.DownloadInfoDto
import dev.fetchx.bridge.uniffi.DownloadPriorityDto
import dev.fetchx.bridge.uniffi.DownloadService
import dev.fetchx.bridge.uniffi.DownloadStatusDto
import dev.fetchx.bridge.uniffi.TorrentTelemetryDto
import dev.fetchx.bridge.uniffi.TorrentMetadataDto
import dev.fetchx.core.download.TorrentDownloadMetadata
import dev.fetchx.core.download.DownloadEvent
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.DownloadRepository
import dev.fetchx.core.download.DownloadState
import dev.fetchx.core.download.TorrentDownloadRepository
import dev.fetchx.core.download.TorrentFile
import dev.fetchx.core.download.TorrentMetadata
import dev.fetchx.core.download.TorrentTelemetry
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
import kotlinx.coroutines.CompletableDeferred
import java.util.concurrent.atomic.AtomicLong

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
    private val publicDownloadStorage: PublicDownloadStorage,
    private val externalScope: CoroutineScope =
        CoroutineScope(SupervisorJob() + Dispatchers.IO),
) : TorrentDownloadRepository {

    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    private val _events = MutableSharedFlow<DownloadEvent>(extraBufferCapacity = 64)
    private val initialSnapshotReady = CompletableDeferred<Unit>()
    private val refreshGeneration = AtomicLong()

    init {
        // Register the Rust → Kotlin event listener. The callback is invoked on a
        // Rust Tokio thread, so all operations inside it must be thread-safe.
        service.addEventListener(object : DownloadEventListener {
            override fun onEvent(event: DownloadEventDto) {
                val generation = refreshGeneration.incrementAndGet()
                externalScope.launch {
                    try {
                        val completedStage = (event as? DownloadEventDto.Updated)
                            ?.item
                            ?.takeIf {
                                it.status == DownloadStatusDto.COMPLETED &&
                                    publicDownloadStorage.isStagingPath(it.destination)
                            }
                        if (completedStage != null) {
                            try {
                                withContext(Dispatchers.IO) {
                                    publicDownloadStorage.publishCompleted(completedStage.destination)
                                }
                                mapEvent(event)?.let { _events.emit(it) }
                            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                                throw cancelled
                            } catch (error: Exception) {
                                Log.e(TAG, "Could not publish completed download to public storage", error)
                            }
                        } else {
                            mapEvent(event)?.let { _events.emit(it) }
                        }
                        val items = withContext(Dispatchers.IO) {
                            loadDownloadsAndReconcile()
                        }
                        if (generation == refreshGeneration.get()) _downloads.value = items
                    } catch (cancelled: kotlinx.coroutines.CancellationException) {
                        throw cancelled
                    } catch (error: Exception) {
                        Log.e(TAG, "Could not refresh download snapshot after event", error)
                    }
                }
            }
        })

        // Load initial state.
        externalScope.launch {
            try {
                val generation = refreshGeneration.incrementAndGet()
                val items = withContext(Dispatchers.IO) {
                    loadDownloadsAndReconcile()
                }
                if (generation == refreshGeneration.get()) _downloads.value = items
                initialSnapshotReady.complete(Unit)
            } catch (error: Exception) {
                initialSnapshotReady.completeExceptionally(error)
                Log.e(TAG, "Could not load initial download snapshot", error)
                throw error
            }
        }
    }

    // ── DownloadRepository interface ──────────────────────────────────────

    override fun observeDownloads(): Flow<List<DownloadItem>> = _downloads.asStateFlow()

    suspend fun awaitInitialSnapshot() {
        initialSnapshotReady.await()
    }

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
        if (url.trimStart().startsWith("magnet:", ignoreCase = true)) {
            service.addMagnetDownload(url.trim(), mapPriorityToDto(priority))
        } else if (!url.contains("://") && url.trim().endsWith(".torrent", ignoreCase = true)) {
            service.addTorrentFileDownload(url.trim(), mapPriorityToDto(priority))
        } else if (isMediaUrl(url)) {
            val safeDestination = if (destinationPath.endsWith(".mp4", ignoreCase = true)) destinationPath else "$destinationPath.mp4"
            val stagingPath = publicDownloadStorage.stageDestinationIfPublic(safeDestination)
            service.addMediaDownload(url, fileName, stagingPath, mapPriorityToDto(priority), null)
        } else {
            val stagingPath = publicDownloadStorage.stageDestinationIfPublic(destinationPath)
            service.addDownload(url, fileName, stagingPath, mapPriorityToDto(priority))
        }
    }

    override suspend fun addMediaDownload(
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority,
        formatId: String?,
    ): String = withContext(Dispatchers.IO) {
        val stagingPath = publicDownloadStorage.stageDestinationIfPublic(destinationPath)
        service.addMediaDownload(
            url,
            fileName,
            stagingPath,
            mapPriorityToDto(priority),
            formatId,
        )
    }

    override suspend fun discoverMediaFormats(url: String): List<YtDlpFormat> =
        withContext(Dispatchers.IO) {
            service.discoverMediaFormats(url).map { format ->
                YtDlpFormat(
                    format.formatId,
                    format.label,
                    format.extension,
                    format.title,
                    format.platform,
                )
            }
        }

    override suspend fun addMagnet(
        magnetUri: String,
        priority: DownloadPriority,
    ): String = withContext(Dispatchers.IO) {
        service.addMagnetDownload(magnetUri, mapPriorityToDto(priority))
    }

    override suspend fun torrentDownloadMetadata(
        downloadId: String,
    ): TorrentDownloadMetadata? = withContext(Dispatchers.IO) {
        service.getTorrentDownloadMetadata(downloadId)?.let { value ->
            TorrentDownloadMetadata(
                metadataState = value.metadataState,
                metadata = value.metadata?.let(::mapTorrentMetadata),
                selectedFileIndexes = value.selectedFileIndexes.map(UInt::toInt),
                selectedTotalBytes = value.selectedTotalBytes.toLong(),
                selectionConfirmed = value.selectionConfirmed,
            )
        }
    }

    override suspend fun selectTorrentFiles(downloadId: String, fileIndexes: List<Int>) =
        withContext(Dispatchers.IO) {
            service.selectTorrentFiles(downloadId, fileIndexes.map(Int::toUInt))
        }

    override suspend fun setTorrentFileSelected(
        downloadId: String,
        fileIndex: Int,
        selected: Boolean,
    ) = withContext(Dispatchers.IO) {
        service.setTorrentFileSelected(downloadId, fileIndex.toUInt(), selected)
    }

    override suspend fun setTorrentSelectionAll(downloadId: String, selected: Boolean) =
        withContext(Dispatchers.IO) {
            service.setTorrentSelectionAll(downloadId, selected)
        }

    override suspend fun setTorrentDirectorySelected(
        downloadId: String,
        directory: String,
        selected: Boolean,
    ) = withContext(Dispatchers.IO) {
        service.setTorrentDirectorySelected(downloadId, directory, selected)
    }

    override suspend fun torrentTelemetry(downloadId: String): TorrentTelemetry? =
        withContext(Dispatchers.IO) {
            service.getTorrentTelemetry(downloadId)?.let { telemetry ->
                TorrentTelemetry(
                    downloadSpeedBytesPerSecond = telemetry.downloadSpeedBps?.toLong(),
                    uploadSpeedBytesPerSecond = telemetry.uploadSpeedBps?.toLong(),
                    downloadedBytes = telemetry.downloadedBytes?.toLong(),
                    uploadedBytes = telemetry.uploadedBytes?.toLong(),
                    peerCount = telemetry.peerCount?.toLong(),
                    completedPeerCount = telemetry.completedPeerCount?.toLong(),
                    seedCount = telemetry.seedCount?.toLong(),
                    shareRatio = telemetry.shareRatio,
                    seeding = telemetry.seeding,
                    trackerUris = telemetry.trackerUris,
                )
            }
        }

    override suspend fun inspectTorrentFile(torrentPath: String): TorrentMetadata =
        withContext(Dispatchers.IO) {
            mapTorrentMetadata(service.inspectTorrentFile(torrentPath))
        }

    override suspend fun addTorrentFile(
        torrentPath: String,
        selectedFileIndexes: List<Int>,
        destinationDirectory: String,
        priority: DownloadPriority,
    ): String = withContext(Dispatchers.IO) {
        val destination = publicDownloadStorage.torrentDirectoryIfPublic(destinationDirectory)
        service.addTorrentFileDownloadSelected(
            torrentPath,
            mapPriorityToDto(priority),
            selectedFileIndexes.map(Int::toUInt),
            destination,
        )
    }

    override suspend fun stopTorrentSeeding(downloadId: String) =
        withContext(Dispatchers.IO) { service.stopTorrentSeeding(downloadId) }

    override suspend fun pauseDownload(downloadId: String) =
        withContext(Dispatchers.IO) { service.pauseDownload(downloadId) }

    override suspend fun resumeDownload(downloadId: String) =
        withContext(Dispatchers.IO) { service.resumeDownload(downloadId) }

    override suspend fun cancelDownload(downloadId: String, deleteFile: Boolean) =
        withContext(Dispatchers.IO) { service.cancelDownload(downloadId) }

    override suspend fun removeDownload(downloadId: String, deleteFile: Boolean) =
        withContext(Dispatchers.IO) {
            val destination = service.getDownload(downloadId)?.destination
            service.removeDownload(downloadId)
            if (destination != null && publicDownloadStorage.isStagingPath(destination)) {
                if (deleteFile) {
                    publicDownloadStorage.deletePublished(destination)
                } else {
                    publicDownloadStorage.forgetPublication(destination)
                }
            }
        }

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
            service.getDownload(downloadId)?.let { dto ->
                if (dto.status == DownloadStatusDto.COMPLETED) {
                    reconcileCompleted(dto.destination)
                }
                mapItem(dto)
            }
        }

    suspend fun hasRelevantDownloads(): Boolean = withContext(Dispatchers.IO) {
        loadDownloadsAndReconcile().any { item ->
            item.state is DownloadState.Active || item.state is DownloadState.Queued
        }
    }

    private fun loadDownloadsAndReconcile(): List<DownloadItem> {
        val items = service.listDownloads()
        items.asSequence()
            .filter { it.status == DownloadStatusDto.COMPLETED }
            .forEach { reconcileCompleted(it.destination) }
        return items.map(::mapItem)
    }

    private fun reconcileCompleted(destination: String) {
        if (!publicDownloadStorage.isStagingPath(destination)) {
            return
        }
        if (publicDownloadStorage.publishedUri(destination) != null) {
            publicDownloadStorage.cleanupPublishedStagingArtifacts(destination)
            return
        }
        try {
            publicDownloadStorage.reconcileCompleted(destination)
        } catch (error: Exception) {
            Log.e(TAG, "Could not reconcile public download publication", error)
        }
    }

    // ── Mapping helpers ───────────────────────────────────────────────────

    /**
     * Converts a [DownloadInfoDto] from Rust into the Android [DownloadItem].
     * Sentinel values (-1) for numeric optional fields are mapped to null.
     */
    internal fun mapItem(dto: DownloadInfoDto): DownloadItem {
        val destination = publicDownloadStorage.displayDestination(dto.destination)
        return DownloadItem(
            id = dto.id,
            fileName = dto.filename,
            url = dto.url,
            destinationPath = destination,
            state = mapState(dto, destination),
            priority = mapPriorityFromDto(dto.priority),
            engine = dto.engine,
            mimeType = null,
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L },
            createdAt = dto.createdAtMillis,
        )
    }

    private fun mapState(dto: DownloadInfoDto, displayDestination: String): DownloadState = when (dto.status) {
        DownloadStatusDto.QUEUED -> DownloadState.Queued
        DownloadStatusDto.METADATA_ACQUIRING -> DownloadState.MetadataAcquiring(
            downloadedBytes = dto.progress.downloadedBytes.toLong(),
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L },
        )
        DownloadStatusDto.PREPARING,
        DownloadStatusDto.DOWNLOADING -> DownloadState.Active(
            downloadedBytes = dto.progress.downloadedBytes.toLong(),
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L },
            speedBytesPerSec = dto.progress.speedBytesPerSecond.takeIf { it >= 0L } ?: 0L,
            etaSeconds = dto.progress.estimatedRemainingSeconds.takeIf { it >= 0L },
            segmentCount = 1,
        )
        DownloadStatusDto.PAUSED -> DownloadState.Paused(
            downloadedBytes = dto.progress.downloadedBytes.toLong(),
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L },
        )
        DownloadStatusDto.COMPLETED -> {
            val totalBytes = dto.progress.totalBytes.takeIf { it >= 0L } ?: 0L
            completedDownloadState(
                totalBytes = totalBytes,
                destinationPath = displayDestination,
                isPublicStagingPath = publicDownloadStorage.isStagingPath(dto.destination),
                isPublished = publicDownloadStorage.publishedUri(dto.destination) != null,
            )
        }
        DownloadStatusDto.FAILED -> DownloadState.Failed(
            reason = dto.errorMessage ?: "unknown error",
            retryable = true,
        )
        DownloadStatusDto.CANCELLED -> DownloadState.Cancelled
        DownloadStatusDto.SEEDING -> DownloadState.Seeding(
            downloadedBytes = dto.progress.downloadedBytes.toLong(),
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L },
        )
        DownloadStatusDto.RECOVERING -> DownloadState.Recovering
        DownloadStatusDto.STOPPED -> DownloadState.Stopped
    }

    private companion object {
        const val TAG = "BridgeDownloadRepo"
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

internal fun completedDownloadState(
    totalBytes: Long,
    destinationPath: String,
    isPublicStagingPath: Boolean,
    isPublished: Boolean,
): DownloadState =
    if (isPublicStagingPath && !isPublished) {
        DownloadState.PublicationPending(totalBytes, destinationPath)
    } else {
        DownloadState.Completed(totalBytes, destinationPath)
    }

internal fun isMediaUrl(url: String): Boolean {
    val host = try {
        java.net.URI(url).host?.lowercase() ?: ""
    } catch (_: Exception) {
        ""
    }
    return host.contains("youtube.com") || host == "youtu.be"
}

private fun mapTorrentMetadata(metadata: TorrentMetadataDto) = TorrentMetadata(
    name = metadata.name,
    infoHash = metadata.infoHash,
    totalSizeBytes = metadata.totalSizeBytes.toLong(),
    files = metadata.files.map { file ->
        TorrentFile(
            index = file.index.toInt(),
            path = file.path,
            sizeBytes = file.sizeBytes.toLong(),
        )
    },
    trackers = metadata.trackers,
    pieceLength = metadata.pieceLength.toLong(),
    pieceCount = metadata.pieceCount.toLong(),
)
