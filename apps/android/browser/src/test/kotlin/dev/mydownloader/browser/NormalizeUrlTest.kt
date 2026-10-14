package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals

class NormalizeUrlTest {

    @Test
    fun blankInputOpensNewTabPage() =
        assertEquals("about:blank", normalizeUrl(""))

    @Test
    fun whitespaceInputOpensNewTabPage() =
        assertEquals("about:blank", normalizeUrl("   "))

    @Test
    fun aboutBlankPassesThrough() =
        assertEquals("about:blank", normalizeUrl("about:blank"))

    @Test
    fun httpsUrlPassesThrough() =
        assertEquals("https://example.com", normalizeUrl("https://example.com"))

    @Test
    fun httpUrlPassesThrough() =
        assertEquals("http://example.com", normalizeUrl("http://example.com"))

    @Test
    fun bareHostGetsPrefixed() =
        assertEquals("https://example.com", normalizeUrl("example.com"))

    @Test
    fun subdirectoryPathGetsPrefixed() =
        assertEquals("https://example.com/page", normalizeUrl("example.com/page"))

    @Test
    fun trimmingWhitespaceAroundUrl() =
        assertEquals("https://example.com", normalizeUrl("  https://example.com  "))

    @Test
    fun ipAddressGetsPrefixed() =
        assertEquals("https://192.168.1.1", normalizeUrl("192.168.1.1"))

    @Test
    fun plainSiteNameUsesSelectedSearchEngine() =
        assertEquals(
            "https://search.brave.com/search?q=instagram",
            normalizeUrl("instagram", SearchEngine.BRAVE),
        )

    @Test
    fun plainTextUsesSelectedSearchEngine() =
        assertEquals(
            "https://www.google.com/search?q=how+to+code",
            normalizeUrl("how to code", SearchEngine.GOOGLE),
        )
}
