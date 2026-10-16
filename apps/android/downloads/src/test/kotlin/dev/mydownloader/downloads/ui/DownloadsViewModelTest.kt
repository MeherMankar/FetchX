package dev.fetchx.downloads.ui

import dev.fetchx.core.download.DownloadEvent
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.DownloadRepository
import dev.fetchx.core.download.DownloadState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
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

private class FakeDownloadRepository : DownloadRepository {

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

    override suspend fun scheduleDownload(downloadId: String, startAtMillis: Long) = Unit

    override suspend fun getDownload(downloadId: String): DownloadItem? = null
}
