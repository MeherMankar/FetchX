package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNull
import kotlin.test.assertTrue

class BrowserSearchTest {
    private val provider = UrlTemplateSearchProvider("https://search.example/?q=%s")

    @Test
    fun recognizesHttpAndBareDomainAddresses() {
        assertEquals(
            SearchIntent.NavigateUrl("https://example.com/path"),
            SearchIntentClassifier.classify("example.com/path", emptyList(), provider),
        )
        assertEquals(
            SearchIntent.NavigateUrl("http://example.com"),
            SearchIntentClassifier.classify("http://example.com", emptyList(), provider),
        )
    }

    @Test
    fun recognizesLocalhostAndIpAddresses() {
        assertEquals(
            "https://localhost:8080",
            SearchIntentClassifier.normalizeBrowserUrl("localhost:8080"),
        )
        assertEquals(
            "https://192.168.1.1",
            SearchIntentClassifier.normalizeBrowserUrl("192.168.1.1"),
        )
    }

    @Test
    fun rejectsUnsafeAndCredentialBearingUrls() {
        assertNull(SearchIntentClassifier.normalizeBrowserUrl("javascript:alert(1)"))
        assertNull(SearchIntentClassifier.normalizeBrowserUrl("https://user:pass@example.com"))
    }

    @Test
    fun treatsFileLikeQueriesAsLocalSearch() {
        assertIs<SearchIntent.LocalSearch>(
            SearchIntentClassifier.classify("video.mp4", emptyList(), provider),
        )
        assertEquals(
            SearchIntent.NavigateUrl("https://example.com/video.mp4"),
            SearchIntentClassifier.classify("https://example.com/video.mp4", emptyList(), provider),
        )
    }

    @Test
    fun treatsOrdinaryTextAsWebSearch() {
        assertEquals(
            SearchIntent.WebSearch("how to code", "https://search.example/?q=how+to+code"),
            SearchIntentClassifier.classify("how to code", emptyList(), provider),
        )
    }

    @Test
    fun encodesSearchQueries() {
        assertEquals(
            "https://search.example/?q=hello+%26+goodbye",
            provider.searchUrl("hello & goodbye"),
        )
    }

    @Test(expected = IllegalArgumentException::class)
    fun rejectsNonHttpsSearchTemplates() {
        UrlTemplateSearchProvider("http://search.example/?q=%s")
    }

    @Test(expected = IllegalArgumentException::class)
    fun rejectsSearchTemplatesWithMultiplePlaceholders() {
        UrlTemplateSearchProvider("https://search.example/?q=%s&other=%s")
    }

    @Test
    fun searchesAndRanksBrowserOwnedResults() {
        val results = BrowserLocalSearch.search(
            query = "kotlin",
            bookmarks = listOf(
                Bookmark("bookmark", "https://kotlinlang.org", "Kotlin", 1L, 2L),
            ),
            history = listOf(
                HistoryEntry("history", "https://example.com/kotlin", "Learn Kotlin", 10L),
            ),
            tabs = listOf(
                TabState("tab", "https://docs.example/kotlin", "Kotlin docs"),
            ),
        )

        assertEquals(
            listOf(
                BrowserSearchResultType.Bookmark,
                BrowserSearchResultType.Tab,
                BrowserSearchResultType.History,
            ),
            results.map { it.type },
        )
        assertEquals("tab", results[1].tabId)
    }

    @Test
    fun boundsLocalResults() {
        val bookmarks = (1..25).map { index ->
            Bookmark(
                id = "bookmark-$index",
                url = "https://example.com/kotlin/$index",
                title = "Kotlin $index",
                createdAtMs = index.toLong(),
                updatedAtMs = index.toLong(),
            )
        }

        val results = BrowserLocalSearch.search("kotlin", bookmarks, emptyList(), emptyList(), limit = 5)

        assertEquals(5, results.size)
        assertTrue(results.all { it.type == BrowserSearchResultType.Bookmark })
    }
}
