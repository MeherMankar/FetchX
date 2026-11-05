package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals

class TabSearchTest {
    @Test
    fun `search matches URL even when page title does not contain query`() {
        val tab = TabState(
            id = "selected",
            url = "https://example.com/five-tab-selected",
            title = "Example Domain",
        )

        assertEquals(
            listOf(tab),
            filterVisibleTabs(listOf(tab), "five-tab-selected", null),
        )
    }

    @Test
    fun `search matches title and URL without regard to case`() {
        val tabs = listOf(
            TabState("title", "https://title.example", "FIVE-TAB-SELECTED"),
            TabState("url", "https://example.com/FIVE-TAB-SELECTED", "Other page"),
            TabState("other", "https://other.example", "Other page"),
        )

        assertEquals(
            listOf(tabs[0], tabs[1]),
            filterVisibleTabs(tabs, "five-tab-selected", null),
        )
    }

    @Test
    fun `space filter intersects query results and preserves source order`() {
        val tabs = listOf(
            TabState("work-url", "https://example.com/unique", spaceId = "work"),
            TabState("default-url", "https://example.com/unique", spaceId = "default"),
            TabState("work-title", "https://other.example", "Unique", spaceId = "work"),
        )

        assertEquals(
            listOf(tabs[0], tabs[2]),
            filterVisibleTabs(tabs, "unique", "work"),
        )
        assertEquals(
            listOf(tabs[0], tabs[1], tabs[2]),
            filterVisibleTabs(tabs, "unique", null),
        )
    }

    @Test
    fun `updated navigation URL becomes searchable`() {
        val original = TabState("selected", "about:blank", "New tab")
        val navigated = original.copy(url = "https://example.com/five-tab-selected")

        assertEquals(emptyList(), filterVisibleTabs(listOf(original), "five-tab-selected", null))
        assertEquals(
            listOf(navigated),
            filterVisibleTabs(listOf(navigated), "five-tab-selected", null),
        )
    }
}
