package dev.fetchx.downloads.notification

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class NotificationFormattingTest {

    @Test fun `format speed bytes`() {
        assertEquals("512 B/s", formatDownloadSpeed(512))
    }

    @Test fun `format speed kilobytes`() {
        assertTrue(formatDownloadSpeed(1024).contains("KB/s"))
    }

    @Test fun `format speed megabytes`() {
        assertTrue(formatDownloadSpeed(1_048_576).contains("MB/s"))
    }

    @Test fun `format eta seconds`() {
        assertEquals("30s", formatEta(30))
    }

    @Test fun `format eta minutes`() {
        assertEquals("2m", formatEta(120))
    }

    @Test fun `format eta hours`() {
        assertTrue(formatEta(7200).contains("h"))
    }

    @Test fun `format bytes`() {
        assertEquals("512 B", formatBytes(512))
        assertTrue(formatBytes(1024).contains("KB"))
        assertTrue(formatBytes(1_048_576).contains("MB"))
        assertTrue(formatBytes(1_073_741_824).contains("GB"))
    }
}
