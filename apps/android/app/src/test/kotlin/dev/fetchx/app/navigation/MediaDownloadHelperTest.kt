package dev.fetchx.app.navigation

import org.junit.Test
import kotlin.test.assertEquals

class MediaDownloadHelperTest {

    @Test
    fun canonicalizeMobileYouTubeWatchUrl() {
        assertEquals(
            "https://www.youtube.com/watch?v=RcDxmQmHuJw",
            canonicalizeMediaUrl("https://m.youtube.com/watch?v=RcDxmQmHuJw"),
        )
    }

    @Test
    fun canonicalizeMobileYouTubeWatchUrlWithExtraParams() {
        assertEquals(
            "https://www.youtube.com/watch?v=RcDxmQmHuJw",
            canonicalizeMediaUrl("https://m.youtube.com/watch?feature=share&v=RcDxmQmHuJw&si=123"),
        )
    }

    @Test
    fun preservesDesktopYouTubeWatchUrl() {
        assertEquals(
            "https://www.youtube.com/watch?v=RcDxmQmHuJw",
            canonicalizeMediaUrl("https://www.youtube.com/watch?v=RcDxmQmHuJw"),
        )
    }

    @Test
    fun preservesShortYouTubeUrl() {
        assertEquals(
            "https://youtu.be/RcDxmQmHuJw",
            canonicalizeMediaUrl("https://youtu.be/RcDxmQmHuJw"),
        )
    }

    @Test
    fun preservesNonYouTubeUrl() {
        assertEquals(
            "https://example.com/video.mp4",
            canonicalizeMediaUrl("https://example.com/video.mp4"),
        )
    }

    @Test
    fun sanitizeMediaFileNameAppendsExtensionIfMissing() {
        assertEquals("Sample Video.mp4", sanitizeMediaFileName("Sample Video"))
    }

    @Test
    fun sanitizeMediaFileNamePreservesExistingMp4Extension() {
        assertEquals("Sample Video.mp4", sanitizeMediaFileName("Sample Video.mp4"))
        assertEquals("Sample Video.MP4", sanitizeMediaFileName("Sample Video.MP4"))
    }

    @Test
    fun sanitizeMediaFileNameHandlesBlankTitle() {
        assertEquals("Media download.mp4", sanitizeMediaFileName(""))
        assertEquals("Media download.mp4", sanitizeMediaFileName("   "))
    }
}
