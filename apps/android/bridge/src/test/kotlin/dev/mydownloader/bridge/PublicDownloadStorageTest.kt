package dev.fetchx.bridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import dev.fetchx.core.download.DownloadState
import java.io.File

class PublicDownloadStorageTest {

    @Test
    fun `public folder uses Downloads relative path`() {
        assertEquals("Download/FetchX/", publicRelativeDirectoryPath("Download"))
    }

    @Test
    fun `default public directory resolves under public Downloads`() {
        val downloadsRoot = File(File.separator + "storage", "emulated/0/Download")
        val actual = publicFetchXDirectory(downloadsRoot).absolutePath
        assertEquals(File(downloadsRoot, "FetchX").absolutePath, actual)
        assertTrue(actual.endsWith("Download${File.separator}FetchX"))
        assertFalse(actual.contains("${File.separator}Android${File.separator}data${File.separator}"))
        assertFalse(actual.contains("${File.separator}data${File.separator}data${File.separator}"))
    }

    @Test
    fun `filename sanitization prevents path and directory injection`() {
        val safe = sanitizePublicFileName("../../folder\\file?.zip")
        assertFalse(safe.contains('/'))
        assertFalse(safe.contains('\\'))
        assertFalse(safe.contains(".."))
        assertEquals("____folder_file_.zip", safe)
    }

    @Test
    fun `duplicate filename uses stable numbered suffix`() {
        val existing = setOf("archive.zip", "archive (1).zip")
        val result = uniqueAvailablePublicFileName("archive.zip") { it in existing }
        assertEquals("archive (2).zip", result)
    }

    @Test
    fun `filename without extension receives suffix`() {
        val result = uniqueAvailablePublicFileName("download") { it == "download" }
        assertEquals("download (1)", result)
        assertTrue(result.startsWith("download"))
    }

    @Test
    fun `filename reserved by an active download receives a suffix`() {
        val result = uniqueAvailablePublicFileName(
            desiredName = "archive.zip",
            reservedFileNames = setOf("archive.zip", "archive (1).zip"),
        ) { false }

        assertEquals("archive (2).zip", result)
    }

    @Test
    fun `reservation decoder remains compatible with Phase 20Z2 journal values`() {
        val digest = "0123456789abcdef".repeat(4)
        val legacy = """{"name":"archive.zip","size":24,"sha256":"$digest"}"""

        assertEquals(
            PublicDownloadReservation("archive.zip", 24, digest),
            decodePublicDownloadReservation(legacy),
        )
        assertEquals(
            PublicDownloadReservation("archive.zip", 24, digest),
            decodePublicDownloadReservation(
                encodePublicDownloadReservation(PublicDownloadReservation("archive.zip", 24, digest)),
            ),
        )
    }

    @Test
    fun `staged completion remains pending until public publication succeeds`() {
        val pending = completedDownloadState(24, "/storage/emulated/0/Download/FetchX/file.txt", true, false)
        assertTrue(pending is DownloadState.PublicationPending)
        val published = completedDownloadState(24, "content://media/downloads/42", true, true)
        assertEquals(DownloadState.Completed(24, "content://media/downloads/42"), published)
        val custom = completedDownloadState(24, "/custom/file.txt", false, false)
        assertEquals(DownloadState.Completed(24, "/custom/file.txt"), custom)
    }
}
