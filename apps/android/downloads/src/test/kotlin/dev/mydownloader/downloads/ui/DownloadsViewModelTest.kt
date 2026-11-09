package dev.fetchx.downloads.ui

import androidx.lifecycle.viewModelScope
import dev.fetchx.core.download.DownloadEvent
import dev.fetchx.core.download.BatchDownloadRepository
import dev.fetchx.core.download.DownloadBatchError
import dev.fetchx.core.download.DownloadBatchItem
import dev.fetchx.core.download.DownloadBatchOutcome
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.DownloadRepository
import dev.fetchx.core.download.DownloadState
import dev.fetchx.core.download.DuplicateDownloadRepository
import dev.fetchx.core.download.TorrentDownloadMetadata
import dev.fetchx.core.download.TorrentDownloadRepository
import dev.fetchx.core.download.TorrentMetadata
import dev.fetchx.core.download.TorrentTelemetry
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

@OptIn(ExperimentalCoroutinesApi::class)
class DownloadsViewModelTest {

    private val testDispatcher = UnconfinedTestDispatcher()

    @Before
    fun setUp() {
        Dispatchers.setMain(testDispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    // ── 1. Empty state ────────────────────────────────────────────────────

    @Test
    fun emptyStateWhenNoDl() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)
        // Collect to activate the flow, then assert.
        val result = backgroundScope.launch(testDispatcher) {
            // Start collecting so the WhileSubscribed flow is active.
            vm.allDownloads.collect { }
        }
        assertEquals(emptyList(), vm.allDownloads.value)
        result.cancel()
    }

    // ── 2. Download appears in list ────────────────────────────────────────

    @Test
    fun downloadAppearsInList() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)

        // Subscribe to activate the WhileSubscribed flow before emitting.
        val job = backgroundScope.launch(testDispatcher) { vm.allDownloads.collect { } }

        val item = makeItem("id-1", DownloadState.Queued)
        repo.emit(listOf(item))

        assertEquals(1, vm.allDownloads.value.size)
        assertEquals("id-1", vm.allDownloads.value.first().id)
        job.cancel()
    }

    // ── 3. addDownload calls repository ──────────────────────────────────

    @Test
    fun addDownloadCallsRepository() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)
        vm.addDownload(
            "https://example.com/file.zip",
            "file.zip",
            "/dl/file.zip",
            DownloadPriority.NORMAL,
        )
        assertEquals("https://example.com/file.zip", repo.lastAddUrl)
    }

    @Test
    fun addDownloadBatchPreservesInputsAndReportsPartialFailure() = runTest {
        val repo = FakeDownloadRepository().apply {
            batchOutcomes = listOf(
                DownloadBatchOutcome("https://example.com/one.zip", "id-one", null),
                DownloadBatchOutcome(
                    "https://example.com/two.zip",
                    null,
                    DownloadBatchError.INVALID_URL,
                ),
            )
        }
        val vm = DownloadsViewModel(repo)
        val items = listOf(
            DownloadBatchItem(
                "https://example.com/one.zip",
                "one.zip",
                "/public/Pack/one.zip",
            ),
            DownloadBatchItem(
                "https://example.com/two.zip",
                "two.zip",
                "/public/Pack/two.zip",
            ),
        )

        vm.addDownloadBatch(items, DownloadPriority.HIGH, "a".repeat(64))

        assertEquals(items, repo.lastBatchItems)
        assertEquals(DownloadPriority.HIGH, repo.lastBatchPriority)
        assertEquals("a".repeat(64), repo.lastBatchChecksum)
        assertEquals("1 of 2 downloads could not be fully added. Check Downloads for accepted items.", vm.uiState.value.error)
        assertFalse(vm.uiState.value.isLoading)
    }

    @Test
    fun findDuplicateDownloadUsesSharedRepositoryCapability() = runTest {
        val repo = FakeDownloadRepository().apply { duplicateFileName = "file.zip" }
        val vm = DownloadsViewModel(repo)

        assertEquals("file.zip", vm.findDuplicateDownload("https://example.com/file.zip"))
        assertEquals("https://example.com/file.zip", repo.lastDuplicateLookupUrl)
    }

    @Test
    fun findDuplicateDownloadReturnsNullWhenThereIsNoMatch() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)

        assertNull(vm.findDuplicateDownload("https://example.com/file.zip"))
    }

    @Test
    fun duplicateLookupFailureIsNotConvertedToNoMatch() = runTest {
        val repo = FakeDownloadRepository().apply {
            duplicateLookupFailure = IllegalStateException("storage unavailable")
        }
        val vm = DownloadsViewModel(repo)

        val error = runCatching {
            vm.findDuplicateDownload("https://example.com/file.zip")
        }.exceptionOrNull()
        assertEquals("storage unavailable", error?.message)
    }

    // ── 4. pause calls repository ──────────────────────────────────────────

    @Test
    fun pauseCallsRepository() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)
        vm.pause("id1")
        assertEquals("id1", repo.lastPausedId)
    }

    @Test
    fun unsupportedPauseShowsMessageInsteadOfThrowing() = runTest {
        val repo = FakeDownloadRepository().apply {
            actionFailure = IllegalStateException(
                "engine unavailable: YtDlp engine does not support active pause",
            )
        }
        val vm = DownloadsViewModel(repo)

        vm.pause("id1")

        assertEquals("Pause isn't supported for this download type.", vm.uiState.value.error)
    }

    // ── 5. resume calls repository ────────────────────────────────────────

    @Test
    fun resumeCallsRepository() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)
        vm.resume("id1")
        assertEquals("id1", repo.lastResumedId)
    }

    // ── 6. cancel calls repository ────────────────────────────────────────

    @Test
    fun cancelCallsRepository() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)
        vm.cancel("id1")
        assertEquals("id1", repo.lastCancelledId)
    }

    // ── 7. retry calls repository ──────────────────────────────────────────

    @Test
    fun retryCallsRepository() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)
        vm.retry("id1")
        assertEquals("id1", repo.lastRetriedId)
    }

    // ── 8. remove calls repository ────────────────────────────────────────

    @Test
    fun removeCallsRepository() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)
        vm.remove("id1")
        assertEquals("id1", repo.lastRemovedId)
    }

    @Test
    fun removeFailureIsShownInsteadOfCrashing() = runTest {
        val repo = FakeDownloadRepository().apply {
            actionFailure = IllegalStateException("remove failed")
        }
        val vm = DownloadsViewModel(repo)

        vm.remove("id1")

        assertEquals("Could not remove this download. Please try again.", vm.uiState.value.error)
    }

    // ── 9. setPriority calls repository ──────────────────────────────────

    @Test
    fun setPriorityCallsRepository() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)
        vm.setPriority("id1", DownloadPriority.HIGH)
        assertEquals("id1", repo.lastSetPriorityId)
    }

    // ── 10. Filtering active ────────────────────────────────────────────────

    @Test
    fun filteringActiveReturnsOnlyActive() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)

        val jobAll = backgroundScope.launch(testDispatcher) { vm.activeDownloads.collect { } }

        repo.emit(
            listOf(
                makeItem("a1", DownloadState.Active(512, 1024, 100, 5, 1)),
                makeItem("q1", DownloadState.Queued),
            )
        )

        val active = vm.activeDownloads.value
        assertEquals(1, active.size)
        assertTrue(active.first().state is DownloadState.Active)
        jobAll.cancel()
    }

    // ── 11. Filtering completed ────────────────────────────────────────────

    @Test
    fun filteringCompletedReturnsOnlyCompleted() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)

        val job = backgroundScope.launch(testDispatcher) { vm.completedDownloads.collect { } }

        repo.emit(
            listOf(
                makeItem("c1", DownloadState.Completed(1024, "/dl/file.zip")),
                makeItem("a1", DownloadState.Active(512, 1024, 100, 5, 1)),
            )
        )

        val completed = vm.completedDownloads.value
        assertEquals(1, completed.size)
        assertTrue(completed.first().state is DownloadState.Completed)
        job.cancel()
    }

    @Test
    fun activeDownloadsWaitForInitialSnapshotBeforeShowingEmptyState() = runTest {
        val repo = FakeDownloadRepository()
        val snapshotReady = CompletableDeferred<Unit>()
        repo.initialSnapshot = snapshotReady
        val vm = DownloadsViewModel(repo)
        val activeCollector = backgroundScope.launch(testDispatcher) {
            vm.activeDownloads.collect { }
        }

        assertEquals(DownloadSnapshotState.Loading, vm.downloadSnapshotState.value)
        assertTrue(vm.activeDownloads.value.isEmpty())

        val activeItem = makeItem("active-on-startup", DownloadState.Active(1, 2, 1, 1, 1))
        repo.emit(listOf(activeItem))
        snapshotReady.complete(Unit)

        assertEquals(DownloadSnapshotState.Ready, vm.downloadSnapshotState.value)
        assertEquals(listOf(activeItem), vm.activeDownloads.value)
        activeCollector.cancel()
    }

    // ── 12. Filtering cancelled ────────────────────────────────────────────

    @Test
    fun filteringCancelledReturnsOnlyCancelled() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)

        val job = backgroundScope.launch(testDispatcher) { vm.cancelledDownloads.collect { } }

        repo.emit(
            listOf(
                makeItem("x1", DownloadState.Cancelled),
                makeItem("a1", DownloadState.Active(512, 1024, 100, 5, 1)),
            )
        )

        val cancelled = vm.cancelledDownloads.value
        assertEquals(1, cancelled.size)
        assertTrue(cancelled.first().state is DownloadState.Cancelled)
        job.cancel()
    }

    // ── 13. Bridge unavailable sets bridgeAvailable = false ────────────────

    @Test
    fun bridgeUnavailableSetsBridgeAvailableFalse() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel.Factory(repo, bridgeAvailable = false)
            .create(DownloadsViewModel::class.java)
        assertFalse(vm.uiState.value.bridgeAvailable)
    }

    // ── 14. URL validation — error set on repository exception ────────────

    @Test
    fun urlValidationEmptyShowsError() = runTest {
        val repo = FakeDownloadRepository() // throws IllegalArgumentException on blank URL
        val vm = DownloadsViewModel(repo)
        vm.addDownload("", "file.zip", "/dl/file.zip", DownloadPriority.NORMAL)
        // UnconfinedTestDispatcher runs coroutines eagerly — error should be set already.
        assertNotNull(vm.uiState.value.error)
    }

    // ── clearError resets error ────────────────────────────────────────────

    @Test
    fun clearErrorResetsError() = runTest {
        val repo = FakeDownloadRepository()
        val vm = DownloadsViewModel(repo)
        vm.addDownload("", "file.zip", "/dl/file.zip", DownloadPriority.NORMAL)
        assertNotNull(vm.uiState.value.error)
        vm.clearError()
        assertNull(vm.uiState.value.error)
    }

    // ── 15. native_torrent telemetry is polled ─────────────────────────────

    @Test
    fun nativeTorrentTelemetryIsPolled() = runTest {
        val fakeTelemetry = TorrentTelemetry(
            downloadSpeedBytesPerSecond = 512_000L,
            uploadSpeedBytesPerSecond = 128_000L,
            downloadedBytes = 1_048_576L,
            uploadedBytes = 65_536L,
            peerCount = 8L,
            completedPeerCount = 3L,
            seedCount = 5L,
            shareRatio = 0.5,
            seeding = false,
            trackerUris = listOf("udp://tracker.example.com:6969"),
        )
        val repo = FakeTorrentDownloadRepository(telemetryByDownloadId = mapOf("nt-1" to fakeTelemetry))
        val vm = DownloadsViewModel(repo)

        // Subscribe to both downloads and torrentTelemetry so WhileSubscribed flows activate
        // and the init-block collectLatest can receive the emission.
        val job1 = backgroundScope.launch(testDispatcher) { vm.downloads.collect { } }
        val job2 = backgroundScope.launch(testDispatcher) { vm.torrentTelemetry.collect { } }

        val nativeTorrentItem = makeItem("nt-1", DownloadState.Active(512, 1024, 100, 5, 1))
            .copy(engine = "native_torrent", url = "magnet:?xt=urn:btih:abc123")
        repo.emit(listOf(nativeTorrentItem))

        // Allow the polling coroutine to run at least one iteration before the delay(2000).
        advanceTimeBy(100)

        val telemetryMap = vm.torrentTelemetry.value
        assertNotNull(telemetryMap["nt-1"], "Expected telemetry for native_torrent download 'nt-1'")
        assertEquals(512_000L, telemetryMap["nt-1"]!!.downloadSpeedBytesPerSecond)
        assertEquals(8L, telemetryMap["nt-1"]!!.peerCount)

        // Cancel viewModelScope to stop the infinite polling loop so runTest can drain cleanly.
        vm.viewModelScope.cancel()
        job1.cancel()
        job2.cancel()
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

private fun makeItem(id: String, state: DownloadState): DownloadItem = DownloadItem(
    id = id,
    fileName = "file-$id.zip",
    url = "https://example.com/$id",
    destinationPath = "/downloads/file-$id.zip",
    state = state,
    priority = DownloadPriority.NORMAL,
    engine = "direct",
    mimeType = null,
    totalBytes = null,
    createdAt = System.currentTimeMillis(),
)

// ── Fake repository ────────────────────────────────────────────────────────────

private class FakeDownloadRepository :
    DownloadRepository,
    DuplicateDownloadRepository,
    BatchDownloadRepository {

    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    private val _events = MutableSharedFlow<DownloadEvent>()

    var lastAddUrl: String? = null
    var lastPausedId: String? = null
    var lastResumedId: String? = null
    var lastCancelledId: String? = null
    var lastRetriedId: String? = null
    var lastRemovedId: String? = null
    var lastSetPriorityId: String? = null
    var actionFailure: Exception? = null
    var initialSnapshot: CompletableDeferred<Unit>? = null
    var duplicateFileName: String? = null
    var lastDuplicateLookupUrl: String? = null
    var duplicateLookupFailure: Exception? = null
    var batchOutcomes: List<DownloadBatchOutcome> = emptyList()
    var lastBatchItems: List<DownloadBatchItem>? = null
    var lastBatchPriority: DownloadPriority? = null
    var lastBatchChecksum: String? = null

    fun emit(items: List<DownloadItem>) {
        _downloads.value = items
    }

    override fun observeDownloads(): Flow<List<DownloadItem>> = _downloads.asStateFlow()

    override suspend fun awaitInitialSnapshot() {
        initialSnapshot?.await()
    }

    override fun observeEvents(): Flow<DownloadEvent> = _events.asSharedFlow()

    override suspend fun findDuplicateDownload(url: String): String? {
        duplicateLookupFailure?.let { throw it }
        lastDuplicateLookupUrl = url
        return duplicateFileName
    }

    override suspend fun addDownloadBatch(
        items: List<DownloadBatchItem>,
        priority: DownloadPriority,
        expectedSha256: String?,
    ): List<DownloadBatchOutcome> {
        lastBatchItems = items
        lastBatchPriority = priority
        lastBatchChecksum = expectedSha256
        return batchOutcomes
    }

    override suspend fun addDownload(
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority,
        mimeType: String?,
        totalBytes: Long?,
        referrer: String?,
    ): String {
        if (url.isBlank()) throw IllegalArgumentException("URL cannot be empty")
        lastAddUrl = url
        val item = makeItem(url.hashCode().toString(), DownloadState.Queued)
        _downloads.value = _downloads.value + item
        return item.id
    }

    override suspend fun pauseDownload(downloadId: String) {
        actionFailure?.let { throw it }
        lastPausedId = downloadId
    }

    override suspend fun resumeDownload(downloadId: String) {
        lastResumedId = downloadId
    }

    override suspend fun cancelDownload(downloadId: String, deleteFile: Boolean) {
        lastCancelledId = downloadId
    }

    override suspend fun removeDownload(downloadId: String, deleteFile: Boolean) {
        actionFailure?.let { throw it }
        lastRemovedId = downloadId
    }

    override suspend fun retryDownload(downloadId: String): String {
        lastRetriedId = downloadId
        return downloadId
    }

    override suspend fun setPriority(downloadId: String, priority: DownloadPriority) {
        lastSetPriorityId = downloadId
    }

    override suspend fun moveUpInQueue(downloadId: String) = Unit
    override suspend fun moveDownInQueue(downloadId: String) = Unit

    override suspend fun scheduleDownload(downloadId: String, startAtMillis: Long) = Unit

    override suspend fun getDownload(downloadId: String): DownloadItem? = null

}

// ── Fake TorrentDownloadRepository ────────────────────────────────────────────

private class FakeTorrentDownloadRepository(
    private val telemetryByDownloadId: Map<String, TorrentTelemetry> = emptyMap(),
) : TorrentDownloadRepository {

    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    private val _events = MutableSharedFlow<DownloadEvent>()

    fun emit(items: List<DownloadItem>) {
        _downloads.value = items
    }

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
    ): String = url.hashCode().toString()

    override suspend fun pauseDownload(downloadId: String) = Unit
    override suspend fun resumeDownload(downloadId: String) = Unit
    override suspend fun cancelDownload(downloadId: String, deleteFile: Boolean) = Unit
    override suspend fun removeDownload(downloadId: String, deleteFile: Boolean) = Unit
    override suspend fun retryDownload(downloadId: String): String = downloadId
    override suspend fun setPriority(downloadId: String, priority: DownloadPriority) = Unit
    override suspend fun moveUpInQueue(downloadId: String) = Unit
    override suspend fun moveDownInQueue(downloadId: String) = Unit
    override suspend fun scheduleDownload(downloadId: String, startAtMillis: Long) = Unit
    override suspend fun getDownload(downloadId: String): DownloadItem? = null

    override suspend fun addMagnet(magnetUri: String, priority: DownloadPriority): String =
        magnetUri.hashCode().toString()

    override suspend fun inspectTorrentFile(torrentPath: String): TorrentMetadata =
        TorrentMetadata("fake", "abc123", 0L, emptyList())

    override suspend fun addTorrentFile(
        torrentPath: String,
        selectedFileIndexes: List<Int>,
        destinationDirectory: String,
        priority: DownloadPriority,
    ): String = torrentPath.hashCode().toString()

    override suspend fun torrentTelemetry(downloadId: String): TorrentTelemetry? =
        telemetryByDownloadId[downloadId]

    override suspend fun torrentDownloadMetadata(downloadId: String): TorrentDownloadMetadata? = null

    override suspend fun selectTorrentFiles(downloadId: String, fileIndexes: List<Int>) = Unit
    override suspend fun setTorrentFileSelected(downloadId: String, fileIndex: Int, selected: Boolean) = Unit
    override suspend fun setTorrentSelectionAll(downloadId: String, selected: Boolean) = Unit
    override suspend fun setTorrentDirectorySelected(downloadId: String, directory: String, selected: Boolean) = Unit
    override suspend fun stopTorrentSeeding(downloadId: String) = Unit
}
