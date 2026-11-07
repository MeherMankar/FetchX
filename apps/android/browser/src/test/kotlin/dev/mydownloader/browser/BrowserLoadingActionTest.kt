package dev.fetchx.browser

import org.junit.Assert.assertEquals
import org.junit.Test

class BrowserLoadingActionTest {
    @Test
    fun `loading action uses stop semantics`() {
        assertEquals("Stop", browserLoadingActionLabel(isLoading = true))
        assertEquals("Stop loading", browserLoadingActionDescription(isLoading = true))
    }

    @Test
    fun `idle action uses reload semantics`() {
        assertEquals("Reload", browserLoadingActionLabel(isLoading = false))
        assertEquals("Reload", browserLoadingActionDescription(isLoading = false))
    }
}
