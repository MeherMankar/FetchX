package dev.fetchx.bridge

import android.util.Log
import dev.fetchx.bridge.uniffi.DownloadEventDto
import dev.fetchx.bridge.uniffi.DownloadEventListener
import dev.fetchx.bridge.uniffi.DownloadInfoDto
import dev.fetchx.bridge.uniffi.BatchDownloadAudioFormatDto
import dev.fetchx.bridge.uniffi.BatchDownloadEngineDto
import dev.fetchx.bridge.uniffi.BatchDownloadErrorCodeDto
import dev.fetchx.bridge.uniffi.BatchDownloadRequestDto
import dev.fetchx.bridge.uniffi.Aria2RecoveryStateDto
import dev.fetchx.bridge.uniffi.DownloadPriorityDto
import dev.fetchx.bridge.uniffi.DownloadService
import dev.fetchx.bridge.uniffi.DownloadStatusDto
import dev.fetchx.bridge.uniffi.TorrentTelemetryDto
import dev.fetchx.bridge.uniffi.TorrentMetadataDto
import dev.fetchx.core.download.TorrentDownloadMetadata
import dev.fetchx.core.download.Aria2RecoveryAssessment
import dev.fetchx.core.download.Aria2RecoveryRepository
import dev.fetchx.core.download.Aria2RecoveryState
import dev.fetchx.core.download.DownloadEvent
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.BatchDownloadRepository
import dev.fetchx.core.download.DownloadBatchError
import dev.fetchx.core.download.DownloadBatchItem
import dev.fetchx.core.download.DownloadBatchOutcome
import dev.fetchx.core.download.DownloadRepository
import dev.fetchx.core.download.DownloadState
import dev.fetchx.core.download.DuplicateDownloadRepository
import dev.fetchx.core.download.TorrentDownloadRepository
import dev.fetchx.core.download.TorrentFile
import dev.fetchx.core.download.TorrentMetadata
import dev.fetchx.core.download.TorrentTelemetry
import dev.fetchx.core.download.YtDlpFormat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.CompletableDeferred
import org.json.JSONObject
import java.io.File
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

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
) : TorrentDownloadRepository, DuplicateDownloadRepository, BatchDownloadRepository,
    Aria2RecoveryRepository {

    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    private val _events = MutableSharedFlow<DownloadEvent>(extraBufferCapacity = 64)
    private val initialSnapshotReady = CompletableDeferred<Unit>()
    private val publicationAttempts = ConcurrentHashMap.newKeySet<String>()
    private val snapshotRefreshWorker = SnapshotRefreshWorker(
        scope = externalScope,
        refresh = {
            val items = withContext(Dispatchers.IO) { loadDownloadsAndReconcile() }
            _downloads.value = items
            initialSnapshotReady.complete(Unit)
        },
        onError = { error ->
            if (initialSnapshotReady.completeExceptionally(error)) {
                Log.e(TAG, "Could not load initial download snapshot", error)
            } else {
                Log.e(TAG, "Could not refresh download snapshot after event", error)
            }
        },
    )

    init {
        // Register the Rust → Kotlin event listener. The callback is invoked on a
        // Rust Tokio thread, so all operations inside it must be thread-safe.
        service.addEventListener(object : DownloadEventListener {
            override fun onEvent(event: DownloadEventDto) {
                externalScope.launch {
                    try {
                        val completedStage = (event as? DownloadEventDto.Updated)
                            ?.item
                            ?.takeIf {
                                it.status == DownloadStatusDto.COMPLETED &&
                                    publicDownloadStorage.isStagingPath(it.destination)
                            }
                        if (completedStage != null) {
                            val stillCompleted = withContext(Dispatchers.IO) {
                                publicDownloadStorage.withPublicationLock {
                                    val current = service.getDownload(completedStage.id)
                                    if (
                                        isCurrentCompletedStagingDestination(
                                            current?.status,
                                            current?.destination,
                                            completedStage.destination,
                                        )
                                    ) {
                                        reconcileCompleted(completedStage.destination)
                                        true
                                    } else {
                                        false
                                    }
                                }
                            }
                            if (stillCompleted) mapEvent(event)?.let { _events.emit(it) }
                        } else {
                            mapEvent(event)?.let { _events.emit(it) }
                        }
                    } catch (cancelled: CancellationException) {
                        throw cancelled
                    } catch (error: Exception) {
                        Log.e(TAG, "Could not process download event", error)
                    } finally {
                        snapshotRefreshWorker.request()
                    }
                }
            }
        })

        snapshotRefreshWorker.request()
    }

    // ── DownloadRepository interface ──────────────────────────────────────

    override fun observeDownloads(): Flow<List<DownloadItem>> = _downloads.asStateFlow()

    fun reservedPublicFileNames(): Set<String> =
        _downloads.value.flatMap { item ->
            listOf(item.fileName, File(item.destinationPath).name)
        }.filter { it.isNotBlank() }.toSet()

    override suspend fun awaitInitialSnapshot() {
        initialSnapshotReady.await()
    }

    override suspend fun findDuplicateDownload(url: String): String? =
        withContext(Dispatchers.IO) {
            service.findDuplicateDownload(url)?.filename
        }

    override suspend fun addDownloadBatch(
        items: List<DownloadBatchItem>,
        priority: DownloadPriority,
        expectedSha256: String?,
    ): List<DownloadBatchOutcome> = withContext(Dispatchers.IO) {
        val requests = items.map { item ->
            toBatchDownloadRequestDto(item, expectedSha256)
        }
        service.enqueueDownloadBatchWithDestinations(requests, mapPriorityToDto(priority))
            .map { outcome ->
                DownloadBatchOutcome(
                    inputUrl = outcome.inputUrl,
                    downloadId = outcome.downloadId,
                    error = outcome.error?.let(::mapBatchDownloadError),
                )
            }
    }

    internal fun toBatchDownloadRequestDto(
        item: DownloadBatchItem,
        expectedSha256: String? = null,
    ): BatchDownloadRequestDto = createBatchDownloadRequestDto(
        item = item,
        expectedSha256 = expectedSha256,
        stageDestination = publicDownloadStorage::stageDestinationIfPublic,
    )

    suspend fun diagnosticsSnapshot(): String = withContext(Dispatchers.IO) {
        service.diagnosticsSnapshot()
    }

    suspend fun dispatchBrowserRpc(requestJson: String): String = withContext(Dispatchers.IO) {
        service.dispatchBrowserRpc(requestJson)
    }

    suspend fun addBrowserDownload(url: String, filename: String?): String =
        withContext(Dispatchers.IO) {
            val requestId = UUID.randomUUID().toString()
            val response = dispatchBrowserDownloadRequest(
                requestId = requestId,
                url = url,
                filename = filename,
                includeFilename = filename != null,
            )
            val error = response.optJSONObject("error")
            val errorCode = error?.optInt("code")
            val compatibleResponse = if (
                filename != null && (errorCode == -32600 || errorCode == -32602)
            ) {
                dispatchBrowserDownloadRequest(
                    requestId = requestId,
                    url = url,
                    filename = filename,
                    includeFilename = false,
                )
            } else {
                response
            }
            val rpcError = compatibleResponse.optJSONObject("error")
            if (rpcError != null) {
                val code = rpcError.optInt("code")
                val message = if (code == -32011 || code == -32603) {
                    "FetchX may have accepted the download. Check Downloads before retrying."
                } else {
                    rpcError.optString("message").ifBlank { "FetchX rejected the download." }
                }
                throw IllegalStateException(message)
            }
            compatibleResponse.optJSONObject("result")
                ?.optString("id")
                ?.takeIf(String::isNotBlank)
                ?: throw IllegalStateException("FetchX returned an invalid download response.")
        }

    private suspend fun dispatchBrowserDownloadRequest(
        requestId: String,
        url: String,
        filename: String?,
        includeFilename: Boolean,
    ): JSONObject {
        val params = JSONObject().put("url", url)
        if (includeFilename) filename?.let { params.put("filename", it) }
        val request = JSONObject()
            .put("jsonrpc", "2.0")
            .put("id", requestId)
            .put("method", "download.create")
            .put("params", params)
        return try {
            JSONObject(dispatchBrowserRpc(request.toString()))
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            throw IllegalStateException(
                "FetchX may have accepted the download. Check Downloads before retrying.",
                error,
            )
        }
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
            service.addMediaDownload(url, fileName, stagingPath, mapPriorityToDto(priority), null, null)
        } else {
            val stagingPath = publicDownloadStorage.stageDestinationIfPublic(destinationPath)
            service.addDownload(url, fileName, stagingPath, mapPriorityToDto(priority))
        }
    }

    override suspend fun addDownloadWithSha256(
        url: String,
        fileName: String,
        destinationPath: String,
        expectedSha256: String,
        priority: DownloadPriority,
    ): String = withContext(Dispatchers.IO) {
        val stagingPath = publicDownloadStorage.stageDestinationIfPublic(destinationPath)
        service.addDownloadWithSha256(
            url,
            fileName,
            stagingPath,
            mapPriorityToDto(priority),
            expectedSha256,
        )
    }

    override suspend fun addMediaDownload(
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority,
        formatId: String?,
        audioFormat: String?,
    ): String = withContext(Dispatchers.IO) {
        val stagingPath = publicDownloadStorage.stageDestinationIfPublic(destinationPath)
        service.addMediaDownload(
            url,
            fileName,
            stagingPath,
            mapPriorityToDto(priority),
            formatId,
            audioFormat,
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

    override suspend fun assessAria2Recovery(downloadId: String): Aria2RecoveryAssessment? =
        withContext(Dispatchers.IO) {
            service.assessAria2Recovery(downloadId)?.let { assessment ->
                Aria2RecoveryAssessment(
                    state = mapAria2RecoveryState(assessment.state),
                    explanation = assessment.explanation,
                    nextStep = assessment.nextStep,
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
        withContext(Dispatchers.IO) {
            publicDownloadStorage.withPublicationLock {
                if (shouldCancelDownload(service.getDownload(downloadId)?.status)) {
                    service.cancelDownload(downloadId)
                }
            }
        }

    override suspend fun removeDownload(downloadId: String, deleteFile: Boolean) =
        withContext(Dispatchers.IO) {
            publicDownloadStorage.withPublicationLock {
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
        }

    override suspend fun retryDownload(downloadId: String): String =
        withContext(Dispatchers.IO) {
            val item = service.getDownload(downloadId)
            if (
                item?.status == DownloadStatusDto.COMPLETED &&
                publicDownloadStorage.isStagingPath(item.destination)
            ) {
                publicationAttempts.remove(item.destination)
                try {
                    publicDownloadStorage.retryPublication(item.destination)
                    publicationAttempts.add(item.destination)
                } catch (error: Exception) {
                    publicationAttempts.add(item.destination)
                    throw error
                }
                _downloads.value = loadDownloadsAndReconcile()
                downloadId
            } else {
                service.retryDownload(downloadId)
            }
        }

    override suspend fun setPriority(downloadId: String, priority: DownloadPriority) =
        withContext(Dispatchers.IO) {
            service.setPriority(downloadId, mapPriorityToDto(priority))
        }

    override suspend fun moveUpInQueue(downloadId: String) =
        withContext(Dispatchers.IO) {
            service.moveDownloadUpInQueue(downloadId)
            _downloads.value = loadDownloadsAndReconcile()
        }

    override suspend fun moveDownInQueue(downloadId: String) =
        withContext(Dispatchers.IO) {
            service.moveDownloadDownInQueue(downloadId)
            _downloads.value = loadDownloadsAndReconcile()
        }

    override suspend fun pauseQueueDispatch() = withContext(Dispatchers.IO) {
        service.pauseQueueDispatch()
    }

    override suspend fun resumeQueueDispatch() = withContext(Dispatchers.IO) {
        service.resumeQueueDispatch()
    }

    override suspend fun isQueueDispatchPaused(): Boolean = withContext(Dispatchers.IO) {
        service.isQueueDispatchPaused()
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
        try {
            publicDownloadStorage.reconcileOrphanedPublications(
                items.asSequence()
                    .filter { it.status == DownloadStatusDto.COMPLETED }
                    .map { it.destination }
                    .filter(publicDownloadStorage::isStagingPath)
                    .toSet(),
            )
        } catch (error: Exception) {
            Log.e(TAG, "Could not clean up stale public download entries", error)
        }
        items.asSequence()
            .filter { it.status == DownloadStatusDto.COMPLETED }
            .forEach { reconcileCompleted(it.destination) }
        return items.map(::mapItem)
    }

    private fun reconcileCompleted(destination: String) {
        if (!publicDownloadStorage.isStagingPath(destination)) {
            return
        }
        try {
            if (publicDownloadStorage.publishedUri(destination) != null) {
                publicDownloadStorage.cleanupPublishedStagingArtifacts(destination)
                return
            }
            if (!publicationAttempts.add(destination)) return
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

internal fun mapAria2RecoveryState(dto: Aria2RecoveryStateDto): Aria2RecoveryState = when (dto) {
    Aria2RecoveryStateDto.NOT_ARIA2 -> Aria2RecoveryState.NOT_ARIA2
    Aria2RecoveryStateDto.NOT_REQUIRED -> Aria2RecoveryState.NOT_REQUIRED
    Aria2RecoveryStateDto.RUNTIME_UNAVAILABLE -> Aria2RecoveryState.RUNTIME_UNAVAILABLE
    Aria2RecoveryStateDto.READY_TO_START -> Aria2RecoveryState.READY_TO_START
    Aria2RecoveryStateDto.JOB_AVAILABLE -> Aria2RecoveryState.JOB_AVAILABLE
    Aria2RecoveryStateDto.MISSING_TORRENT_METADATA -> Aria2RecoveryState.MISSING_TORRENT_METADATA
    Aria2RecoveryStateDto.MISSING_GID -> Aria2RecoveryState.MISSING_GID
    Aria2RecoveryStateDto.STALE_GID -> Aria2RecoveryState.STALE_GID
    Aria2RecoveryStateDto.MISSING_CONTROL_FILE -> Aria2RecoveryState.MISSING_CONTROL_FILE
    Aria2RecoveryStateDto.CONTROL_FILE_PRESENT -> Aria2RecoveryState.CONTROL_FILE_PRESENT
    Aria2RecoveryStateDto.UNKNOWN -> Aria2RecoveryState.UNKNOWN
}

private fun String.withExtension(extension: String): String {
    val nameStart = maxOf(lastIndexOf('/'), lastIndexOf('\\')) + 1
    val name = substring(nameStart)
    val stem = name.substringBeforeLast('.', name)
    return substring(0, nameStart) + "$stem.$extension"
}

internal fun createBatchDownloadRequestDto(
    item: DownloadBatchItem,
    expectedSha256: String?,
    stageDestination: (String) -> String,
): BatchDownloadRequestDto {
    val media = item.isMedia || isMediaUrl(item.url) ||
        item.formatId != null || item.extension != null || item.audioFormat != null
    val selectedExtension = item.audioFormat?.lowercase() ?: item.extension
    val audioFormat = when (item.audioFormat?.lowercase()) {
        "mp3" -> BatchDownloadAudioFormatDto.MP3
        "m4a" -> BatchDownloadAudioFormatDto.M4A
        null -> null
        else -> null
    }
    val desiredDestination = when {
        selectedExtension != null -> item.destinationPath.withExtension(selectedExtension)
        media && item.formatId == null ->
            if (item.destinationPath.endsWith(".mp4", ignoreCase = true)) item.destinationPath
            else item.destinationPath.withExtension("mp4")
        else -> item.destinationPath
    }
    val stagedDestination = stageDestination(desiredDestination)
    return BatchDownloadRequestDto(
        url = item.url,
        filename = selectedExtension?.let { item.fileName.withExtension(it) } ?: item.fileName,
        destination = stagedDestination,
        engine = if (media) BatchDownloadEngineDto.YT_DLP else BatchDownloadEngineDto.DIRECT_HTTP,
        formatId = item.formatId,
        extension = selectedExtension,
        audioFormat = audioFormat,
        expectedSha256 = expectedSha256,
    )
}

private fun mapBatchDownloadError(error: BatchDownloadErrorCodeDto): DownloadBatchError =
    when (error) {
        BatchDownloadErrorCodeDto.DUPLICATE_URL -> DownloadBatchError.DUPLICATE_URL
        BatchDownloadErrorCodeDto.INVALID_URL -> DownloadBatchError.INVALID_URL
        BatchDownloadErrorCodeDto.INVALID_PACKAGE_NAME -> DownloadBatchError.INVALID_PACKAGE_NAME
        BatchDownloadErrorCodeDto.INVALID_FORMAT -> DownloadBatchError.INVALID_FORMAT
        BatchDownloadErrorCodeDto.INVALID_CHECKSUM -> DownloadBatchError.INVALID_CHECKSUM
        BatchDownloadErrorCodeDto.INVALID_DESTINATION -> DownloadBatchError.INVALID_DESTINATION
        BatchDownloadErrorCodeDto.UNAVAILABLE_ENGINE -> DownloadBatchError.UNAVAILABLE_ENGINE
        BatchDownloadErrorCodeDto.DESTINATION -> DownloadBatchError.DESTINATION
        BatchDownloadErrorCodeDto.QUEUE -> DownloadBatchError.QUEUE
        BatchDownloadErrorCodeDto.STORAGE -> DownloadBatchError.STORAGE
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

internal fun isCurrentCompletedStagingDestination(
    currentStatus: DownloadStatusDto?,
    currentDestination: String?,
    eventDestination: String,
): Boolean =
    currentStatus == DownloadStatusDto.COMPLETED && currentDestination == eventDestination

internal fun shouldCancelDownload(currentStatus: DownloadStatusDto?): Boolean =
    currentStatus != DownloadStatusDto.COMPLETED

internal class SnapshotRefreshWorker(
    scope: CoroutineScope,
    private val refresh: suspend () -> Unit,
    private val onError: (Exception) -> Unit,
) {
    private val requests = Channel<Unit>(Channel.CONFLATED)

    init {
        scope.launch {
            for (ignored in requests) {
                try {
                    refresh()
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (error: Exception) {
                    onError(error)
                }
            }
        }
    }

    fun request() {
        check(requests.trySend(Unit).isSuccess) {
            "Could not queue download snapshot refresh"
        }
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
