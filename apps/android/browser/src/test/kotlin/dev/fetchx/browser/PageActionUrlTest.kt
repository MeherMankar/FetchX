package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class PageActionUrlTest {
    @Test
    fun `page actions accept valid HTTP and HTTPS URLs`() {
        assertTrue(isHttpOrHttpsPageUrl("http://example.com/path"))
        assertTrue(isHttpOrHttpsPageUrl("https://example.com/path?q=value"))
        assertTrue(isHttpOrHttpsPageUrl("HTTPS://example.com"))
    }

    @Test
    fun `page actions reject internal and unsupported schemes`() {
        assertFalse(isHttpOrHttpsPageUrl(""))
        assertFalse(isHttpOrHttpsPageUrl("about:blank"))
        assertFalse(isHttpOrHttpsPageUrl("file:///data/page.html"))
        assertFalse(isHttpOrHttpsPageUrl("data:text/html,content"))
        assertFalse(isHttpOrHttpsPageUrl("javascript:alert(1)"))
        assertFalse(isHttpOrHttpsPageUrl("ftp://example.com/file"))
        assertFalse(isHttpOrHttpsPageUrl("https:///missing-host"))
    }
}
