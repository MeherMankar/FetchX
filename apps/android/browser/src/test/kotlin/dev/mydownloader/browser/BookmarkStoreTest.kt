package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class BookmarkStoreTest {

    @Test fun `add returns bookmark with correct url and title`() {
        val store = BookmarkStore()
        val b = store.add("https://example.com", "Example")
        assertNotNull(b)
        assertEquals("https://example.com", b.url)
        assertEquals("Example", b.title)
    }

    @Test fun `add returns null for blank url`() {
        val store = BookmarkStore()
        assertNull(store.add("", "Title"))
        assertNull(store.add("   ", "Title"))
    }

    @Test fun `add returns null for url without http or https scheme`() {
        val store = BookmarkStore()
        assertNull(store.add("ftp://example.com", "FTP"))
        assertNull(store.add("example.com", "No scheme"))
        assertNull(store.add("about:blank", "Blank"))
        assertNull(store.add("file:///sdcard/file.html", "File"))
    }

    @Test fun `add accepts http url`() {
        val store = BookmarkStore()
        val b = store.add("http://example.com", "HTTP")
        assertNotNull(b)
        assertEquals("http://example.com", b.url)
    }

    @Test fun `add accepts https url`() {
        val store = BookmarkStore()
        val b = store.add("https://example.com", "HTTPS")
        assertNotNull(b)
        assertEquals("https://example.com", b.url)
    }

    @Test fun `add returns null for duplicate url`() {
        val store = BookmarkStore()
        assertNotNull(store.add("https://example.com", "First"))
        assertNull(store.add("https://example.com", "Second"))
        assertEquals(1, store.count())
    }

    @Test fun `all returns newest first`() {
        val store = BookmarkStore()
        store.add("https://a.com", "A", nowMs = 1000L)
        store.add("https://b.com", "B", nowMs = 2000L)
        val all = store.all()
        assertEquals("https://b.com", all[0].url)
        assertEquals("https://a.com", all[1].url)
    }

    @Test fun `isBookmarked returns true after add`() {
        val store = BookmarkStore()
        store.add("https://example.com", "Example")
        assertTrue(store.isBookmarked("https://example.com"))
    }

    @Test fun `isBookmarked returns false for unknown url`() {
        val store = BookmarkStore()
        assertFalse(store.isBookmarked("https://unknown.com"))
    }

    @Test fun `removeById removes the bookmark`() {
        val store = BookmarkStore()
        val b = store.add("https://example.com", "Example")!!
        assertTrue(store.removeById(b.id))
        assertEquals(0, store.count())
        assertFalse(store.isBookmarked("https://example.com"))
    }

    @Test fun `removeById returns false for unknown id`() {
        val store = BookmarkStore()
        assertFalse(store.removeById("non-existent-id"))
    }

    @Test fun `removeByUrl removes the bookmark`() {
        val store = BookmarkStore()
        store.add("https://example.com", "Example")
        assertTrue(store.removeByUrl("https://example.com"))
        assertEquals(0, store.count())
    }

    @Test fun `removeByUrl returns false for unknown url`() {
        val store = BookmarkStore()
        assertFalse(store.removeByUrl("https://unknown.com"))
    }

    @Test fun `update changes title`() {
        val store = BookmarkStore()
        val b = store.add("https://example.com", "Original")!!
        assertTrue(store.update(b.id, "Updated"))
        assertEquals("Updated", store.findByUrl("https://example.com")?.title)
    }

    @Test fun `update returns false for blank new title`() {
        val store = BookmarkStore()
        val b = store.add("https://example.com", "Title")!!
        assertFalse(store.update(b.id, ""))
        assertFalse(store.update(b.id, "   "))
        // Title is unchanged
        assertEquals("Title", store.findByUrl("https://example.com")?.title)
    }

    @Test fun `update returns false for unknown id`() {
        val store = BookmarkStore()
        assertFalse(store.update("non-existent-id", "New Title"))
    }

    @Test fun `findByUrl returns bookmark after add`() {
        val store = BookmarkStore()
        store.add("https://example.com", "Example")
        val b = store.findByUrl("https://example.com")
        assertNotNull(b)
        assertEquals("https://example.com", b.url)
    }

    @Test fun `findByUrl trims whitespace on input`() {
        val store = BookmarkStore()
        store.add("https://example.com", "Example")
        val b = store.findByUrl("  https://example.com  ")
        assertNotNull(b)
        assertEquals("https://example.com", b.url)
    }

    @Test fun `findByUrl returns null for unknown url`() {
        val store = BookmarkStore()
        assertNull(store.findByUrl("https://unknown.com"))
    }

    @Test fun `clearAll removes all bookmarks`() {
        val store = BookmarkStore()
        store.add("https://a.com", "A")
        store.add("https://b.com", "B")
        store.clearAll()
        assertEquals(0, store.count())
        assertTrue(store.all().isEmpty())
    }

    @Test fun `count returns correct size`() {
        val store = BookmarkStore()
        assertEquals(0, store.count())
        store.add("https://a.com", "A")
        assertEquals(1, store.count())
        store.add("https://b.com", "B")
        assertEquals(2, store.count())
    }

    @Test fun `blank title falls back to host`() {
        val store = BookmarkStore()
        val b = store.add("https://example.com", "")
        assertNotNull(b)
        assertEquals("example.com", b.title)
    }

    @Test fun `clearing history does not clear bookmarks`() {
        val bookmarkStore = BookmarkStore()
        bookmarkStore.add("https://example.com", "Example")
        val historyStore = HistoryStore()
        historyStore.recordNavigation("t1", "https://example.com", 1000L)
        // Clear history — must not affect bookmarks
        historyStore.clearAll()
        assertEquals(0, historyStore.totalCount())
        assertEquals(1, bookmarkStore.count())
        assertTrue(bookmarkStore.isBookmarked("https://example.com"))
    }

    @Test fun `removing bookmark does not affect history`() {
        val bookmarkStore = BookmarkStore()
        bookmarkStore.add("https://example.com", "Example")
        val historyStore = HistoryStore()
        historyStore.recordNavigation("t1", "https://example.com", 1000L)
        // Remove bookmark — history must remain intact
        bookmarkStore.removeByUrl("https://example.com")
        assertEquals(0, bookmarkStore.count())
        assertEquals(1, historyStore.entriesForTab("t1").size)
        assertEquals("https://example.com", historyStore.entriesForTab("t1").first().url)
    }
}
