package dev.fetchx.bridge

import android.content.ContentValues
import android.database.sqlite.SQLiteDatabase
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.fetchx.bridge.uniffi.CoreConfig
import dev.fetchx.bridge.uniffi.DownloadPriorityDto
import dev.fetchx.bridge.uniffi.DownloadService
import dev.fetchx.bridge.uniffi.DownloadStatusDto
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.security.MessageDigest
import java.util.UUID

@RunWith(AndroidJUnit4::class)
class StorageMigrationInstrumentedTest {

    @Test
    fun androidBridgeMigratesPrivateDatabaseAndPreservesDataAcrossReopen() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val root = File(context.filesDir, "phase24-${UUID.randomUUID()}").apply {
            check(mkdirs())
        }
        val downloads = File(root, "downloads").apply { check(mkdirs()) }
        val runtime = File(root, "runtime").apply { check(mkdirs()) }
        val databaseFile = File(root, "fetchx.db")
        val databasePath = databaseFile.canonicalFile
        assertTrue(databasePath.toPath().startsWith(context.filesDir.canonicalFile.toPath()))
        createVersionTenDatabase(databaseFile, downloads)

        System.loadLibrary("fetchx_android_bridge")
        val config = CoreConfig(
            databasePath = databaseFile.absolutePath,
            downloadsDir = downloads.absolutePath,
            runtimeDir = runtime.absolutePath,
            ytDlpExecutablePath = null,
        )
        val expectedId: String
        val service = DownloadService(config, null)
        try {
            val item = service.listDownloads().single()
            expectedId = item.id
            assertEquals(DownloadStatusDto.PAUSED, item.status)
            assertEquals("phase24-persisted.bin", item.filename)
        } finally {
            service.shutdown()
        }

        val migrationBackups = root.listFiles()
            .orEmpty()
            .filter { it.name.startsWith("fetchx.db.pre-migration-") && it.name.endsWith(".bak") }
        assertEquals(1, migrationBackups.size)
        SQLiteDatabase.openDatabase(
            migrationBackups.single().absolutePath,
            null,
            SQLiteDatabase.OPEN_READONLY,
        ).use { backup ->
            assertEquals(10L, backup.rawQuery("SELECT MAX(version) FROM _sqlx_migrations", null)
                .use { cursor -> check(cursor.moveToFirst()); cursor.getLong(0) })
            assertEquals("ok", backup.rawQuery("PRAGMA integrity_check", null)
                .use { cursor -> check(cursor.moveToFirst()); cursor.getString(0) })
        }
        SQLiteDatabase.openDatabase(
            databaseFile.absolutePath,
            null,
            SQLiteDatabase.OPEN_READONLY,
        ).use { migrated ->
            assertEquals(11L, migrated.rawQuery("SELECT MAX(version) FROM _sqlx_migrations", null)
                .use { cursor -> check(cursor.moveToFirst()); cursor.getLong(0) })
            assertEquals("ok", migrated.rawQuery("PRAGMA integrity_check", null)
                .use { cursor -> check(cursor.moveToFirst()); cursor.getString(0) })
        }

        val reopened = DownloadService(config, null)
        try {
            val restored = reopened.listDownloads().single()
            assertEquals(expectedId, restored.id)
            assertEquals(DownloadStatusDto.PAUSED, restored.status)
            Log.i(TAG, "migrated private database to v11 and reopened persisted download $expectedId")
        } finally {
            reopened.shutdown()
        }
    }

    private fun createVersionTenDatabase(databaseFile: File, downloads: File) {
        val database = SQLiteDatabase.openOrCreateDatabase(databaseFile, null)
        try {
            database.execSQL(
                """
                CREATE TABLE _sqlx_migrations (
                    version BIGINT PRIMARY KEY,
                    description TEXT NOT NULL,
                    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    success BOOLEAN NOT NULL,
                    checksum BLOB NOT NULL,
                    execution_time BIGINT NOT NULL
                )
                """.trimIndent(),
            )
            MIGRATIONS.take(10).forEachIndexed { index, migration ->
                val sql = InstrumentationRegistry.getInstrumentation().context.assets
                    .open(migration.file)
                    .bufferedReader()
                    .use { it.readText() }
                sql.lineSequence()
                    .filterNot { it.trimStart().startsWith("--") }
                    .joinToString("\n")
                    .split(';')
                    .map(String::trim)
                    .filter(String::isNotEmpty)
                    .forEach(database::execSQL)

                val values = ContentValues().apply {
                    put("version", (index + 1).toLong())
                    put("description", migration.description)
                    put("success", 1)
                    put("checksum", MessageDigest.getInstance("SHA-384")
                        .digest(sql.toByteArray(Charsets.UTF_8)))
                    put("execution_time", 0L)
                }
                database.insertOrThrow("_sqlx_migrations", null, values)
            }

            val createdAt = 1_725_000_000_123L
            val values = ContentValues().apply {
                put("id", FIXTURE_ID)
                put("source_url", "http://127.0.0.1:9/phase24-persisted.bin")
                put("engine_kind", "direct_http")
                put("filename", "phase24-persisted.bin")
                put("destination", File(downloads, "phase24-persisted.bin").absolutePath)
                put("status", "paused")
                put("priority", "normal")
                put("downloaded_bytes", 0L)
                put("created_at", createdAt)
                put("updated_at", createdAt)
                put("queue_order", 0L)
            }
            database.insertOrThrow("downloads", null, values)
        } finally {
            database.close()
        }
    }

    private data class Migration(val file: String, val description: String)

    companion object {
        private const val TAG = "Phase24StorageTest"
        private const val FIXTURE_ID = "b647bcec-eafe-4efb-970a-5e1e6e03e6a1"
        private val MIGRATIONS = listOf(
            Migration("0001_create_downloads.sql", "create_downloads"),
            Migration("0002_add_queue_order.sql", "add_queue_order"),
            Migration("0003_create_download_schedules.sql", "create_download_schedules"),
            Migration("0004_create_engine_config.sql", "create_engine_config"),
            Migration("0005_add_ytdlp_format.sql", "add_ytdlp_format"),
            Migration("0006_add_hls_engine_kind.sql", "add_hls_engine_kind"),
            Migration("0007_torrent_metadata.sql", "torrent_metadata"),
            Migration("0008_magnet_metadata.sql", "magnet_metadata"),
            Migration("0009_native_torrent_engine.sql", "native_torrent_engine"),
            Migration("0010_add_ftp_sftp_metalink_engines.sql", "add_ftp_sftp_metalink_engines"),
            Migration("0011_allow_merged_ytdlp_format_selectors.sql", "allow_merged_ytdlp_format_selectors"),
        )
    }
}
