package dev.fetchx.app.navigation

import dev.fetchx.core.download.DownloadEvent
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.DownloadRepository
import dev.fetchx.core.download.DownloadState
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

@OptIn(ExperimentalCoroutinesApi::class)
class RepositoryReadinessTest {

    // ── 1. Immediately available ──────────────────────────────────────────

    @Test
    fun `returnsRepositoryWhenImmediatelyAvailable`() = runTest {
        var calls = 0
        val repo = FakeRepository()
        val provider: () -> DownloadRepository? = {
            calls++
            repo
        }

        val result = awaitRepositoryReady(provider)

        assertEquals(repo, result)
        // The provider returned on the first call — no polling delay occurred.
        assertEquals(1, calls)
    }

    // ── 2. Waits until the repository becomes available ───────────────────

    @Test
    fun `waitsUntilRepositoryBecomesAvailable`() = runTest {
        val repo = FakeRepository()
        var polls = 0
        val provider: () -> DownloadRepository? = {
            polls++
            if (polls < 3) null else repo
        }

        val result = awaitRepositoryReady(provider, pollIntervalMs = 100L, timeoutMs = 10_000L)

        assertEquals(repo, result)
        // Two null polls (with their cancellable delays) before the repo was returned.
        assertEquals(3, polls)
    }

    // ── 3. Returns null on timeout (no infinite loop) ─────────────────────

    @Test
    fun `returnsNullOnTimeout`() = runTest {
        var calls = 0
        val provider: () -> DownloadRepository? = {
            calls++
            null
        }

        val result = awaitRepositoryReady(provider, pollIntervalMs = 100L, timeoutMs = 1_000L)

        assertNull(result)
        // Polled every 100ms until the 1000ms budget was exhausted: exactly 10 polls,
        // proving the wait is bounded rather than an infinite loop.
        assertEquals(10, calls)
    }

    // ── 4. Cancellable (no coroutine leak on Activity destruction) ───────

    @Test
    fun `isCancellable`() = runTest {
        val provider: () -> DownloadRepository? = { null }
        val job = launch {
            awaitRepositoryReady(provider, pollIntervalMs = 100L, timeoutMs = 60_000L)
        }
        // Let the helper enter its wait loop.
        advanceTimeBy(250L)
        job.cancelAndJoin()
        assertTrue(job.isCancelled)
    }

    // ── 5. Headline cold-start regression ─────────────────────────────────
    //
    // On a cold start the bridge is null at t=0 (no NoOp fallback is used for
    // resolution); a real repository containing a native_torrent download
    // becomes available after the wait begins. The helper must wait and then
    // resolve the torrent item, rather than resolving an empty snapshot at t=0.

    @Test
    fun `doesNotResolveFromEmptyWhenBridgeNotReady`() = runTest {
        val torrentItem = makeTorrentItem("dl-1", engine = "native_torrent")
        val readyRepo = FakeRepository(itemById = mapOf("dl-1" to torrentItem))
        var polls = 0
        val provider: () -> DownloadRepository? = {
            // Bridge is null on the first polls (cold start), then becomes ready.
            polls++
            if (polls < 3) null else readyRepo
        }

        val resolvedRepo = awaitRepositoryReady(provider, pollIntervalMs = 100L, timeoutMs = 10_000L)

        assertNotNull(resolvedRepo, "repository should become ready rather than resolving empty at t=0")
        val item = resolvedRepo.getDownload("dl-1")
        assertNotNull(item)
        assertEquals("native_torrent", item.engine)
    }

    // ── 6. Stale/removed download resolves to null (graceful fallback) ────

    @Test
    fun `missingDownloadResolvesToNullAfterReady`() = runTest {
        val readyRepo = FakeRepository(itemById = emptyMap())

        val resolvedRepo = awaitRepositoryReady({ readyRepo })

        assertNotNull(resolvedRepo)
        // A download id that no longer exists resolves to null, so the deep-link
        // stays on Downloads — the same graceful fallback as the old behaviour.
        assertNull(resolvedRepo.getDownload("gone-id"))
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

private fun makeTorrentItem(id: String, engine: String): DownloadItem = DownloadItem(
    id = id,
    fileName = "torrent-$id",
    url = "magnet:?xt=urn:btih:$id",
    destinationPath = "/downloads/$id",
    state = DownloadState.Queued,
    priority = DownloadPriority.NORMAL,
    engine = engine,
    mimeType = null,
    totalBytes = null,
    createdAt = 0L,
)

// ── Minimal fake repository ──────────────────────────────────────────────────
//
// Only getDownload() is exercised by these tests; the rest are no-ops so the
// fake satisfies the full DownloadRepository contract.

private class FakeRepository(
    private val itemById: Map<String, DownloadItem> = emptyMap(),
) : DownloadRepository {

    override fun observeDownloads(): Flow<List<DownloadItem>> = flowOf(itemById.values.toList())

    override fun observeEvents(): Flow<DownloadEvent> =
        MutableSharedFlow<DownloadEvent>().asSharedFlow()

    override suspend fun addDownload(
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority,
        mimeType: String?,
        totalBytes: Long?,
        referrer: String?,
    ): String = ""

    override suspend fun pauseDownload(downloadId: String) = Unit
    override suspend fun resumeDownload(downloadId: String) = Unit
    override suspend fun cancelDownload(downloadId: String, deleteFile: Boolean) = Unit
    override suspend fun removeDownload(downloadId: String, deleteFile: Boolean) = Unit
    override suspend fun retryDownload(downloadId: String): String = downloadId
    override suspend fun setPriority(downloadId: String, priority: DownloadPriority) = Unit
    override suspend fun scheduleDownload(downloadId: String, startAtMillis: Long) = Unit
    override suspend fun getDownload(downloadId: String): DownloadItem? = itemById[downloadId]
}
