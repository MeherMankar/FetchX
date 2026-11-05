package dev.mydownloader.browser

import dev.fetchx.browser.BrowserTabSnapshot
import dev.fetchx.browser.BrowserTabSnapshotCodec
import dev.fetchx.browser.SnapshotSlotStorage
import dev.fetchx.browser.TabState
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicLong

internal class DurableBrowserTabStore(
    private val slots: SnapshotSlotStorage,
    private val executor: Executor = writerExecutor,
    private val onReadError: (Exception) -> Unit,
    private val onWriteResult: (WriteResult) -> Unit,
) {
    sealed interface WriteResult {
        data class Persisted(val generation: Long) : WriteResult
        data class Superseded(val generation: Long, val byGeneration: Long) : WriteResult
        data class Failed(val generation: Long, val error: Exception) : WriteResult
    }

    private val lock = Any()
    private val storageLock = storageLocks.computeIfAbsent(slots.identity) { Any() }
    private val generationSequence = generationSequences.computeIfAbsent(slots.identity) { AtomicLong() }
    private var lastSubmittedGeneration = 0L
    private var lastSubmittedContent: SnapshotContent? = null

    fun load(): BrowserTabSnapshot? {
        val loaded = synchronized(storageLock) { readLatest() }
        synchronized(lock) {
            if (loaded != null) {
                val latestGeneration = generationSequence.updateAndGet { maxOf(it, loaded.generation) }
                lastSubmittedGeneration = maxOf(lastSubmittedGeneration, latestGeneration)
                lastSubmittedContent = contentOf(loaded.tabs, loaded.activeTabId)
            }
        }
        return loaded
    }

    fun submit(tabs: List<TabState>, activeTabId: String?) {
        val selectedId = activeTabId?.takeIf { id -> tabs.any { it.id == id } } ?: tabs.lastOrNull()?.id
        if (tabs.isEmpty() || selectedId == null) {
            onWriteResult(WriteResult.Failed(0, IllegalArgumentException("Cannot persist an empty tab snapshot")))
            return
        }

        val content = contentOf(tabs, selectedId)
        val snapshot: BrowserTabSnapshot
        synchronized(lock) {
            if (content == lastSubmittedContent) return
            val generation = generationSequence.incrementAndGet()
            lastSubmittedGeneration = generation
            snapshot = BrowserTabSnapshot(generation, tabs.toList(), selectedId)
            lastSubmittedContent = content
        }

        try {
            executor.execute {
                val result = try {
                    persist(snapshot, BrowserTabSnapshotCodec.encode(snapshot))
                } catch (error: IOException) {
                    WriteResult.Failed(snapshot.generation, error)
                } catch (error: RuntimeException) {
                    WriteResult.Failed(snapshot.generation, error)
                }
                if (result is WriteResult.Failed) {
                    synchronized(lock) {
                        if (lastSubmittedGeneration == snapshot.generation) {
                            lastSubmittedContent = null
                        }
                    }
                }
                onWriteResult(result)
            }
        } catch (error: java.util.concurrent.RejectedExecutionException) {
            synchronized(lock) {
                if (lastSubmittedGeneration == snapshot.generation) {
                    lastSubmittedContent = null
                }
            }
            onWriteResult(WriteResult.Failed(snapshot.generation, error))
        }
    }

    private fun contentOf(tabs: List<TabState>, activeTabId: String) = SnapshotContent(
        tabs.map { tab ->
            PersistedTabContent(
                id = tab.id,
                url = tab.url,
                title = tab.title,
                spaceId = tab.spaceId,
                desktopSiteEnabled = tab.desktopSiteEnabled,
            )
        },
        activeTabId,
    )

    private fun persist(snapshot: BrowserTabSnapshot, encoded: ByteArray): WriteResult {
        return synchronized(storageLock) {
            val current = readLatest()
            if (current != null && current.generation >= snapshot.generation) {
                return@synchronized WriteResult.Superseded(snapshot.generation, current.generation)
            }

            slots.writeAtomically((snapshot.generation and 1L).toInt(), encoded)

            val verified = readSlot(snapshot.generation)
            if (verified?.generation != snapshot.generation ||
                contentOf(verified.tabs, verified.activeTabId) !=
                contentOf(snapshot.tabs, snapshot.activeTabId)
            ) {
                throw IOException("Browser snapshot verification failed for generation ${snapshot.generation}")
            }
            WriteResult.Persisted(snapshot.generation)
        }
    }

    private fun readLatest(): BrowserTabSnapshot? {
        val candidates = (0..1).mapNotNull { slot ->
            val encoded = try {
                slots.read(slot)
            } catch (error: IOException) {
                onReadError(error)
                throw IOException("Could not read browser snapshot slot $slot", error)
            } catch (error: SecurityException) {
                onReadError(error)
                throw IOException("Could not read browser snapshot slot $slot", error)
            }
            if (encoded == null) {
                null
            } else {
                BrowserTabSnapshotCodec.decode(encoded) ?: run {
                    onReadError(IOException("Ignoring malformed browser snapshot in slot $slot"))
                    null
                }
            }
        }
        return candidates.maxByOrNull { it.generation }
    }

    private fun readSlot(generation: Long): BrowserTabSnapshot? {
        val slot = (generation and 1L).toInt()
        val encoded = slots.read(slot) ?: return null
        return BrowserTabSnapshotCodec.decode(encoded)
    }

    private data class SnapshotContent(
        val tabs: List<PersistedTabContent>,
        val activeTabId: String,
    )

    private data class PersistedTabContent(
        val id: String,
        val url: String,
        val title: String?,
        val spaceId: String,
        val desktopSiteEnabled: Boolean,
    )

    companion object {
        private val generationSequences = ConcurrentHashMap<String, AtomicLong>()
        private val storageLocks = ConcurrentHashMap<String, Any>()
        private val writerExecutor = Executors.newSingleThreadExecutor { task ->
            Thread(task, "fetchx-tab-snapshot-writer").apply { isDaemon = true }
        }
    }
}
