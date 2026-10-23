package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class MediaFilenameTest {
    @Test
    fun `builds title quality platform filename`() {
        assertEquals(
            "A video title - 720p - XHamster.mp4",
            buildMediaFileName(
                title = "A video title",
                quality = "720p + audio - mp4 - 1.0 GB",
                platform = "XHamster",
                extension = "mp4",
            ),
        )
    }

    @Test
    fun `preserves periods and strips unsafe filename characters`() {
        assertEquals(
            "Part 1. Episode 2 - Best - Example.mp4",
            buildMediaFileName(
                title = "Part 1. Episode 2",
                quality = null,
                platform = "Example/../",
                extension = "mp4",
            ),
        )
    }

    @Test
    fun `uses safe defaults when metadata is unavailable`() {
        assertEquals(
            "video - Best - Web.mp4",
            buildMediaFileName("", null, null, null),
        )
    }

    @Test
    fun `direct media detection uses URL path and ignores query and fragment`() {
        assertTrue(BrowserViewModel.isDirectMediaUrl("https://cdn.example.test/video.mp4?token=abc#t=10"))
        assertTrue(BrowserViewModel.isDirectMediaUrl("https://cdn.example.test/stream.m3u8#fragment"))
        assertFalse(BrowserViewModel.isDirectMediaUrl("https://video.mp4/"))
        assertFalse(BrowserViewModel.isDirectMediaUrl("https://example.test/watch?format=video.mp4"))
        assertFalse(BrowserViewModel.isDirectMediaUrl("https://example.test/watch#video.mp4"))
    }
}
