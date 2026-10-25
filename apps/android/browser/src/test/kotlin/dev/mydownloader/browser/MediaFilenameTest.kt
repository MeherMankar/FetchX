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

    @Test
    fun `automatic selection after format discovery uses page url without a manual selector`() {
        val route = BrowserViewModel.resolveMediaDownloadRoute(
            videoUrl = "https://rr.example.test/videoplayback?token=abc",
            pageUrl = "https://www.youtube.com/watch?v=example",
            formatId = null,
            hasDiscoveredFormats = true,
        )

        assertEquals("https://www.youtube.com/watch?v=example", route.url)
        assertFalse(route.isDirectMedia)
    }

    @Test
    fun `automatic selection without discovered formats keeps the stream url`() {
        val route = BrowserViewModel.resolveMediaDownloadRoute(
            videoUrl = "https://cdn.example.test/videoplayback?token=abc",
            pageUrl = "https://example.test/watch?v=example",
            formatId = null,
            hasDiscoveredFormats = false,
        )

        assertEquals("https://cdn.example.test/videoplayback?token=abc", route.url)
        assertFalse(route.isDirectMedia)
    }

    @Test
    fun `manual format selection uses page url`() {
        val route = BrowserViewModel.resolveMediaDownloadRoute(
            videoUrl = "https://rr.example.test/videoplayback?token=abc",
            pageUrl = "https://www.youtube.com/watch?v=example",
            formatId = "137+bestaudio/137",
            hasDiscoveredFormats = true,
        )

        assertEquals("https://www.youtube.com/watch?v=example", route.url)
        assertFalse(route.isDirectMedia)
    }

    @Test
    fun `direct media selection keeps using the direct url`() {
        val route = BrowserViewModel.resolveMediaDownloadRoute(
            videoUrl = "https://cdn.example.test/video.mp4?token=abc",
            pageUrl = "https://example.test/watch?v=example",
            formatId = null,
            hasDiscoveredFormats = false,
        )

        assertEquals("https://cdn.example.test/video.mp4?token=abc", route.url)
        assertTrue(route.isDirectMedia)
    }
}
