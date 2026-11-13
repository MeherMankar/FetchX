package dev.fetchx.app

import org.junit.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class AndroidYtDlpRuntimeTest {
    @Test
    fun `does not require native media processor for default muxed selector`() {
        assertFalse(
            AndroidYtDlpRuntime.requiresNativeMediaProcessor(
                listOf("--format", "best"),
            ),
        )
    }

    @Test
    fun `requires native media processor for explicit format id merge selector`() {
        assertTrue(
            AndroidYtDlpRuntime.requiresNativeMediaProcessor(
                listOf("--format", "137+140"),
            ),
        )
    }

    @Test
    fun `does not require native media processor for a single format id`() {
        assertFalse(
            AndroidYtDlpRuntime.requiresNativeMediaProcessor(
                listOf("--format", "137"),
            ),
        )
    }

    @Test
    fun `requires native media processor for audio extraction`() {
        assertTrue(
            AndroidYtDlpRuntime.requiresNativeMediaProcessor(
                listOf("--format", "bestaudio/best", "--extract-audio", "--audio-format", "mp3"),
            ),
        )
    }
}
