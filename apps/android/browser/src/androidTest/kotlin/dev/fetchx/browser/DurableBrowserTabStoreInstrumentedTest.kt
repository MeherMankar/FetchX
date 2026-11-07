package dev.fetchx.browser

import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.util.AtomicFile
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.fetchx.browser.DurableBrowserTabStore
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class DurableBrowserTabStoreInstrumentedTest {
    @Test
    fun backgroundLoadWaitsForWriterLockWithoutBlockingMainThread() {
        val slots = ContendedSlots()
        val writerFinished = CountDownLatch(1)
        val writerExecutor = Executor { command ->
            Thread(command, "snapshot-contention-writer").start()
        }
        val store = DurableBrowserTabStore(
            slots = slots,
            executor = writerExecutor,
            onReadError = { error -> throw AssertionError("Snapshot read failed", error) },
            onWriteResult = { writerFinished.countDown() },
        )

        slots.blockNextWrite()
        store.submit(listOf(TabState("contended")), "contended")
        assertTrue("writer did not reach the storage barrier", slots.writeStarted.await(10, TimeUnit.SECONDS))

        val loadStarted = CountDownLatch(1)
        val loadFinished = CountDownLatch(1)
        val loadError = AtomicReference<Throwable?>()
        val loadDurationNanos = AtomicLong()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
        scope.launch {
            loadStarted.countDown()
            val started = SystemClock.elapsedRealtimeNanos()
            try {
                val restored = withContext(Dispatchers.IO) {
                    assertTrue(Looper.myLooper() != Looper.getMainLooper())
                    store.load()
                }
                assertEquals(1L, restored?.generation)
            } catch (error: Throwable) {
                loadError.set(error)
            } finally {
                loadDurationNanos.set(SystemClock.elapsedRealtimeNanos() - started)
                loadFinished.countDown()
            }
        }

        try {
            assertTrue("background load did not start", loadStarted.await(10, TimeUnit.SECONDS))
            assertTrue("load did not wait for the storage lock", awaitSnapshotLockWaiter())
            val loadBlockedAt = SystemClock.elapsedRealtimeNanos()

            val mainResponsive = CountDownLatch(1)
            Handler(Looper.getMainLooper()).post { mainResponsive.countDown() }
            assertTrue(
                "main thread did not process work while snapshot load waited",
                mainResponsive.await(2, TimeUnit.SECONDS),
            )

            val lockWaitBeforeReleaseMs =
                (SystemClock.elapsedRealtimeNanos() - loadBlockedAt) / 1_000_000.0
            slots.releaseWrite()
            assertTrue("writer did not finish", writerFinished.await(10, TimeUnit.SECONDS))
            assertTrue("background load did not finish", loadFinished.await(10, TimeUnit.SECONDS))
            loadError.get()?.let { throw AssertionError("Background snapshot load failed", it) }

            val backgroundLoadMs = loadDurationNanos.get() / 1_000_000.0
            Log.i(
                "TabSnapshotLockPerf",
                "mainThreadResponsive=true loadThread=Dispatchers.IO " +
                    "lockWaitBeforeReleaseMs=$lockWaitBeforeReleaseMs backgroundLoadMs=$backgroundLoadMs",
            )
        } finally {
            slots.releaseWrite()
            scope.cancel()
        }
    }

    @Test
    fun cancelledRestoreDoesNotDeliverAResultAfterLockRelease() {
        val slots = ContendedSlots()
        val writerFinished = CountDownLatch(1)
        val store = DurableBrowserTabStore(
            slots = slots,
            executor = Executor { command -> Thread(command, "cancelled-restore-writer").start() },
            onReadError = { error -> throw AssertionError("Snapshot read failed", error) },
            onWriteResult = { writerFinished.countDown() },
        )
        slots.blockNextWrite()
        store.submit(listOf(TabState("cancelled")), "cancelled")
        assertTrue("writer did not reach the storage barrier", slots.writeStarted.await(10, TimeUnit.SECONDS))

        val loadStarted = CountDownLatch(1)
        val loadFinished = CountDownLatch(1)
        val result = AtomicReference<BrowserTabSnapshot?>()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
        val job: Job = scope.launch {
            loadStarted.countDown()
            try {
                result.set(withContext(Dispatchers.IO) { store.load() })
            } finally {
                loadFinished.countDown()
            }
        }

        try {
            assertTrue("restore load did not start", loadStarted.await(10, TimeUnit.SECONDS))
            assertTrue("restore load did not wait for the storage lock", awaitSnapshotLockWaiter())
            job.cancel()
            slots.releaseWrite()
            assertTrue("writer did not finish", writerFinished.await(10, TimeUnit.SECONDS))
            assertTrue("cancelled restore coroutine did not finish", loadFinished.await(10, TimeUnit.SECONDS))
            runBlocking { job.join() }
            assertTrue("restore coroutine was not cancelled", job.isCancelled)
            assertNull(result.get())
        } finally {
            slots.releaseWrite()
            scope.cancel()
        }
    }

    @Test
    fun productionStoresWithEquivalentDirectoryPathsShareDefaultWriterState() {
        val context = InstrumentationRegistry.getInstrumentation().context
        val directory = File(context.filesDir, "normalized-sharing-${UUID.randomUUID()}")
        val firstStorage = AtomicFileSnapshotSlotStorage(directory)
        val secondStorage = AtomicFileSnapshotSlotStorage(File(directory, "."))
        assertEquals(firstStorage.identity, secondStorage.identity)

        val completed = CountDownLatch(2)
        val results = java.util.Collections.synchronizedList(
            mutableListOf<DurableBrowserTabStore.WriteResult>(),
        )
        val firstStore = DurableBrowserTabStore(
            slots = firstStorage,
            onReadError = { error -> throw AssertionError("Snapshot read failed", error) },
            onWriteResult = { results.add(it); completed.countDown() },
        )
        val secondStore = DurableBrowserTabStore(
            slots = secondStorage,
            onReadError = { error -> throw AssertionError("Snapshot read failed", error) },
            onWriteResult = { results.add(it); completed.countDown() },
        )

        firstStore.submit(listOf(TabState(id = "first", url = "https://first.example")), "first")
        secondStore.submit(listOf(TabState(id = "second", url = "https://second.example")), "second")

        assertTrue("default writer did not finish both snapshots", completed.await(10, TimeUnit.SECONDS))
        assertEquals(
            listOf(
                DurableBrowserTabStore.WriteResult.Persisted(1),
                DurableBrowserTabStore.WriteResult.Persisted(2),
            ),
            results.sortedBy { result ->
                when (result) {
                    is DurableBrowserTabStore.WriteResult.Persisted -> result.generation
                    is DurableBrowserTabStore.WriteResult.Superseded -> result.generation
                    is DurableBrowserTabStore.WriteResult.Failed -> result.generation
                }
            },
        )
        val latest = DurableBrowserTabStore(
            slots = AtomicFileSnapshotSlotStorage(directory),
            onReadError = { error -> throw AssertionError("Snapshot read failed", error) },
            onWriteResult = {},
        ).load()
        assertEquals(2L, latest?.generation)
        assertEquals(listOf("second"), latest?.tabs?.map { it.id })
    }

    @Test
    fun atomicFileReadDuringWriteCanPreventReplacementCommit() {
        val context = InstrumentationRegistry.getInstrumentation().context
        val directory = File(context.filesDir, "atomic-read-write-${UUID.randomUUID()}")
        val storage = AtomicFileSnapshotSlotStorage(directory)
        val previous = snapshotBytes(1, "previous")
        val replacement = snapshotBytes(2, "replacement")
        storage.writeAtomically(0, previous)
        val writeStarted = CountDownLatch(1)
        val finishWrite = CountDownLatch(1)
        val writeFailure = AtomicReference<Throwable?>()
        val writer = Thread {
            val atomicFile = AtomicFile(File(directory, "snapshot-0"))
            var stream: java.io.FileOutputStream? = null
            try {
                stream = atomicFile.startWrite()
                stream.write(replacement, 0, replacement.size / 2)
                writeStarted.countDown()
                assertTrue(finishWrite.await(10, TimeUnit.SECONDS))
                stream.write(replacement, replacement.size / 2, replacement.size - replacement.size / 2)
                atomicFile.finishWrite(stream)
                stream = null
            } catch (error: Throwable) {
                stream?.let(atomicFile::failWrite)
                writeFailure.set(error)
            }
        }
        writer.start()

        assertTrue("writer did not reach its partial-write barrier", writeStarted.await(10, TimeUnit.SECONDS))
        assertArrayEquals(previous, storage.read(0))
        finishWrite.countDown()
        writer.join(TimeUnit.SECONDS.toMillis(10))

        assertTrue("AtomicFile writer did not finish", !writer.isAlive)
        assertNull(writeFailure.get())
        assertArrayEquals(previous, storage.read(0))
    }

    @Test
    fun atomicFileFailureRestoresPreviousSnapshotAfterPartialWrite() {
        val context = InstrumentationRegistry.getInstrumentation().context
        val directory = File(context.filesDir, "atomic-failed-write-${UUID.randomUUID()}")
        val storage = AtomicFileSnapshotSlotStorage(directory)
        val previous = snapshotBytes(1, "previous")
        val replacement = snapshotBytes(2, "replacement")
        storage.writeAtomically(0, previous)
        val writeStarted = CountDownLatch(1)
        val failWrite = CountDownLatch(1)
        val writeFailure = AtomicReference<Throwable?>()
        val writer = Thread {
            val atomicFile = AtomicFile(File(directory, "snapshot-0"))
            var stream: java.io.FileOutputStream? = null
            try {
                stream = atomicFile.startWrite()
                stream.write(replacement, 0, replacement.size / 2)
                writeStarted.countDown()
                assertTrue(failWrite.await(10, TimeUnit.SECONDS))
                atomicFile.failWrite(stream)
                stream = null
            } catch (error: Throwable) {
                stream?.let(atomicFile::failWrite)
                writeFailure.set(error)
            }
        }
        writer.start()

        assertTrue("writer did not reach its partial-write barrier", writeStarted.await(10, TimeUnit.SECONDS))
        assertArrayEquals(previous, storage.read(0))
        failWrite.countDown()
        writer.join(TimeUnit.SECONDS.toMillis(10))

        assertTrue("AtomicFile writer did not finish", !writer.isAlive)
        assertNull(writeFailure.get())
        assertArrayEquals(previous, storage.read(0))
    }

    @Test
    fun productionAtomicFileStoreRestoresSnapshotsAndMeasuresSubmitCost() {
        val context = InstrumentationRegistry.getInstrumentation().context
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val runtime = Runtime.getRuntime()

        for (tabCount in listOf(1, 10, 50, 100)) {
            val directory = File(context.filesDir, "durability-${UUID.randomUUID()}")
            val storage = AtomicFileSnapshotSlotStorage(directory)
            val tabs = (1..tabCount).map { index ->
                TabState(
                    id = "tab-$index",
                    url = "https://example.test/$index",
                    title = "Tab $index",
                    spaceId = if (index % 2 == 0) "work" else "default",
                    desktopSiteEnabled = index % 3 == 0,
                )
            }
            val latch = CountDownLatch(1)
            val result = AtomicReference<DurableBrowserTabStore.WriteResult>()
            val store = DurableBrowserTabStore(
                slots = storage,
                onReadError = { error -> throw AssertionError("AtomicFile read failed", error) },
                onWriteResult = {
                    result.set(it)
                    latch.countDown()
                },
            )
            var submitNanos = 0L
            instrumentation.runOnMainSync {
                val started = SystemClock.elapsedRealtimeNanos()
                store.submit(tabs, tabs.last().id)
                submitNanos = SystemClock.elapsedRealtimeNanos() - started
            }

            assertTrue("writer did not finish for $tabCount tabs", latch.await(10, TimeUnit.SECONDS))
            assertTrue("write failed for $tabCount tabs: ${result.get()}", result.get() is DurableBrowserTabStore.WriteResult.Persisted)
            val encodedBytes = File(directory, "snapshot-1").length()
            val heapBeforeRestore = runtime.totalMemory() - runtime.freeMemory()
            val restoreStarted = SystemClock.elapsedRealtimeNanos()
            val restored = DurableBrowserTabStore(
                slots = AtomicFileSnapshotSlotStorage(directory),
                onReadError = { error -> throw AssertionError("Snapshot read failed", error) },
                onWriteResult = {},
            ).load()
            val restoreNanos = SystemClock.elapsedRealtimeNanos() - restoreStarted
            val heapAfterRestore = runtime.totalMemory() - runtime.freeMemory()

            assertTrue("snapshot missing for $tabCount tabs", restored != null)
            assertEquals(tabCount, restored?.tabs?.size)
            assertEquals(tabs.last().id, restored?.activeTabId)
            Log.i(
                "TabSnapshotPerf",
                "tabs=$tabCount encodedBytes=$encodedBytes mainSubmitMs=${submitNanos / 1_000_000.0} " +
                    "restoreMs=${restoreNanos / 1_000_000.0} heapDeltaBytes=${heapAfterRestore - heapBeforeRestore}",
            )
        }
    }

    private fun snapshotBytes(generation: Long, id: String): ByteArray =
        BrowserTabSnapshotCodec.encode(
            BrowserTabSnapshot(generation, listOf(TabState(id = id, url = "https://$id.example")), id),
        )

    private fun awaitSnapshotLockWaiter(): Boolean {
        val deadline = SystemClock.elapsedRealtime() + 10_000
        while (SystemClock.elapsedRealtime() < deadline) {
            val waiterFound = Thread.getAllStackTraces().any { (thread, trace) ->
                thread.state == Thread.State.BLOCKED && trace.any { frame ->
                    frame.className == DurableBrowserTabStore::class.java.name &&
                        frame.methodName == "load"
                }
            }
            if (waiterFound) return true
            Thread.sleep(5)
        }
        return false
    }

    private class ContendedSlots : SnapshotSlotStorage {
        override val identity: String = "contention-${UUID.randomUUID()}"
        private val values = arrayOfNulls<ByteArray>(2)
        private var shouldBlockNextWrite = false
        private val finishWrite = CountDownLatch(1)
        val writeStarted = CountDownLatch(1)

        override fun read(slot: Int): ByteArray? = values[slot]?.copyOf()

        override fun writeAtomically(slot: Int, bytes: ByteArray) {
            if (shouldBlockNextWrite) {
                shouldBlockNextWrite = false
                writeStarted.countDown()
                if (!finishWrite.await(10, TimeUnit.SECONDS)) {
                    throw IOException("Test write was not released")
                }
            }
            values[slot] = bytes.copyOf()
        }

        fun blockNextWrite() {
            shouldBlockNextWrite = true
        }

        fun releaseWrite() {
            finishWrite.countDown()
        }
    }
}
