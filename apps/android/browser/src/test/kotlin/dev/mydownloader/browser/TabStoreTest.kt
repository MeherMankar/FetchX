package dev.mydownloader.browser

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
    fun `clear empties all tabs and active id`() {
        val store = TabStore()
        store.addTab(makeTab("t1"))
        store.addTab(makeTab("t2"))
        store.clear()
        assertEquals(0, store.tabCount())
        assertNull(store.activeTabId())
    }
}
