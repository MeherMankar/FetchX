package dev.fetchx.app

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.fetchx.app.media.MediaDownloadPreferences
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MediaDownloadPreferencesInstrumentedTest {
    @Test
    fun metadataEmbeddingIsOptInForAndroidMediaDownloads() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val preferences = context.getSharedPreferences(
            "fetchx_media_download_preferences",
            android.content.Context.MODE_PRIVATE,
        )
        val hadStoredPreference = preferences.contains("embed_metadata")
        val previousPreference = preferences.getBoolean("embed_metadata", false)
        try {
            preferences.edit().remove("embed_metadata").commit()
            val arguments = listOf(
                "--format",
                "bestvideo+bestaudio",
                "--",
                "https://example.test/video",
            )

            assertFalse(MediaDownloadPreferences.isMetadataEmbeddingEnabled(context))
            assertFalse(
                MediaDownloadPreferences.applyArguments(context, arguments)
                    .contains("--embed-metadata"),
            )

            MediaDownloadPreferences.setMetadataEmbeddingEnabled(context, true)
            assertTrue(
                MediaDownloadPreferences.applyArguments(context, arguments)
                    .contains("--embed-metadata"),
            )
        } finally {
            val editor = preferences.edit()
            if (hadStoredPreference) {
                editor.putBoolean("embed_metadata", previousPreference)
            } else {
                editor.remove("embed_metadata")
            }
            editor.commit()
        }
    }
}
