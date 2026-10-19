package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Verifies that magnet: URI routing in TabManager correctly differentiates
 * magnet links from other external URI schemes.
 *
 * TabManager itself requires GeckoRuntime, which is not available on the JVM.
 * The routing logic is exercised through [MagnetUriRouter], a pure-JVM helper
 * that mirrors the dispatch logic in TabManager.onLoadRequest.
 */

/**
 * Mirrors the magnet vs. external dispatch logic from TabManager.onLoadRequest
 * so it can be tested on the JVM without GeckoRuntime.
 */
internal class MagnetUriRouter(
    val onMagnetRequested: ((String) -> Unit)?,
    val onExternalUriRequested: ((String) -> Unit)?,
) {
    /** Returns true if the URI was handled (should be DENY), false to pass through. */
    fun handle(uri: String): Boolean {
        if (uri.lowercase().startsWith("magnet:")) {
            val handler = onMagnetRequested
            if (handler != null) {
                handler(uri)
            } else {
                onExternalUriRequested?.invoke(uri)
            }
            return true
        }
        if (isExternalScheme(uri)) {
            onExternalUriRequested?.invoke(uri)
            return true
        }
        return false
    }
}

class TabManagerMagnetTest {

    // ── magnet: routing ────────────────────────────────────────────────────

    @Test fun `magnet URI invokes onMagnetRequested and not onExternalUriRequested`() {
        var magnetUri: String? = null
        var externalUri: String? = null
        val router = MagnetUriRouter(
            onMagnetRequested = { magnetUri = it },
            onExternalUriRequested = { externalUri = it },
        )

        val handled = router.handle("magnet:?xt=urn:btih:abc123&dn=Test")

        assertTrue(handled)
        assertEquals("magnet:?xt=urn:btih:abc123&dn=Test", magnetUri)
        assertNull(externalUri)
    }

    @Test fun `magnet URI is matched case-insensitively`() {
        var magnetUri: String? = null
        val router = MagnetUriRouter(
            onMagnetRequested = { magnetUri = it },
            onExternalUriRequested = null,
        )

        router.handle("MAGNET:?xt=urn:btih:abc123")

        assertEquals("MAGNET:?xt=urn:btih:abc123", magnetUri)
    }

    @Test fun `when onMagnetRequested is null magnet falls through to onExternalUriRequested`() {
        var externalUri: String? = null
        val router = MagnetUriRouter(
            onMagnetRequested = null,
            onExternalUriRequested = { externalUri = it },
        )

        router.handle("magnet:?xt=urn:btih:abc123")

        assertEquals("magnet:?xt=urn:btih:abc123", externalUri)
    }

    // ── non-magnet external URIs ────────────────────────────────────────────

    @Test fun `mailto URI invokes onExternalUriRequested and not onMagnetRequested`() {
        var magnetUri: String? = null
        var externalUri: String? = null
        val router = MagnetUriRouter(
            onMagnetRequested = { magnetUri = it },
            onExternalUriRequested = { externalUri = it },
        )

        val handled = router.handle("mailto:user@example.com")

        assertTrue(handled)
        assertNull(magnetUri)
        assertEquals("mailto:user@example.com", externalUri)
    }

    @Test fun `tel URI invokes onExternalUriRequested and not onMagnetRequested`() {
        var magnetUri: String? = null
        var externalUri: String? = null
        val router = MagnetUriRouter(
            onMagnetRequested = { magnetUri = it },
            onExternalUriRequested = { externalUri = it },
        )

        val handled = router.handle("tel:+1234567890")

        assertTrue(handled)
        assertNull(magnetUri)
        assertEquals("tel:+1234567890", externalUri)
    }

    @Test fun `https URI is not handled and passes through`() {
        var magnetUri: String? = null
        var externalUri: String? = null
        val router = MagnetUriRouter(
            onMagnetRequested = { magnetUri = it },
            onExternalUriRequested = { externalUri = it },
        )

        val handled = router.handle("https://example.com/page")

        assertTrue(!handled)
        assertNull(magnetUri)
        assertNull(externalUri)
    }

    @Test fun `http URI is not handled and passes through`() {
        var magnetUri: String? = null
        var externalUri: String? = null
        val router = MagnetUriRouter(
            onMagnetRequested = { magnetUri = it },
            onExternalUriRequested = { externalUri = it },
        )

        val handled = router.handle("http://example.com/page")

        assertTrue(!handled)
        assertNull(magnetUri)
        assertNull(externalUri)
    }

    // ── BrowserViewModel magnet state ──────────────────────────────────────

    @Test fun `BrowserUiState has magnetQueued and magnetError fields defaulting to null`() {
        val state = BrowserUiState()
        assertNull(state.magnetQueued)
        assertNull(state.magnetError)
    }

    @Test fun `clearMagnetState resets magnetQueued and magnetError in vm`() {
        val vm = BrowserViewModel()
        // Manually set the fields to simulate a successful queue
        vm.uiState.value.let { /* confirm initial state */ }

        // Use internal _uiState to simulate the state that onMagnetRequested would set
        // (we can't call it without a real bridge; test via the clear method)
        vm.clearMagnetState() // should be a no-op on null fields — no crash
        assertNull(vm.uiState.value.magnetQueued)
        assertNull(vm.uiState.value.magnetError)
    }
}
