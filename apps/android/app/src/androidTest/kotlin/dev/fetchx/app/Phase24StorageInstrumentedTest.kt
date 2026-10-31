package dev.fetchx.app

import android.database.sqlite.SQLiteDatabase
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.fetchx.bridge.BridgeInitializer
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

@RunWith(AndroidJUnit4::class)
class Phase24StorageInstrumentedTest {

    @Test
    fun applicationMigratesAndReopensPrivateDatabaseWithPersistedRecord() {
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val database = File(context.filesDir, "fetchx.db")
            assertTrue(
                database.canonicalFile.toPath().startsWith(context.filesDir.canonicalFile.toPath()),
            )

            val repository = requireNotNull(BridgeInitializer.repositoryOrNull()) {
                "FetchX application did not initialize the Rust bridge"
            }
            repository.awaitInitialSnapshot()
            val item = repository.getDownload(FIXTURE_ID)
            assertNotNull("Migrated download record was not preserved", item)
            assertEquals("phase24-persisted.bin", item?.fileName)

            SQLiteDatabase.openDatabase(
                database.absolutePath,
                null,
                SQLiteDatabase.OPEN_READONLY,
            ).use { migrated ->
                assertEquals(
                    11L,
                    migrated.rawQuery("SELECT MAX(version) FROM _sqlx_migrations", null)
                        .use { cursor -> check(cursor.moveToFirst()); cursor.getLong(0) },
                )
                assertEquals(
                    "ok",
                    migrated.rawQuery("PRAGMA integrity_check", null)
                        .use { cursor -> check(cursor.moveToFirst()); cursor.getString(0) },
                )
            }
            assertTrue(
                context.filesDir.listFiles()
                    .orEmpty()
                    .any {
                        it.name.startsWith("fetchx.db.pre-migration-") && it.name.endsWith(".bak")
                    },
            )
            Log.i(TAG, "application reopened migrated private database with record $FIXTURE_ID")
        }
    }

    companion object {
        private const val TAG = "Phase24StorageTest"
        private const val FIXTURE_ID = "b647bcec-eafe-4efb-970a-5e1e6e03e6a1"
    }
}
