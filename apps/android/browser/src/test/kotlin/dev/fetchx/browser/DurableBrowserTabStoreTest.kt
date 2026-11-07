package dev.fetchx.browser

import dev.fetchx.browser.DurableBrowserTabStore
import java.io.IOException
import java.util.Collections
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executor
import java.util.concurrent.TimeUnit
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class DurableBrowserTabStoreTest {
    @Test
    fun `empty snapshot slots produce no restored snapshot`() {
        assertNull(store(MemorySlots()).load())
    }

    @Test
    fun `corrupt snapshot slots produce no restored snapshot`() {
        val slots = MemorySlots().apply {
            seed(0, byteArrayOf(1, 2, 3))
            seed(1, byteArrayOf(4, 5, 6))
        }

        assertNull(store(slots).load())
    }

    @Test
    fun `serialized generations keep the latest complete snapshot`() {
        val slots = MemorySlots()
        val executor = QueuedExecutor()
        val results = mutableListOf<DurableBrowserTabStore.WriteResult>()
        val store = store(slots, executor, results)
        val fourTabs = (1..4).map { tab("t$it") }

        store.submit(fourTabs, "t1")
        store.submit(fourTabs, "t2")
        store.submit(fourTabs.dropLast(1), "t2")
        store.submit(fourTabs.take(1), "t1")
        executor.runReverse()

        val restored = store(slots).load()
        assertNotNull(restored)
        assertEquals(4, restored.generation)
        assertEquals(listOf("t1"), restored.tabs.map { it.id })
        assertEquals("t1", restored.activeTabId)
        assertEquals(1, results.filterIsInstance<DurableBrowserTabStore.WriteResult.Persisted>().size)
        assertEquals(3, results.filterIsInstance<DurableBrowserTabStore.WriteResult.Superseded>().size)
    }

    @Test
    fun `snapshot restore retains tab spaces without changing existing assignments`() {
        val slots = MemorySlots()
        val executor = QueuedExecutor()
        val store = store(slots, executor)
        val tabs = listOf(
            tab("default").copy(spaceId = "default"),
            tab("work-existing").copy(spaceId = "work"),
            tab("work-new").copy(spaceId = "work"),
        )

        store.submit(tabs, "work-new")
        executor.runNext()

        val restored = store(slots).load()
        assertNotNull(restored)
        assertEquals(1L, restored.generation)
        assertEquals(tabs, restored.tabs)
        assertEquals("work-new", restored.activeTabId)
        assertEquals(
            listOf("work-existing", "work-new"),
            restored.tabs.filter { it.spaceId == "work" }.map { it.id },
        )
    }

    @Test
    fun `corrupt newest slot falls back to the previous valid generation`() {
        val slots = MemorySlots().apply {
            seed(1, BrowserTabSnapshotCodec.encode(BrowserTabSnapshot(1, listOf(tab("one")), "one")))
            seed(0, BrowserTabSnapshotCodec.encode(BrowserTabSnapshot(2, listOf(tab("two")), "two")))
            corrupt(0)
        }

        val restored = store(slots).load()

        assertNotNull(restored)
        assertEquals(1, restored.generation)
        assertEquals(listOf("one"), restored.tabs.map { it.id })
    }

    @Test
    fun `separate store instances share generation ordering`() {
        val slots = MemorySlots()
        val executor = QueuedExecutor()
        val firstResults = mutableListOf<DurableBrowserTabStore.WriteResult>()
        val secondResults = mutableListOf<DurableBrowserTabStore.WriteResult>()
        val firstStore = store(slots, executor, firstResults)
        val secondStore = store(slots, executor, secondResults)

        firstStore.submit(listOf(tab("first")), "first")
        secondStore.submit(listOf(tab("second")), "second")
        executor.runReverse()

        val restored = store(slots).load()
        assertNotNull(restored)
        assertEquals(2, restored.generation)
        assertEquals(listOf("second"), restored.tabs.map { it.id })
        assertEquals(DurableBrowserTabStore.WriteResult.Persisted(2), secondResults.single())
        assertIs<DurableBrowserTabStore.WriteResult.Superseded>(firstResults.single())
        assertEquals(listOf(2L, null), slots.generationsBySlot())
    }

    @Test
    fun `separate executors serialize shared-storage writes`() {
        val slots = MemorySlots().apply {
            seed(1, BrowserTabSnapshotCodec.encode(BrowserTabSnapshot(1, listOf(tab("one")), "one")))
        }
        val firstExecutor = QueuedExecutor()
        val secondExecutor = QueuedExecutor()
        val results = Collections.synchronizedList(mutableListOf<DurableBrowserTabStore.WriteResult>())
        val firstStore = store(slots, firstExecutor, results)
        val secondStore = store(slots, secondExecutor, results)
        firstStore.load()
        secondStore.load()

        firstStore.submit(listOf(tab("two")), "two")
        secondStore.submit(listOf(tab("three")), "three")
        secondStore.submit(listOf(tab("four")), "four")

        val ready = CountDownLatch(2)
        val start = CountDownLatch(1)
        val olderWrite = Thread({
            ready.countDown()
            check(start.await(10, TimeUnit.SECONDS))
            firstExecutor.takeNext().run()
        }, "older-snapshot-writer").apply { start() }
        val newerWrite = Thread({
            ready.countDown()
            check(start.await(10, TimeUnit.SECONDS))
            secondExecutor.takeAt(1).run()
        }, "newer-snapshot-writer").apply { start() }
        try {
            assertTrue(
                ready.await(10, TimeUnit.SECONDS),
                "writer threads did not reach the start barrier",
            )
            start.countDown()
        } finally {
            start.countDown()
        }
        olderWrite.join(TimeUnit.SECONDS.toMillis(10))
        newerWrite.join(TimeUnit.SECONDS.toMillis(10))

        assertTrue(!olderWrite.isAlive, "older writer did not finish")
        assertTrue(!newerWrite.isAlive, "newer writer did not finish")
        assertEquals(
            setOf(2L, 4L),
            results.map { result ->
                when (result) {
                    is DurableBrowserTabStore.WriteResult.Persisted -> result.generation
                    is DurableBrowserTabStore.WriteResult.Superseded -> result.generation
                    is DurableBrowserTabStore.WriteResult.Failed -> result.generation
                }
            }.toSet(),
        )
        val latest = store(slots).load()
        assertEquals(4L, latest?.generation)
        assertEquals(listOf("four"), latest?.tabs?.map { it.id })
        assertEquals(listOf(4L, 1L), slots.generationsBySlot())
    }

    @Test
    fun `load waits for an in-progress snapshot replacement`() {
        val previous = BrowserTabSnapshot(1, listOf(tab("previous")), "previous")
        val slots = BlockingWriteSlots().apply {
            seed(1, BrowserTabSnapshotCodec.encode(previous))
        }
        val executor = QueuedExecutor()
        val store = store(slots, executor)
        assertEquals(1L, store.load()?.generation)

        slots.blockNextWrite()
        store.submit(listOf(tab("replacement")), "replacement")
        val writer = Thread(executor.takeNext(), "snapshot-writer").apply { start() }
        assertTrue(slots.writeStarted.await(10, TimeUnit.SECONDS), "writer did not reach the write barrier")

        val loadStarted = CountDownLatch(1)
        val loaded = java.util.concurrent.atomic.AtomicReference<BrowserTabSnapshot?>()
        val reader = Thread({
            loadStarted.countDown()
            loaded.set(store.load())
        }, "snapshot-loader").apply { start() }
        try {
            assertTrue(loadStarted.await(10, TimeUnit.SECONDS), "loader did not start")
            assertEquals(Thread.State.BLOCKED, awaitThreadState(reader, Thread.State.BLOCKED))
        } finally {
            slots.releaseWrite()
        }

        writer.join(TimeUnit.SECONDS.toMillis(10))
        reader.join(TimeUnit.SECONDS.toMillis(10))
        assertTrue(!writer.isAlive, "writer did not finish")
        assertTrue(!reader.isAlive, "loader did not finish")
        assertEquals(2L, loaded.get()?.generation)
    }

    @Test
    fun `default writer serializes alternating submissions from shared-storage instances`() {
        val slots = MemorySlots()
        val results = Collections.synchronizedList(mutableListOf<DurableBrowserTabStore.WriteResult>())
        val completed = CountDownLatch(3)
        val firstStore = productionStore(slots, results, completed)
        val secondStore = productionStore(slots, results, completed)

        firstStore.submit(listOf(tab("first")), "first")
        secondStore.submit(listOf(tab("second")), "second")
        firstStore.submit(listOf(tab("third")), "third")

        assertTrue(completed.await(10, TimeUnit.SECONDS), "writer did not finish all submissions")
        assertEquals(
            listOf(
                DurableBrowserTabStore.WriteResult.Persisted(1),
                DurableBrowserTabStore.WriteResult.Persisted(2),
                DurableBrowserTabStore.WriteResult.Persisted(3),
            ),
            results.sortedBy { result ->
                when (result) {
                    is DurableBrowserTabStore.WriteResult.Persisted -> result.generation
                    is DurableBrowserTabStore.WriteResult.Superseded -> result.generation
                    is DurableBrowserTabStore.WriteResult.Failed -> result.generation
                }
            },
        )

        val snapshots = slots.generationsBySlot()
        assertEquals(listOf(2L, 3L), snapshots)
        val latest = slots.read(1)?.let(BrowserTabSnapshotCodec::decode)
        assertNotNull(latest)
        assertEquals(3L, latest.generation)
        assertEquals(listOf("third"), latest.tabs.map { it.id })
        assertEquals("third", latest.activeTabId)
        assertTrue(
            slots.read(1)!!.contentEquals(
                BrowserTabSnapshotCodec.encode(BrowserTabSnapshot(3, listOf(tab("third")), "third")),
            ),
        )
    }

    @Test
    fun `write failure is reported and later snapshot can be persisted`() {
        val slots = MemorySlots(failNextWrite = true)
        val executor = QueuedExecutor()
        val results = mutableListOf<DurableBrowserTabStore.WriteResult>()
        val store = store(slots, executor, results)

        store.submit(listOf(tab("one")), "one")
        executor.runNext()
        store.submit(listOf(tab("two")), "two")
        executor.runNext()

        assertIs<DurableBrowserTabStore.WriteResult.Failed>(results[0])
        assertEquals(DurableBrowserTabStore.WriteResult.Persisted(2), results[1])
        assertEquals(2, store(slots).load()?.generation)
        assertTrue(results[0].let { it is DurableBrowserTabStore.WriteResult.Failed && it.error is IOException })
    }

    @Test
    fun `failed replacement preserves the previous valid snapshot`() {
        val previous = BrowserTabSnapshot(1, listOf(tab("previous")), "previous")
        val slots = MemorySlots(failNextWrite = true).apply {
            seed(1, BrowserTabSnapshotCodec.encode(previous))
        }
        val results = mutableListOf<DurableBrowserTabStore.WriteResult>()
        val store = store(slots, results = results)

        assertEquals(1L, store.load()?.generation)
        store.submit(listOf(tab("replacement")), "replacement")

        assertIs<DurableBrowserTabStore.WriteResult.Failed>(results.single())
        assertEquals(1L, store(slots).load()?.generation)
        assertEquals(listOf("previous"), store(slots).load()?.tabs?.map { it.id })
    }

    @Test
    fun `read back verification rejects corruption and restores the previous slot`() {
        val previous = BrowserTabSnapshot(1, listOf(tab("previous")), "previous")
        val slots = MemorySlots(corruptNextWrite = true).apply {
            seed(1, BrowserTabSnapshotCodec.encode(previous))
        }
        val results = mutableListOf<DurableBrowserTabStore.WriteResult>()
        val store = store(slots, results = results)

        assertEquals(1L, store.load()?.generation)
        store.submit(listOf(tab("newest")), "newest")

        val failure = assertIs<DurableBrowserTabStore.WriteResult.Failed>(results.single())
        assertEquals(2L, failure.generation)
        assertTrue(failure.error.message.orEmpty().contains("verification failed"))
        assertEquals(listOf(2L), slots.writeTrace)

        val recovered = store(slots).load()
        assertNotNull(recovered)
        assertEquals(1L, recovered.generation)
        assertEquals(listOf("previous"), recovered.tabs.map { it.id })
    }

    @Test
    fun `missing active ID falls back to a tab in the submitted snapshot`() {
        val slots = MemorySlots()
        val executor = QueuedExecutor()
        val results = mutableListOf<DurableBrowserTabStore.WriteResult>()
        val store = store(slots, executor, results)

        store.submit(listOf(tab("one"), tab("two")), "missing")
        executor.runNext()

        assertEquals(DurableBrowserTabStore.WriteResult.Persisted(1), results.single())
        assertEquals("two", store(slots).load()?.activeTabId)
        assertNull(BrowserTabSnapshotCodec.decode(byteArrayOf()))
    }

    private fun store(
        slots: SnapshotSlotStorage,
        executor: Executor = Executor { it.run() },
        results: MutableList<DurableBrowserTabStore.WriteResult> = mutableListOf(),
    ) = DurableBrowserTabStore(
        slots = slots,
        executor = executor,
        onReadError = {},
        onWriteResult = { results += it },
    )

    private fun productionStore(
        slots: MemorySlots,
        results: MutableList<DurableBrowserTabStore.WriteResult>,
        completed: CountDownLatch,
    ) = DurableBrowserTabStore(
        slots = slots,
        onReadError = {},
        onWriteResult = {
            results += it
            completed.countDown()
        },
    )

    private fun tab(id: String) = TabState(id = id, url = "https://$id.example")

    private class QueuedExecutor : Executor {
        private val tasks = mutableListOf<Runnable>()

        override fun execute(command: Runnable) {
            tasks += command
        }

        fun runNext() {
            tasks.removeAt(0).run()
        }

        fun takeNext(): Runnable = tasks.removeAt(0)

        fun takeAt(index: Int): Runnable = tasks.removeAt(index)

        fun runReverse() {
            tasks.asReversed().toList().forEach(Runnable::run)
            tasks.clear()
        }
    }

    private class MemorySlots(
        var failNextWrite: Boolean = false,
        var corruptNextWrite: Boolean = false,
    ) : SnapshotSlotStorage {
        override val identity: String = UUID.randomUUID().toString()
        private val slots = arrayOfNulls<ByteArray>(2)
        val writeTrace = mutableListOf<Long>()

        override fun read(slot: Int): ByteArray? = slots[slot]?.copyOf()

        override fun writeAtomically(slot: Int, bytes: ByteArray) {
            if (failNextWrite) {
                failNextWrite = false
                throw IOException("Injected storage failure")
            }
            writeTrace += BrowserTabSnapshotCodec.decode(bytes)?.generation ?: -1L
            slots[slot] = if (corruptNextWrite) {
                corruptNextWrite = false
                bytes.copyOf().also { it[it.lastIndex] = (it.last().toInt() xor 1).toByte() }
            } else {
                bytes.copyOf()
            }
        }

        fun seed(slot: Int, bytes: ByteArray) {
            slots[slot] = bytes.copyOf()
        }

        fun corrupt(slot: Int) {
            slots[slot] = byteArrayOf(1, 2, 3)
        }

        fun generationsBySlot(): List<Long?> =
            slots.map { encoded -> encoded?.let(BrowserTabSnapshotCodec::decode)?.generation }
    }

    private fun awaitThreadState(thread: Thread, expected: Thread.State): Thread.State {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
        while (thread.state != expected && thread.state != Thread.State.TERMINATED &&
            System.nanoTime() < deadline
        ) {
            Thread.yield()
        }
        return thread.state
    }

    private class BlockingWriteSlots : SnapshotSlotStorage {
        override val identity: String = UUID.randomUUID().toString()
        private val values = arrayOfNulls<ByteArray>(2)
        private var shouldBlockNextWrite = false
        private val finishWrite = CountDownLatch(1)
        val writeStarted = CountDownLatch(1)

        override fun read(slot: Int): ByteArray? = synchronized(this) { values[slot]?.copyOf() }

        override fun writeAtomically(slot: Int, bytes: ByteArray) {
            if (shouldBlockNextWrite) {
                shouldBlockNextWrite = false
                writeStarted.countDown()
                if (!finishWrite.await(10, TimeUnit.SECONDS)) {
                    throw IOException("Test write was not released")
                }
            }
            synchronized(this) {
                values[slot] = bytes.copyOf()
            }
        }

        fun seed(slot: Int, bytes: ByteArray) {
            synchronized(this) {
                values[slot] = bytes.copyOf()
            }
        }

        fun blockNextWrite() {
            shouldBlockNextWrite = true
        }

        fun releaseWrite() {
            finishWrite.countDown()
        }
    }
}
