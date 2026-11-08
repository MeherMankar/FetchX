package dev.fetchx.app

import org.junit.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class AndroidYtDlpRuntimeTest {
    @Test
    fun `detects default video and audio merge selector`() {
        assertTrue(
            AndroidYtDlpRuntime.requiresFfmpeg(
                listOf("--format", "bestvideo*+bestaudio/best"),
            ),
        )
    }

    @Test
    fun `detects explicit format id merge selector`() {
        assertTrue(
            AndroidYtDlpRuntime.requiresFfmpeg(
                listOf("--format", "137+140"),
            ),
        )
    }

    @Test
    fun `does not require ffmpeg for a single format id`() {
        assertFalse(
            AndroidYtDlpRuntime.requiresFfmpeg(
                listOf("--format", "137"),
            ),
        )
    }

    @Test
    fun `requires ffmpeg for audio extraction`() {
        assertTrue(
            AndroidYtDlpRuntime.requiresFfmpeg(
                listOf("--format", "bestaudio/best", "--extract-audio", "--audio-format", "mp3"),
            ),
        )
    }
}
