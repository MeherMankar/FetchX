package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class SessionNavigationActionsTest {
    @Test
    fun `back dispatches to GeckoSession only when available`() {
        var backCalls = 0
        val noHistory = TabState("tab")
        val withHistory = noHistory.copy(canGoBack = true)

        assertFalse(SessionNavigationActions.goBackIfAvailable(noHistory) { backCalls++ })
        assertEquals(0, backCalls)
        assertTrue(SessionNavigationActions.goBackIfAvailable(withHistory) { backCalls++ })
        assertEquals(1, backCalls)
    }

    @Test
    fun `forward dispatches to GeckoSession only when available`() {
        var forwardCalls = 0
        val noForwardHistory = TabState("tab")
        val withForwardHistory = noForwardHistory.copy(canGoForward = true)

        assertFalse(
            SessionNavigationActions.goForwardIfAvailable(noForwardHistory) { forwardCalls++ },
        )
        assertEquals(0, forwardCalls)
        assertTrue(
            SessionNavigationActions.goForwardIfAvailable(withForwardHistory) { forwardCalls++ },
        )
        assertEquals(1, forwardCalls)
    }
}
