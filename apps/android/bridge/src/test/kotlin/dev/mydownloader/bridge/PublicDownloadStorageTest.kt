package dev.fetchx.bridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import dev.fetchx.core.download.DownloadState

class PublicDownloadStorageTest {

    @Test
    fun `public folder uses Downloads relative path`() {
        assertEquals("Download/FetchX/", publicRelativeDirectoryPath("Download"))
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
        val result = uniquePublicFileName("archive.zip") { it in existing }
        assertEquals("archive (2).zip", result)
    }

    @Test
    fun `filename without extension receives suffix`() {
        val result = uniquePublicFileName("download") { it == "download" }
        assertEquals("download (1)", result)
        assertTrue(result.startsWith("download"))
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
