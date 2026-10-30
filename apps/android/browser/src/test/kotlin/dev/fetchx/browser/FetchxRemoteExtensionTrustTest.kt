package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class FetchxRemoteExtensionTrustTest {
    @Test
    fun `only the built in FetchX extension id is trusted`() {
        assertTrue(isTrustedFetchxRemoteExtension("fetchx-remote@fetchx.dev"))
        assertFalse(isTrustedFetchxRemoteExtension("untrusted@fetchx.dev"))
        assertFalse(isTrustedFetchxRemoteExtension(""))
    }

    @Test
    fun `only the trusted extension can register and invoke the browser action`() {
        val invoker = FetchxRemoteActionInvoker()
        var clickCount = 0
        var popupCount = 0

        assertFalse(
            invoker.register(
                "untrusted@fetchx.dev",
                clickAction = { clickCount++ },
                openPopup = { popupCount++ },
            ),
        )
        assertFalse(invoker.invoke())

        assertTrue(
            invoker.register(
                "fetchx-remote@fetchx.dev",
                clickAction = { clickCount++ },
                openPopup = { popupCount++ },
            ),
        )
        assertTrue(invoker.invoke())
        assertEquals(1, clickCount)
        assertEquals(1, popupCount)

        invoker.clear()
        assertFalse(invoker.invoke())
    }

    @Test
    fun `native messaging is limited to the trusted extension and app name`() {
        assertTrue(isTrustedFetchxRemoteNativeMessage("fetchx-remote@fetchx.dev", "browser"))
        assertFalse(isTrustedFetchxRemoteNativeMessage("untrusted@fetchx.dev", "browser"))
        assertFalse(isTrustedFetchxRemoteNativeMessage("fetchx-remote@fetchx.dev", "other-app"))
    }
}
