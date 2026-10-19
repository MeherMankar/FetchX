package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * Contract test for [isExternalScheme] — the single source of truth shared by
 * [TabManager.onLoadRequest] and the `MagnetUriRouter` test helper.
 *
 * Because `MagnetUriRouter.handle` now calls [isExternalScheme], the existing
 * `TabManagerMagnetTest` cases also exercise this function; this file pins the
 * exact supported/unsupported contract so tests and production cannot drift.
 */
class ExternalUriSchemesTest {

    // ── Supported schemes (case-insensitive) ───────────────────────────────

    @Test
    fun `mailto is external`() {
        assertTrue(isExternalScheme("mailto:user@example.com"))
    }

    @Test
    fun `MAILTO is matched case-insensitively`() {
        assertTrue(isExternalScheme("MAILTO:user@example.com"))
    }

    @Test
    fun `tel is external`() {
        assertTrue(isExternalScheme("tel:+1234567890"))
    }

    @Test
    fun `TEL is matched case-insensitively`() {
        assertTrue(isExternalScheme("TEL:+1234567890"))
    }

    @Test
    fun `sms is external`() {
        assertTrue(isExternalScheme("sms:+1234567890"))
    }

    @Test
    fun `intent is external`() {
        assertTrue(isExternalScheme("intent:#Intent;package=com.example;end"))
    }

    @Test
    fun `market is external`() {
        assertTrue(isExternalScheme("market://details?id=com.example"))
    }

    @Test
    fun `geo is external`() {
        assertTrue(isExternalScheme("geo:0,0?q=coffee"))
    }

    @Test
    fun `whatsapp is external`() {
        assertTrue(isExternalScheme("whatsapp://send?text=hi"))
    }

    // ── Unsupported schemes (pass through to GeckoView) ───────────────────

    @Test
    fun `https is not external`() {
        assertFalse(isExternalScheme("https://example.com/page"))
    }

    @Test
    fun `http is not external`() {
        assertFalse(isExternalScheme("http://example.com/page"))
    }

    @Test
    fun `magnet is not external`() {
        assertFalse(isExternalScheme("magnet:?xt=urn:btih:abc123"))
    }

    @Test
    fun `about blank is not external`() {
        assertFalse(isExternalScheme("about:blank"))
    }

    @Test
    fun `file is not external`() {
        assertFalse(isExternalScheme("file:///path/to/file"))
    }

    @Test
    fun `custom scheme is not external`() {
        assertFalse(isExternalScheme("custom://example"))
    }

    @Test
    fun `bare string without scheme is not external`() {
        assertFalse(isExternalScheme("example"))
    }
}
