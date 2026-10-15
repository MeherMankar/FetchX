package dev.fetchx.browser

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class VideoInterceptorTest {

    // ── isVideoMimeType ───────────────────────────────────────────────────

    @Test
    fun `isVideoMimeType returns true for video-mp4`() {
        assertTrue(VideoInterceptor.isVideoMimeType("video/mp4"))
    }

    @Test
    fun `isVideoMimeType returns true for video-webm`() {
        assertTrue(VideoInterceptor.isVideoMimeType("video/webm"))
    }

    @Test
    fun `isVideoMimeType returns true with charset parameter`() {
        assertTrue(VideoInterceptor.isVideoMimeType("video/mp4; charset=utf-8"))
    }

    @Test
    fun `isVideoMimeType returns false for application-octet-stream`() {
        assertFalse(VideoInterceptor.isVideoMimeType("application/octet-stream"))
    }

    @Test
    fun `isVideoMimeType returns false for null`() {
        assertFalse(VideoInterceptor.isVideoMimeType(null))
    }

    @Test
    fun `isVideoMimeType returns false for text-html`() {
        assertFalse(VideoInterceptor.isVideoMimeType("text/html"))
    }

    // ── isVideoUrl ────────────────────────────────────────────────────────

    @Test
    fun `isVideoUrl returns true for mp4 extension`() {
        assertTrue(VideoInterceptor.isVideoUrl("https://example.com/clip.mp4"))
    }

    @Test
    fun `isVideoUrl returns true for m3u8 extension`() {
        assertTrue(VideoInterceptor.isVideoUrl("https://cdn.example.com/stream.m3u8"))
    }

    @Test
    fun `isVideoUrl returns true for googlevideo videoplayback`() {
        assertTrue(
            VideoInterceptor.isVideoUrl(
                "https://r4---sn-abc.googlevideo.com/videoplayback?expire=12345",
            ),
        )
    }

    @Test
    fun `isVideoUrl returns true for extension before query string`() {
        assertTrue(VideoInterceptor.isVideoUrl("https://example.com/video.mp4?token=abc"))
    }

    @Test
    fun `isVideoUrl returns false for html page`() {
        assertFalse(VideoInterceptor.isVideoUrl("https://example.com/page.html"))
    }

    @Test
    fun `isVideoUrl returns false for image`() {
        assertFalse(VideoInterceptor.isVideoUrl("https://example.com/image.png"))
    }

    // ── suggestFilename ───────────────────────────────────────────────────

    @Test
    fun `suggestFilename keeps mp4 extension`() {
        assertEquals("video.mp4", VideoInterceptor.suggestFilename("https://cdn.com/video.mp4", "video/mp4"))
    }

    @Test
    fun `suggestFilename appends mp4 when no video extension`() {
        assertEquals(
            "videoplayback.mp4",
            VideoInterceptor.suggestFilename("https://cdn.com/videoplayback", "video/webm"),
        )
    }

    @Test
    fun `suggestFilename uses fallback name for empty path segment`() {
        val filename = VideoInterceptor.suggestFilename("https://cdn.com/", "video/mp4")
        assertTrue(filename.endsWith(".mp4"))
    }

    @Test
    fun `suggestFilename strips query string before extracting segment`() {
        assertEquals(
            "clip.mkv",
            VideoInterceptor.suggestFilename("https://cdn.com/clip.mkv?token=abc", "video/x-matroska"),
        )
    }

    // ── detect ────────────────────────────────────────────────────────────

    @Test
    fun `detect returns DetectedVideo for video MIME type`() {
        val result = VideoInterceptor.detect(
            tabId = "tab-1",
            url = "https://cdn.example.com/stream",
            mimeType = "video/mp4",
            contentLength = 1024L,
        )
        assertNotNull(result)
        assertEquals("tab-1", result!!.tabId)
        assertEquals("video/mp4", result.mimeType)
        assertEquals(1024L, result.contentLength)
    }

    @Test
    fun `detect returns DetectedVideo for video URL even with non-video MIME`() {
        val result = VideoInterceptor.detect(
            tabId = "tab-2",
            url = "https://cdn.example.com/clip.mp4",
            mimeType = null,
            contentLength = null,
        )
        assertNotNull(result)
    }

    @Test
    fun `detect returns null for non-video MIME and non-video URL`() {
        val result = VideoInterceptor.detect(
            tabId = "tab-3",
            url = "https://example.com/page.html",
            mimeType = "text/html",
            contentLength = null,
        )
        assertNull(result)
    }

    @Test
    fun `detect returns null for null MIME and non-video URL`() {
        val result = VideoInterceptor.detect(
            tabId = "tab-4",
            url = "https://example.com/image.png",
            mimeType = null,
            contentLength = null,
        )
        assertNull(result)
    }

    @Test
    fun `detect populates suggestedFilename`() {
        val result = VideoInterceptor.detect(
            tabId = "tab-5",
            url = "https://cdn.example.com/movie.mp4",
            mimeType = "video/mp4",
            contentLength = null,
        )
        assertEquals("movie.mp4", result?.suggestedFilename)
    }
}
