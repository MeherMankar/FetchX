package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class TabStoreTest {

    private fun makeTab(id: String, url: String = "https://example.com") =
        TabState(id = id, url = url)

    @Test
    fun `empty store has no active tab`() {
        val store = TabStore()
        assertNull(store.activeTabId())
        assertEquals(0, store.tabCount())
    }

    @Test
    fun `addTab makes it active`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        assertEquals("t1", store.activeTabId())
        assertEquals(1, store.tabCount())
    }

    @Test
    fun `addTab with two tabs — second becomes active`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        store.addTab(makeTab("t2"))
        assertEquals("t2", store.activeTabId())
    }

    @Test
    fun `explicit space assignment preserves default tabs and existing assignments`() {
        val store = TabStore()
        store.addTab(makeTab("existing-default"))
        store.addTab(makeTab("existing-work").copy(spaceId = "work"))
        store.addTab(makeTab("new-work").copy(spaceId = "work"))
        store.addTab(makeTab("new-default"))

        assertEquals(
            listOf("existing-default", "new-default"),
            store.allTabsForSpace("default").map { it.id },
        )
        assertEquals(
            listOf("existing-work", "new-work"),
            store.allTabsForSpace("work").map { it.id },
        )
        assertEquals(
            listOf("default", "work", "work", "default"),
            store.allTabs().map { it.spaceId },
        )
    }

    @Test
    fun `setActive switches active tab`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        store.addTab(makeTab("t2"))
        assertTrue(store.setActive("t1"))
        assertEquals("t1", store.activeTabId())
    }

    @Test
    fun `setActive with unknown id returns false`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        assertFalse(store.setActive("unknown"))
        assertEquals("t1", store.activeTabId())
    }

    @Test
    fun `removeTab active tab — selects adjacent tab`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        store.addTab(makeTab("t2"))
        store.addTab(makeTab("t3"))
        store.setActive("t2")
        store.removeTab("t2")
        // t3 takes index 1 after removal, so it is selected
        assertEquals("t3", store.activeTabId())
        assertEquals(2, store.tabCount())
    }

    @Test
    fun `removeTab last remaining — active becomes null`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        store.removeTab("t1")
        assertNull(store.activeTabId())
        assertEquals(0, store.tabCount())
    }

    @Test
    fun `removeTab non-active tab — active unchanged`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        store.addTab(makeTab("t2"))
        store.setActive("t1")
        store.removeTab("t2")
        assertEquals("t1", store.activeTabId())
        assertEquals(1, store.tabCount())
    }

    @Test
    fun `updateTab updates state correctly`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        assertTrue(store.updateTab("t1") { it.copy(title = "My Title") })
        assertEquals("My Title", store.activeTab()?.title)
    }

    @Test
    fun `updateTab unknown id returns false`() {
        val store = TabStore()
        assertFalse(store.updateTab("unknown") { it })
    }

    @Test
    fun `allTabs returns stable snapshot copy`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        store.addTab(makeTab("t2"))
        val snapshot = store.allTabs()
        store.addTab(makeTab("t3"))
        assertEquals(2, snapshot.size) // snapshot not affected by later add
    }

    @Test
    fun `stable tab IDs — removing a tab does not change other IDs`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        store.addTab(makeTab("t2"))
        store.addTab(makeTab("t3"))
        store.removeTab("t2")
        val remaining = store.allTabs().map { it.id }
        assertTrue("t1" in remaining)
        assertTrue("t3" in remaining)
        assertFalse("t2" in remaining)
    }

    @Test
    fun `independent tab state — updating one tab does not affect others`() {
        val store = TabStore()
        store.addTab(makeTab("t1", "https://site-a.com"))
        store.addTab(makeTab("t2", "https://site-b.com"))
        store.updateTab("t1") { it.copy(title = "Site A", isLoading = true) }
        val t1 = store.findTab("t1")
        val t2 = store.findTab("t2")
        assertNotNull(t1)
        assertNotNull(t2)
        assertEquals("Site A", t1.title)
        assertTrue(t1.isLoading)
        assertNull(t2.title)
        assertFalse(t2.isLoading)
    }

    @Test
    fun `navigation capabilities are independent and follow the active tab`() {
        val store = TabStore()
        store.addTab(makeTab("t1").copy(canGoBack = true))
        store.addTab(makeTab("t2").copy(canGoForward = true))

        assertFalse(store.activeTab()!!.canGoBack)
        assertTrue(store.activeTab()!!.canGoForward)

        assertTrue(store.setActive("t1"))
        assertTrue(store.activeTab()!!.canGoBack)
        assertFalse(store.activeTab()!!.canGoForward)
    }

    @Test
    fun `navigation callback cannot update a closed tab`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        store.addTab(makeTab("t2"))
        store.removeTab("t1")

        assertFalse(store.updateTab("t1") { it.copy(canGoBack = true) })
        assertFalse(store.activeTab()!!.canGoBack)
        assertFalse(store.activeTab()!!.canGoForward)
    }

    @Test
    fun `clear empties all tabs and active id`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        store.addTab(makeTab("t2"))
        store.clear()
        assertEquals(0, store.tabCount())
        assertNull(store.activeTabId())
    }

    @Test
    fun `sequential create select and close operations preserve the logical tab snapshot`() {
        val store = TabStore()
        fun assertSnapshot(ids: List<String>, activeId: String?) {
            assertEquals(ids, store.allTabs().map { it.id })
            assertEquals(ids.map { "https://$it.example" }, store.allTabs().map { it.url })
            assertEquals(activeId, store.activeTabId())
        }

        store.addTab(makeTab("t1", "https://t1.example"))
        assertSnapshot(listOf("t1"), "t1")

        store.addTab(makeTab("t2", "https://t2.example"))
        assertSnapshot(listOf("t1", "t2"), "t2")

        store.addTab(makeTab("t3", "https://t3.example"))
        assertSnapshot(listOf("t1", "t2", "t3"), "t3")

        store.addTab(makeTab("t4", "https://t4.example"))
        assertSnapshot(listOf("t1", "t2", "t3", "t4"), "t4")

        assertTrue(store.setActive("t2"))
        assertSnapshot(listOf("t1", "t2", "t3", "t4"), "t2")

        store.removeTab("t4")
        assertSnapshot(listOf("t1", "t2", "t3"), "t2")

        store.removeTab("t2")
        assertSnapshot(listOf("t1", "t3"), "t3")

        store.addTab(makeTab("t5", "https://t5.example"))
        assertSnapshot(listOf("t1", "t3", "t5"), "t5")
        store.removeTab("t5")
        assertSnapshot(listOf("t1", "t3"), "t3")
    }

    @Test
    fun `restored active tab falls back to the last saved tab when missing or invalid`() {
        val tabs = listOf(makeTab("t1"), makeTab("t2"), makeTab("t3"))

        assertEquals("t3", resolveRestoredActiveTabId(tabs, null))
        assertEquals("t3", resolveRestoredActiveTabId(tabs, "missing"))
        assertEquals("t1", resolveRestoredActiveTabId(tabs, "t1"))
    }
}
