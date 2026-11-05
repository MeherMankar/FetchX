package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class RestoredTabLocationGuardTest {
    @Test
    fun `initial blank location does not replace a restored URL`() {
        val guard = RestoredTabLocationGuard()
        guard.track("restored", "https://example.test/page")

        assertTrue(guard.shouldIgnore("restored", "about:blank", hasUserGesture = false))
        assertFalse(guard.shouldIgnore("restored", "https://example.test/page", hasUserGesture = false))
        assertFalse(guard.shouldIgnore("restored", "about:blank", hasUserGesture = false))
    }

    @Test
    fun `multiple restored tabs have independent guards`() {
        val guard = RestoredTabLocationGuard()
        guard.track("first", "https://first.example/page")
        guard.track("second", "https://second.example/page")

        assertTrue(guard.shouldIgnore("first", "about:blank", hasUserGesture = false))
        assertFalse(guard.shouldIgnore("first", "https://first.example/page", hasUserGesture = false))
        assertFalse(guard.shouldIgnore("first", "about:blank", hasUserGesture = false))
        assertTrue(guard.shouldIgnore("second", "about:blank", hasUserGesture = false))
        assertFalse(guard.shouldIgnore("second", "https://second.example/page", hasUserGesture = false))
    }

    @Test
    fun `later blank location is accepted after restored URL callback`() {
        val guard = RestoredTabLocationGuard()
        guard.track("restored", "https://example.test/page")

        assertFalse(guard.shouldIgnore("restored", "https://example.test/page", hasUserGesture = false))
        assertFalse(guard.shouldIgnore("restored", "about:blank", hasUserGesture = false))
    }

    @Test
    fun `user initiated blank navigation is not suppressed while restore is pending`() {
        val guard = RestoredTabLocationGuard()
        guard.track("restored", "https://example.test/page")

        assertFalse(guard.shouldIgnore("restored", "about:blank", hasUserGesture = true))
        assertFalse(guard.shouldIgnore("restored", "https://example.test/page", hasUserGesture = false))
    }

    @Test
    fun `explicit navigation clears the initial restore guard`() {
        val guard = RestoredTabLocationGuard()
        guard.track("restored", "https://example.test/page")
        guard.clear("restored")

        assertFalse(guard.shouldIgnore("restored", "about:blank", hasUserGesture = false))
    }

    @Test
    fun `blank restored tabs are not guarded`() {
        val guard = RestoredTabLocationGuard()
        guard.track("blank", "about:blank")

        assertFalse(guard.shouldIgnore("blank", "about:blank", hasUserGesture = false))
    }

    @Test
    fun `closing a tab or disposing the manager clears its guard`() {
        val guard = RestoredTabLocationGuard()
        guard.track("closed", "https://closed.example")
        guard.track("disposed", "https://disposed.example")

        guard.clear("closed")
        assertFalse(guard.shouldIgnore("closed", "about:blank", hasUserGesture = false))
        assertTrue(guard.shouldIgnore("disposed", "about:blank", hasUserGesture = false))

        guard.clear()
        assertFalse(guard.shouldIgnore("disposed", "about:blank", hasUserGesture = false))
    }

    @Test
    fun `terminal load failure releases the pending restore guard`() {
        val guard = RestoredTabLocationGuard()
        guard.track("failed", "https://failed.example")

        guard.clear("failed")

        assertFalse(guard.shouldIgnore("failed", "about:blank", hasUserGesture = false))
    }
}
