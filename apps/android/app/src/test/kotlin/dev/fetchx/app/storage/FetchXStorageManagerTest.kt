package dev.fetchx.app.storage

import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.nio.file.Files
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class FetchXStorageManagerTest {
    @get:Rule
    val temporaryFolder = TemporaryFolder()

    @Test
    fun `storage totals separate downloads profile cache workspaces and database`() {
        val root = temporaryFolder.root.toPath()
        val files = Files.createDirectories(root.resolve("files"))
        val downloads = Files.createDirectories(root.resolve("external"))
        val cache = Files.createDirectories(root.resolve("cache"))
        val profile = Files.createDirectories(files.resolve("mozilla/profile"))
        val workspace = Files.createDirectories(downloads.resolve(".fetchx-ytdlp-active"))
        val runtime = Files.createDirectories(files.resolve("runtime/ytdlp-run-active"))
        val database = files.resolve("fetchx.db")

        writeBytes(downloads.resolve("complete.bin"), 13)
        writeBytes(downloads.resolve("active.part"), 5)
        writeBytes(workspace.resolve("fragment.part"), 7)
        writeBytes(downloads.resolve(".fetchx-ffmpeg-active/output.tmp"), 11)
        writeBytes(runtime.resolve("progress"), 29)
        writeBytes(profile.resolve("prefs.js"), 11)
        writeBytes(profile.resolve("cache2/entries/cache"), 19)
        writeBytes(cache.resolve("chaquopy-cache"), 23)
        writeBytes(database, 2)
        writeBytes(files.resolve("fetchx.db-wal"), 3)
        writeBytes(files.resolve("fetchx.db-shm"), 4)
        writeBytes(files.resolve("other.bin"), 17)

        val usage = calculateStorageUsage(
            StorageLocations(
                filesDir = files,
                downloadsDir = downloads,
                cacheDirs = listOf(cache),
                databaseFile = database,
                customSafLocationConfigured = true,
            ),
        )

        assertEquals(18, usage.downloadsBytes)
        assertEquals(11, usage.browserDataBytes)
        assertEquals(42, usage.browserCacheBytes)
        assertEquals(47, usage.temporaryBytes)
        assertEquals(9, usage.databaseBytes)
        assertEquals(17, usage.otherAppDataBytes)
        assertEquals(144, usage.totalBytes)
        assertTrue(usage.customSafLocationConfigured)
    }

    @Test
    fun `missing directories have zero size and no custom saf location`() {
        val root = temporaryFolder.root.toPath()
        val files = root.resolve("files")
        val usage = calculateStorageUsage(
            StorageLocations(
                filesDir = files,
                downloadsDir = root.resolve("external"),
                cacheDirs = emptyList(),
                databaseFile = files.resolve("fetchx.db"),
                customSafLocationConfigured = false,
            ),
        )

        assertEquals(0, usage.totalBytes)
        assertFalse(usage.customSafLocationConfigured)
    }

    private fun writeBytes(path: java.nio.file.Path, count: Int) {
        Files.createDirectories(path.parent)
        Files.write(path, ByteArray(count))
    }
}
