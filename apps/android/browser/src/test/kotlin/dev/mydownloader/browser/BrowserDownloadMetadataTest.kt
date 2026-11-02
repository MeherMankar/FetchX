package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class BrowserDownloadMetadataTest {

    @Test
    fun `reads response metadata without depending on header casing`() {
        assertEquals(
            BrowserDownloadMetadata("application/pdf", 12_800L),
            browserDownloadMetadata(
                mapOf(
                    "Content-Type" to "application/pdf; charset=binary",
                    "CONTENT-LENGTH" to "12800",
                ),
            ),
        )
    }

    @Test
    fun `ignores invalid and negative content lengths`() {
        assertEquals(
            BrowserDownloadMetadata(null, null),
            browserDownloadMetadata(
                mapOf(
                    "content-type" to " ",
                    "content-length" to "-1",
                ),
            ),
        )
        assertEquals(
            BrowserDownloadMetadata(null, null),
            browserDownloadMetadata(mapOf("content-length" to "unknown")),
        )
    }

    @Test
    fun `metadata label only includes available type and size`() {
        assertEquals(
            "application/pdf · 12.5 KB",
            browserDownloadMetadataLabel("report.pdf", "application/pdf", 12_800L),
        )
        assertEquals("MP4", browserDownloadMetadataLabel("video.mp4", null, null))
        assertEquals("0 B", browserDownloadMetadataLabel(null, null, 0L))
        assertNull(browserDownloadMetadataLabel(null, null, null))
    }
}
