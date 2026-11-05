package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class BrowserDataCodecTest {
    @Test
    fun `tabs round trip fields containing legacy delimiters and line breaks`() {
        val tabs = listOf(
            TabState(
                id = "tab\t\tone",
                url = "https://example.com/path?value=a\t\tb",
                title = "Title\nwith newline",
                spaceId = "research\nspace",
                desktopSiteEnabled = true,
            ),
        )

        assertEquals(tabs, BrowserDataCodec.decodeTabs(BrowserDataCodec.encodeTabs(tabs)))
    }

    @Test
    fun `bookmarks round trip arbitrary text fields`() {
        val bookmarks = listOf(
            Bookmark(
                id = "bookmark\nid",
                url = "https://example.com/a\t\tb",
                title = "A\nbookmark",
                createdAtMs = 12L,
                updatedAtMs = 34L,
            ),
        )

        assertEquals(bookmarks, BrowserDataCodec.decodeBookmarks(BrowserDataCodec.encodeBookmarks(bookmarks)))
    }

    @Test
    fun `history round trip optional titles and URLs`() {
        val history = listOf(
            HistoryEntry("id1", "https://example.com/?q=a,b", null, 123L),
            HistoryEntry("id2", "https://example.org", "Saved title", 456L),
        )

        assertEquals(history, BrowserDataCodec.decodeHistory(BrowserDataCodec.encodeHistory(history)))
    }

    @Test
    fun `legacy tab and bookmark formats remain readable`() {
        assertEquals(
            listOf(TabState(id = "tab", url = "https://example.com", title = "Title", spaceId = "default")),
            BrowserDataCodec.decodeTabs("tab\t\thttps://example.com\t\tTitle\t\tdefault"),
        )
        assertEquals(
            listOf(Bookmark("id", "https://example.com", "Example", 1L, 2L)),
            BrowserDataCodec.decodeBookmarks("id\t\thttps://example.com\t\tExample\t\t1\t\t2"),
        )
    }

    @Test
    fun `version one tabs remain readable with mobile mode default`() {
        assertEquals(
            listOf(
                TabState(
                    id = "tab",
                    url = "https://example.com",
                    title = "Title",
                    spaceId = "default",
                ),
            ),
            BrowserDataCodec.decodeTabs("v1:dGFi.aHR0cHM6Ly9leGFtcGxlLmNvbQ.VGl0bGU.ZGVmYXVsdA"),
        )
    }

    @Test
    fun `four tab snapshot preserves IDs and selected tab across close and restore`() {
        val store = TabStore()
        val originalIds = listOf("t1", "t2", "t3", "t4")
        originalIds.forEach { id ->
            store.addTab(
                TabState(
                    id = id,
                    url = "https://$id.example.com",
                    title = id,
                    spaceId = "default",
                ),
            )
        }

        assertEquals(originalIds, BrowserDataCodec.decodeTabs(BrowserDataCodec.encodeTabs(store.allTabs())).map { it.id })

        assertTrue(store.setActive("t2"))
        assertEquals(3, store.removeTab("t4"))
        val savedTabs = BrowserDataCodec.decodeTabs(BrowserDataCodec.encodeTabs(store.allTabs()))
        val savedActiveId = store.activeTabId()
        val restored = TabStore()
        savedTabs.forEach { restored.addTab(it) }
        assertTrue(restored.setActive(resolveRestoredActiveTabId(savedTabs, savedActiveId)))

        assertEquals(listOf("t1", "t2", "t3"), restored.allTabs().map { it.id })
        assertEquals("t2", restored.activeTabId())
    }

    @Test
    fun `malformed and duplicate tab records are safely ignored`() {
        val valid = TabState(
            id = "tab",
            url = "https://example.com",
            spaceId = "default",
        )
        val duplicate = valid.copy(url = "https://duplicate.example.com")
        val encoded = BrowserDataCodec.encodeTabs(listOf(valid, duplicate))

        assertEquals(listOf(valid), BrowserDataCodec.decodeTabs(encoded))
        assertEquals(emptyList(), BrowserDataCodec.decodeTabs("v2:%%%"))
        assertEquals(emptyList(), BrowserDataCodec.decodeTabs("v2:"))
    }
}
