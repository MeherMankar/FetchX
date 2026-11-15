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

    @Test
    fun compatibilityRuntimeIsRequiredOnlyForLegacyMediaOptions() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val preferences = context.getSharedPreferences(
            "fetchx_media_download_preferences",
            android.content.Context.MODE_PRIVATE,
        )
        val keys = listOf("sponsorblock", "download_subtitles", "embed_metadata")
        val previous = keys.associateWith { key ->
            preferences.contains(key) to preferences.getBoolean(key, false)
        }
        try {
            val editor = preferences.edit()
            keys.forEach { editor.remove(it) }
            editor.commit()
            assertFalse(MediaDownloadPreferences.requiresCompatibilityRuntime(context))

            MediaDownloadPreferences.setSponsorBlockEnabled(context, true)
            assertTrue(MediaDownloadPreferences.requiresCompatibilityRuntime(context))
            MediaDownloadPreferences.setSponsorBlockEnabled(context, false)

            MediaDownloadPreferences.setSubtitlesEnabled(context, true)
            assertTrue(MediaDownloadPreferences.requiresCompatibilityRuntime(context))
            MediaDownloadPreferences.setSubtitlesEnabled(context, false)

            MediaDownloadPreferences.setMetadataEmbeddingEnabled(context, true)
            assertTrue(MediaDownloadPreferences.requiresCompatibilityRuntime(context))
        } finally {
            val editor = preferences.edit()
            previous.forEach { (key, value) ->
                if (value.first) {
                    editor.putBoolean(key, value.second)
                } else {
                    editor.remove(key)
                }
            }
            editor.commit()
        }
    }
}
