package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals

class BrowserDataCodecTest {
    @Test
    fun `tabs round trip fields containing legacy delimiters and line breaks`() {
        val tabs = listOf(
            TabState(
                id = "tab\t\tone",
                url = "https://example.com/path?value=a\t\tb",
                title = "Title\nwith newline",
                spaceId = "research\nspace",
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
}
