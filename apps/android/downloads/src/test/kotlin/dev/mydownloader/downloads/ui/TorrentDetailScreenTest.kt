package dev.fetchx.downloads.ui

import androidx.lifecycle.viewModelScope
import dev.fetchx.core.download.DownloadEvent
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.DownloadRepository
import dev.fetchx.core.download.DownloadState
import dev.fetchx.core.download.TorrentDownloadMetadata
import dev.fetchx.core.download.TorrentDownloadRepository
import dev.fetchx.core.download.TorrentMetadata
import dev.fetchx.core.download.TorrentTelemetry
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

@OptIn(ExperimentalCoroutinesApi::class)
class TorrentDetailScreenTest {

    private val testDispatcher = UnconfinedTestDispatcher()

    @Before
    fun setUp() {
        Dispatchers.setMain(testDispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    // ── 1. getDownloadItem returns the item when present ───────────────────

    @Test
    fun getDownloadItemReturnsItem() = runTest {
        val item = makeTorrentItem("t-1", DownloadState.MetadataAcquiring(0, null))
        val repo = FakeTorrentDetailRepo()
        val vm = DownloadsViewModel(repo)

        val job = backgroundScope.launch(testDispatcher) { vm.downloads.collect { } }
        repo.emit(listOf(item))

        val found = vm.getDownloadItem("t-1")
        assertNotNull(found)
        assertEquals("t-1", found.id)

        // Cancel viewModelScope to stop the init-block polling loop so runTest drains.
        vm.viewModelScope.cancel()
        job.cancel()
    }

    // ── 2. getDownloadItem returns null for unknown id ─────────────────────

    @Test
    fun getDownloadItemReturnsNullForUnknownId() = runTest {
        val repo = FakeTorrentDetailRepo()
        val vm = DownloadsViewModel(repo)

        val job = backgroundScope.launch(testDispatcher) { vm.downloads.collect { } }
        repo.emit(emptyList())

        assertNull(vm.getDownloadItem("does-not-exist"))

        vm.viewModelScope.cancel()
        job.cancel()
    }

    // ── 3. setTorrentFileSelected calls repository ─────────────────────────

    @Test
    fun setTorrentFileSelectedCallsRepository() = runTest {
        val repo = FakeTorrentDetailRepo()
        val vm = DownloadsViewModel(repo)

        vm.setTorrentFileSelected("t-1", 2, true)

        assertEquals("t-1", repo.lastSetFileSelectedId)
        assertEquals(2, repo.lastSetFileSelectedIndex)
        assertEquals(true, repo.lastSetFileSelectedValue)

        vm.viewModelScope.cancel()
    }

    // ── 4. setTorrentSelectionAll calls repository ─────────────────────────

    @Test
    fun setTorrentSelectionAllCallsRepository() = runTest {
        val repo = FakeTorrentDetailRepo()
        val vm = DownloadsViewModel(repo)

        vm.setTorrentSelectionAll("t-1", false)

        assertEquals("t-1", repo.lastSelectionAllId)
        assertEquals(false, repo.lastSelectionAllValue)

        vm.viewModelScope.cancel()
    }

    // ── 5. setTorrentDirectorySelected calls repository ───────────────────

    @Test
    fun setTorrentDirectorySelectedCallsRepository() = runTest {
        val repo = FakeTorrentDetailRepo()
        val vm = DownloadsViewModel(repo)

        vm.setTorrentDirectorySelected("t-1", "subdir/images", true)

        assertEquals("t-1", repo.lastDirSelectedId)
        assertEquals("subdir/images", repo.lastDirSelectedPath)
        assertEquals(true, repo.lastDirSelectedValue)

        vm.viewModelScope.cancel()
    }

    // ── 6. selectTorrentFiles calls repository ─────────────────────────────

    @Test
    fun selectTorrentFilesCallsRepository() = runTest {
        val repo = FakeTorrentDetailRepo()
        val vm = DownloadsViewModel(repo)

        vm.selectTorrentFiles("t-1", listOf(0, 1, 3))

        assertEquals("t-1", repo.lastSelectFilesId)
        assertEquals(listOf(0, 1, 3), repo.lastSelectFilesIndexes)

        vm.viewModelScope.cancel()
    }

    // ── 7. stopTorrentSeeding calls repository ─────────────────────────────

    @Test
    fun stopTorrentSeedingCallsRepository() = runTest {
        val repo = FakeTorrentDetailRepo()
        val vm = DownloadsViewModel(repo)

        vm.stopTorrentSeeding("t-1")

        assertEquals("t-1", repo.lastStopSeedingId)

        vm.viewModelScope.cancel()
    }

    // ── 8. getDownloadItem reflects updated downloads list ─────────────────

    @Test
    fun getDownloadItemReflectsUpdatedList() = runTest {
        val item = makeTorrentItem("t-1", DownloadState.MetadataAcquiring(0, null))
        val repo = FakeTorrentDetailRepo()
        val vm = DownloadsViewModel(repo)

        val job = backgroundScope.launch(testDispatcher) { vm.downloads.collect { } }
        repo.emit(listOf(item))

        assertNotNull(vm.getDownloadItem("t-1"))
        assertNull(vm.getDownloadItem("no-such-id"))

        // After removal, item should no longer be found.
        repo.emit(emptyList())
        assertNull(vm.getDownloadItem("t-1"))

        vm.viewModelScope.cancel()
        job.cancel()
    }

    // ── 9. selectedDownloadId can be set and cleared ───────────────────────

    @Test
    fun selectedDownloadIdCanBeSetAndCleared() = runTest {
        val repo = FakeTorrentDetailRepo()
        val vm = DownloadsViewModel(repo)

        assertNull(vm.selectedDownloadId.value)

        vm.setSelectedDownloadId("t-42")
        assertEquals("t-42", vm.selectedDownloadId.value)

        vm.setSelectedDownloadId(null)
        assertNull(vm.selectedDownloadId.value)

        vm.viewModelScope.cancel()
    }

    // ── 10. setTorrentFileSelected no-ops for plain DownloadRepository ─────

    @Test
    fun setTorrentFileSelectedNoOpsForPlainRepository() = runTest {
        // FakePlainDetailRepo does NOT implement TorrentDownloadRepository,
        // so the ViewModel init block skips the polling loop entirely.
        val plainRepo = FakePlainDetailRepo()
        val vm = DownloadsViewModel(plainRepo)

        // Should not throw — just silently ignore.
        vm.setTorrentFileSelected("x", 0, true)
        // Absence of exception is the pass condition.
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

private fun makeTorrentItem(id: String, state: DownloadState): DownloadItem = DownloadItem(
    id = id,
    fileName = "torrent-$id",
    url = "magnet:?xt=urn:btih:$id",
    destinationPath = "/downloads/$id",
    state = state,
    priority = DownloadPriority.NORMAL,
    engine = "native_torrent",
    mimeType = null,
    totalBytes = null,
    createdAt = 0L,
)

// ── Fake TorrentDownloadRepository (for torrent-specific tests) ────────────────

private class FakeTorrentDetailRepo : TorrentDownloadRepository {

    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    private val _events = MutableSharedFlow<DownloadEvent>()

    var lastSetFileSelectedId: String? = null
    var lastSetFileSelectedIndex: Int? = null
    var lastSetFileSelectedValue: Boolean? = null
    var lastSelectionAllId: String? = null
    var lastSelectionAllValue: Boolean? = null
    var lastDirSelectedId: String? = null
    var lastDirSelectedPath: String? = null
    var lastDirSelectedValue: Boolean? = null
    var lastSelectFilesId: String? = null
    var lastSelectFilesIndexes: List<Int>? = null
    var lastStopSeedingId: String? = null

    fun emit(items: List<DownloadItem>) { _downloads.value = items }

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
        TorrentMetadata("fake", "abc123", 1024L, emptyList())

    override suspend fun addTorrentFile(
        torrentPath: String,
        selectedFileIndexes: List<Int>,
        destinationDirectory: String,
        priority: DownloadPriority,
    ): String = torrentPath.hashCode().toString()

    override suspend fun torrentTelemetry(downloadId: String): TorrentTelemetry? = null

    override suspend fun torrentDownloadMetadata(downloadId: String): TorrentDownloadMetadata? = null

    override suspend fun selectTorrentFiles(downloadId: String, fileIndexes: List<Int>) {
        lastSelectFilesId = downloadId
        lastSelectFilesIndexes = fileIndexes
    }

    override suspend fun setTorrentFileSelected(downloadId: String, fileIndex: Int, selected: Boolean) {
        lastSetFileSelectedId = downloadId
        lastSetFileSelectedIndex = fileIndex
        lastSetFileSelectedValue = selected
    }

    override suspend fun setTorrentSelectionAll(downloadId: String, selected: Boolean) {
        lastSelectionAllId = downloadId
        lastSelectionAllValue = selected
    }

    override suspend fun setTorrentDirectorySelected(downloadId: String, directory: String, selected: Boolean) {
        lastDirSelectedId = downloadId
        lastDirSelectedPath = directory
        lastDirSelectedValue = selected
    }

    override suspend fun stopTorrentSeeding(downloadId: String) {
        lastStopSeedingId = downloadId
    }
}

// ── Fake plain DownloadRepository (no torrent features, no polling loop) ──────

private class FakePlainDetailRepo : DownloadRepository {

    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    private val _events = MutableSharedFlow<DownloadEvent>()

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
}
