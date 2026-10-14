package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class EssentialsStoreTest {

    @Test
    fun `most visited sites are ordered by visit count and grouped by host`() {
        val store = EssentialsStore()
        val history = listOf(
            HistoryEntry("1", "https://www.example.com/first", "First", 1_000L),
            HistoryEntry("2", "https://other.com/", "Other", 2_000L),
            HistoryEntry("3", "https://example.com/second", "Second", 3_000L),
            HistoryEntry("4", "https://example.com/third", "Third", 4_000L),
        )

        val sites = store.mostVisited(history)

        assertEquals(listOf("Third", "Other"), sites.map { it.title })
        assertEquals("https://example.com", sites.first().url)
    }

    @Test
    fun `shortcut uses the visited page title rather than a hardcoded site label`() {
        val sites = EssentialsStore().mostVisited(
            listOf(HistoryEntry("1", "https://www.instagram.com/reel", "Instagram", 1_000L)),
        )

        assertEquals("Instagram", sites.single().title)
        assertEquals("I", sites.single().initial)
    }

    @Test
    fun `new tab shortcuts exclude non-web and invalid history entries`() {
        val store = EssentialsStore()
        val history = listOf(
            HistoryEntry("1", "about:blank", null, 1_000L),
            HistoryEntry("2", "not a url", null, 2_000L),
            HistoryEntry("3", "https://visited.example/path", null, 3_000L),
        )

        val sites = store.mostVisited(history)

        assertEquals(1, sites.size)
        assertEquals("visited.example", sites.single().title)
        assertTrue(sites.single().url.startsWith("https://"))
    }
}
