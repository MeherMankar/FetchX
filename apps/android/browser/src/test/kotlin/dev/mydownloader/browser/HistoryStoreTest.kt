package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class HistoryStoreTest {

    @Test fun `recordNavigation adds entry`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://example.com", 1000L)
        assertEquals(1, store.entriesForTab("t1").size)
        assertEquals("https://example.com", store.entriesForTab("t1").first().url)
    }

    @Test fun `consecutive duplicate URL is not added`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://example.com", 1000L)
        store.recordNavigation("t1", "https://example.com", 2000L)
        assertEquals(1, store.entriesForTab("t1").size)
    }

    @Test fun `different URLs are added`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://a.com", 1000L)
        store.recordNavigation("t1", "https://b.com", 2000L)
        assertEquals(2, store.entriesForTab("t1").size)
    }

    @Test fun `entries are newest first`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://a.com", 1000L)
        store.recordNavigation("t1", "https://b.com", 2000L)
        assertEquals("https://b.com", store.entriesForTab("t1").first().url)
    }

    @Test fun `updateTitle amends matching entry`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://a.com", 1000L)
        store.updateTitle("t1", "https://a.com", "My Title")
        assertEquals("My Title", store.entriesForTab("t1").first().title)
    }

    @Test fun `updateTitle does not affect wrong URL`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://a.com", 1000L)
        store.updateTitle("t1", "https://b.com", "Wrong Title")
        assertNull(store.entriesForTab("t1").first().title)
    }

    @Test fun `tab history is independent per tab`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://a.com", 1000L)
        store.recordNavigation("t2", "https://b.com", 2000L)
        assertEquals(1, store.entriesForTab("t1").size)
        assertEquals(1, store.entriesForTab("t2").size)
        assertEquals("https://a.com", store.entriesForTab("t1").first().url)
        assertEquals("https://b.com", store.entriesForTab("t2").first().url)
    }

    @Test fun `clearTab removes only that tab history`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://a.com", 1000L)
        store.recordNavigation("t2", "https://b.com", 2000L)
        store.clearTab("t1")
        assertTrue(store.entriesForTab("t1").isEmpty())
        assertEquals(1, store.entriesForTab("t2").size)
    }

    @Test fun `clearAll removes all history`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://a.com", 1000L)
        store.recordNavigation("t2", "https://b.com", 2000L)
        store.clearAll()
        assertEquals(0, store.totalCount())
    }

    @Test fun `removeTab removes its history`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://a.com", 1000L)
        store.removeTab("t1")
        assertTrue(store.entriesForTab("t1").isEmpty())
    }

    @Test fun `about blank is not recorded`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "about:blank", 1000L)
        assertTrue(store.entriesForTab("t1").isEmpty())
    }

    @Test fun `blank URL is not recorded`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "", 1000L)
        store.recordNavigation("t1", "   ", 1000L)
        assertTrue(store.entriesForTab("t1").isEmpty())
    }

    @Test fun `maxEntries cap is respected`() {
        val store = HistoryStore(maxEntries = 5)
        for (i in 1..10) {
            store.recordNavigation("t1", "https://site$i.com", i.toLong() * 1000)
        }
        assertEquals(5, store.entriesForTab("t1").size)
    }

    @Test fun `allEntries returns sorted newest first`() {
        val store = HistoryStore()
        store.recordNavigation("t1", "https://a.com", 1000L)
        store.recordNavigation("t2", "https://b.com", 3000L)
        store.recordNavigation("t1", "https://c.com", 2000L)
        val all = store.allEntries()
        assertEquals("https://b.com", all[0].url)   // timestamp 3000
        assertEquals("https://c.com", all[1].url)   // timestamp 2000
        assertEquals("https://a.com", all[2].url)   // timestamp 1000
    }
}
