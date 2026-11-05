package dev.fetchx.browser

import dev.mydownloader.browser.DurableBrowserTabStore
import java.util.UUID
import java.util.concurrent.Executor
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class TabIdentityConsistencyTest {
    @Test
    fun `tab store and persisted restore retain IDs and active ID through lifecycle changes`() {
        val store = TabStore()
        val storage = MemorySnapshotSlots("identity-${UUID.randomUUID()}")
        val durableStore = DurableBrowserTabStore(
            slots = storage,
            executor = Executor { command -> command.run() },
            onReadError = { error -> throw AssertionError("Snapshot read failed", error) },
            onWriteResult = { result ->
                assertTrue(
                    result is DurableBrowserTabStore.WriteResult.Persisted,
                    "Snapshot write failed: $result",
                )
            },
        )

        fun assertIdentity(expectedIds: List<String>, expectedActiveId: String?) {
            assertEquals(expectedIds, store.allTabs().map { it.id })
            assertEquals(expectedActiveId, store.activeTabId())
        }

        store.addTab(TabState("default-1", "https://one.example", spaceId = "default"))
        store.addTab(TabState("default-2", "https://two.example", spaceId = "default"))
        store.addTab(TabState("work-1", "https://work.example", spaceId = "work"))
        assertIdentity(listOf("default-1", "default-2", "work-1"), "work-1")

        assertTrue(store.setActive("default-1"))
        assertIdentity(listOf("default-1", "default-2", "work-1"), "default-1")

        assertTrue(
            store.updateTab("default-1") {
                it.copy(url = "https://example.test/five-tab-selected")
            },
        )
        assertEquals(
            listOf("default-1"),
            filterVisibleTabs(store.allTabs(), "five-tab-selected", "default").map { it.id },
        )
        assertEquals(
            listOf("work-1"),
            filterVisibleTabs(
                store.allTabs().map { tab ->
                    if (tab.id == "work-1") tab.copy(title = "FIVE-TAB-SELECTED") else tab
                },
                "five-tab-selected",
                "work",
            ).map { it.id },
        )

        store.removeTab("work-1")
        assertIdentity(listOf("default-1", "default-2"), "default-1")
        store.removeTab("default-1")
        assertIdentity(listOf("default-2"), "default-2")

        store.addTab(TabState("replacement", "https://replacement.example", spaceId = "work"))
        assertIdentity(listOf("default-2", "replacement"), "replacement")

        durableStore.submit(store.allTabs(), store.activeTabId())
        val restored = durableStore.load()
        assertEquals(listOf("default-2", "replacement"), restored?.tabs?.map { it.id })
        assertEquals("replacement", restored?.activeTabId)

        val restoredStore = TabStore()
        restored!!.tabs.forEach(restoredStore::addTab)
        assertTrue(restoredStore.setActive(restored.activeTabId))
        assertEquals(store.allTabs().map { it.id }, restoredStore.allTabs().map { it.id })
        assertEquals(store.activeTabId(), restoredStore.activeTabId())
    }

    private class MemorySnapshotSlots(override val identity: String) : SnapshotSlotStorage {
        private val contents = mutableMapOf<Int, ByteArray>()

        override fun read(slot: Int): ByteArray? = contents[slot]?.copyOf()

        override fun writeAtomically(slot: Int, bytes: ByteArray) {
            contents[slot] = bytes.copyOf()
        }
    }
}
