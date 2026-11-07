package dev.fetchx.app

import kotlin.test.assertEquals
import kotlin.test.assertNull
import org.junit.Test

class SharedLinkExtractorTest {
    @Test
    fun extractsFirstUrlAndDecodesAmpersands() {
        assertEquals(
            "https://example.com/watch?v=one&list=two",
            extractSharedUrl("Watch this: https://example.com/watch?v=one&amp;list=two"),
        )
    }

    @Test
    fun trimsCommonTrailingPunctuation() {
        assertEquals(
            "https://example.com/video.mp4",
            extractSharedUrl("https://example.com/video.mp4)."),
        )
    }

    @Test
    fun ignoresTextWithoutHttpUrl() {
        assertNull(extractSharedUrl("A video shared without a link"))
    }
}
