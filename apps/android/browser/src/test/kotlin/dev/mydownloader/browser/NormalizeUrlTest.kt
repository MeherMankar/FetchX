package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals

class NormalizeUrlTest {

    @Test
    fun blankInputFallsBackToExample() =
        assertEquals("https://example.com", normalizeUrl(""))

    @Test
    fun whitespaceInputFallsBackToExample() =
        assertEquals("https://example.com", normalizeUrl("   "))

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
}

