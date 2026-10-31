package dev.fetchx.app

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.fetchx.bridge.BridgeInitializer
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class Phase25DiagnosticsInstrumentedTest {

    @Test
    fun snapshotIsAvailableAndDoesNotExposePrivateDatabasePath() {
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val repository = requireNotNull(BridgeInitializer.repositoryOrNull()) {
                "FetchX application did not initialize the Rust bridge"
            }
            repository.awaitInitialSnapshot()

            val snapshotJson = repository.diagnosticsSnapshot()
            val snapshot = JSONObject(snapshotJson)
            assertEquals(1, snapshot.getInt("schema_version"))
            assertTrue(snapshot.has("queue"))
            assertTrue(snapshot.has("engines"))
            assertEquals("healthy", snapshot.getJSONObject("storage").getString("health"))
            assertFalse(snapshotJson.contains(context.filesDir.canonicalPath))
        }
    }
}
