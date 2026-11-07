package dev.fetchx.app

import dev.fetchx.app.extractSharedUrl
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ExtractSharedUrlTest {
    @Test
    fun extractsFirstHttpUrlFromSharedText() {
        assertEquals(
            "https://example.com/video",
            extractSharedUrl("Watch this https://example.com/video now"),
        )
        assertNull(extractSharedUrl("No URL here"))
    }
}
