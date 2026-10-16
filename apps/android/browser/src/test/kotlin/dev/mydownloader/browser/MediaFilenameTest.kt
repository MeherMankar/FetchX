package dev.fetchx.browser

import kotlin.test.Test
import kotlin.test.assertEquals

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
}
