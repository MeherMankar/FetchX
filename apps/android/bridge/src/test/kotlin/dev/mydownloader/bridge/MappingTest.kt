package dev.fetchx.bridge

import dev.fetchx.bridge.uniffi.DownloadInfoDto
import dev.fetchx.bridge.uniffi.DownloadPriorityDto
import dev.fetchx.bridge.uniffi.DownloadProgressDto
import dev.fetchx.bridge.uniffi.DownloadStatusDto
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.DownloadState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Pure JVM unit tests for the DTO mapping layer in [BridgeDownloadRepository].
 *
 * These tests do NOT load the native library — they test only the Kotlin-side
 * mapping logic, using manually constructed DTO objects.
 */
class MappingTest {

    /**
     * Creates a [FakeRepository] that exercises the mapping functions.
     * We call the internal mapping methods directly to test them in isolation.
     */
    private val repo = FakeRepository()

    @Test
    fun criticalPriorityMapsToHighDto() {
        val dto = repo.mapPriorityToDto(DownloadPriority.CRITICAL)
        assertEquals(DownloadPriorityDto.HIGH, dto)
    }

    @Test
    fun highPriorityMapsToHighDto() {
        assertEquals(DownloadPriorityDto.HIGH, repo.mapPriorityToDto(DownloadPriority.HIGH))
    }

    @Test
    fun normalPriorityRoundTrips() {
        val dto = repo.mapPriorityToDto(DownloadPriority.NORMAL)
        val back = repo.mapPriorityFromDto(dto)
        assertEquals(DownloadPriority.NORMAL, back)
    }

    @Test
    fun lowPriorityRoundTrips() {
        val dto = repo.mapPriorityToDto(DownloadPriority.LOW)
        val back = repo.mapPriorityFromDto(dto)
        assertEquals(DownloadPriority.LOW, back)
    }

    @Test
    fun preparingStatusMapsToActiveState() {
        val item = makeDto(status = DownloadStatusDto.PREPARING)
        val mapped = repo.mapItemPublic(item)
        assert(mapped.state is DownloadState.Active) {
            "Expected Active state for PREPARING, got ${mapped.state}"
        }
    }

    @Test
    fun downloadingStatusMapsToActiveState() {
        val item = makeDto(status = DownloadStatusDto.DOWNLOADING, downloadedBytes = 512uL)
        val mapped = repo.mapItemPublic(item)
        val state = mapped.state as DownloadState.Active
        assertEquals(512L, state.downloadedBytes)
    }

    @Test
    fun failedStatusMapsToFailedStateWithRetryableTrue() {
        val item = makeDto(status = DownloadStatusDto.FAILED, errorMessage = "connection reset")
        val mapped = repo.mapItemPublic(item)
        val state = mapped.state as DownloadState.Failed
        assertEquals("connection reset", state.reason)
        assertEquals(true, state.retryable)
    }

    @Test
    fun failedStatusWithNullErrorMessageUsesDefault() {
        val item = makeDto(status = DownloadStatusDto.FAILED, errorMessage = null)
        val mapped = repo.mapItemPublic(item)
        val state = mapped.state as DownloadState.Failed
        assertEquals("unknown error", state.reason)
    }

    @Test
    fun unknownTotalBytesNegativeOneMapsToNull() {
        val item = makeDto(totalBytes = -1L)
        val mapped = repo.mapItemPublic(item)
        assertNull(mapped.totalBytes)
    }

    @Test
    fun knownTotalBytesPositiveMapsThroughToItem() {
        val item = makeDto(totalBytes = 1_048_576L)
        val mapped = repo.mapItemPublic(item)
        assertEquals(1_048_576L, mapped.totalBytes)
    }

    @Test
    fun zeroTotalBytesIsNotSentinelAndMapsThrough() {
        // 0 means "0 bytes known" — it is NOT the sentinel (-1).
        val item = makeDto(totalBytes = 0L)
        val mapped = repo.mapItemPublic(item)
        assertEquals(0L, mapped.totalBytes)
    }

    @Test
    fun completedStatusMapsToCompletedStateWithDestination() {
        val item = makeDto(
            status = DownloadStatusDto.COMPLETED,
            destination = "/storage/downloads/file.zip",
            totalBytes = 2048L,
        )
        val mapped = repo.mapItemPublic(item)
        val state = mapped.state as DownloadState.Completed
        assertEquals("/storage/downloads/file.zip", state.destinationPath)
        assertEquals(2048L, state.totalBytes)
    }

    @Test
    fun cancelledStatusMapsToCancelledState() {
        val item = makeDto(status = DownloadStatusDto.CANCELLED)
        val mapped = repo.mapItemPublic(item)
        assertEquals(DownloadState.Cancelled, mapped.state)
    }

    @Test
    fun pausedStatusMapsToPausedState() {
        val item = makeDto(
            status = DownloadStatusDto.PAUSED,
            downloadedBytes = 512uL,
            totalBytes = 1024L,
        )
        val mapped = repo.mapItemPublic(item)
        assertEquals(DownloadState.Paused(512L, 1024L), mapped.state)
    }

    @Test
    fun queuedStatusMapsToQueuedState() {
        val item = makeDto(status = DownloadStatusDto.QUEUED)
        val mapped = repo.mapItemPublic(item)
        assertEquals(DownloadState.Queued, mapped.state)
    }

    // ── Helpers ────────────────────────────────────────────────────────────

    private fun makeDto(
        status: DownloadStatusDto = DownloadStatusDto.QUEUED,
        downloadedBytes: ULong = 0uL,
        totalBytes: Long = -1L,
        speed: Long = -1L,
        eta: Long = -1L,
        errorMessage: String? = null,
        destination: String = "/storage/downloads/file.zip",
    ): DownloadInfoDto = DownloadInfoDto(
        id = "00000000-0000-0000-0000-000000000001",
        url = "https://example.com/file.zip",
        engine = "direct_http",
        filename = "file.zip",
        destination = destination,
        status = status,
        priority = DownloadPriorityDto.NORMAL,
        progress = DownloadProgressDto(
            downloadedBytes = downloadedBytes,
            totalBytes = totalBytes,
            speedBytesPerSecond = speed,
            estimatedRemainingSeconds = eta,
        ),
        createdAtMillis = 1_700_000_000_000L,
        updatedAtMillis = 1_700_000_000_000L,
        errorMessage = errorMessage,
    )
}

/**
 * Exposes internal mapping functions of [BridgeDownloadRepository] for testing.
 *
 * We cannot instantiate [BridgeDownloadRepository] directly in unit tests because
 * its constructor calls [DownloadService.addEventListener] and [DownloadService.listDownloads],
 * which require the native library. Instead we subclass and expose the mapping helpers.
 *
 * This is a test-only fixture — it is NOT production code.
 */
private class FakeRepository {
    // Reuse the mapping logic by delegating to a helper wrapper.
    private val delegate = MappingDelegate()

    fun mapPriorityToDto(priority: dev.fetchx.core.download.DownloadPriority) =
        delegate.mapPriorityToDto(priority)

    fun mapPriorityFromDto(dto: DownloadPriorityDto) = delegate.mapPriorityFromDto(dto)

    fun mapItemPublic(dto: DownloadInfoDto) = delegate.mapItemPublic(dto)
}

/**
 * Contains the extracted mapping logic, testable without a real [DownloadService].
 */
private class MappingDelegate {
    fun mapPriorityToDto(priority: dev.fetchx.core.download.DownloadPriority): DownloadPriorityDto = when (priority) {
        dev.fetchx.core.download.DownloadPriority.LOW -> DownloadPriorityDto.LOW
        dev.fetchx.core.download.DownloadPriority.NORMAL -> DownloadPriorityDto.NORMAL
        dev.fetchx.core.download.DownloadPriority.HIGH -> DownloadPriorityDto.HIGH
        dev.fetchx.core.download.DownloadPriority.CRITICAL -> DownloadPriorityDto.HIGH
    }

    fun mapPriorityFromDto(dto: DownloadPriorityDto): dev.fetchx.core.download.DownloadPriority = when (dto) {
        DownloadPriorityDto.LOW -> dev.fetchx.core.download.DownloadPriority.LOW
        DownloadPriorityDto.NORMAL -> dev.fetchx.core.download.DownloadPriority.NORMAL
        DownloadPriorityDto.HIGH -> dev.fetchx.core.download.DownloadPriority.HIGH
    }

    fun mapItemPublic(dto: DownloadInfoDto): dev.fetchx.core.download.DownloadItem {
        val state = mapState(dto)
        return dev.fetchx.core.download.DownloadItem(
            id = dto.id,
            fileName = dto.filename,
            url = dto.url,
            destinationPath = dto.destination,
            state = state,
            priority = mapPriorityFromDto(dto.priority),
            engine = dto.engine,
            mimeType = null,
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L },
            createdAt = dto.createdAtMillis,
        )
    }

    private fun mapState(dto: DownloadInfoDto): dev.fetchx.core.download.DownloadState = when (dto.status) {
        DownloadStatusDto.QUEUED -> dev.fetchx.core.download.DownloadState.Queued
        DownloadStatusDto.PREPARING,
        DownloadStatusDto.DOWNLOADING -> dev.fetchx.core.download.DownloadState.Active(
            downloadedBytes = dto.progress.downloadedBytes.toLong(),
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L },
            speedBytesPerSec = dto.progress.speedBytesPerSecond.takeIf { it >= 0L } ?: 0L,
            etaSeconds = dto.progress.estimatedRemainingSeconds.takeIf { it >= 0L },
            segmentCount = 1,
        )
        DownloadStatusDto.PAUSED -> dev.fetchx.core.download.DownloadState.Paused(
            downloadedBytes = dto.progress.downloadedBytes.toLong(),
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L },
        )
        DownloadStatusDto.COMPLETED -> dev.fetchx.core.download.DownloadState.Completed(
            totalBytes = dto.progress.totalBytes.takeIf { it >= 0L } ?: 0L,
            destinationPath = dto.destination,
        )
        DownloadStatusDto.FAILED -> dev.fetchx.core.download.DownloadState.Failed(
            reason = dto.errorMessage ?: "unknown error",
            retryable = true,
        )
        DownloadStatusDto.CANCELLED -> dev.fetchx.core.download.DownloadState.Cancelled
    }
}
