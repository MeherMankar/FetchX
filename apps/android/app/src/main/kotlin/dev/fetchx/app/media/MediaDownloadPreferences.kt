package dev.fetchx.app.media

import android.content.Context

object MediaDownloadPreferences {
    private const val PREFS_NAME = "fetchx_media_download_preferences"
    private const val KEY_QUICK_DOWNLOAD = "quick_download"
    private const val KEY_SPONSORBLOCK = "sponsorblock"
    private const val KEY_SPONSORBLOCK_CATEGORIES = "sponsorblock_categories"
    private const val KEY_SUBTITLES = "download_subtitles"
    private const val KEY_SUBTITLE_LANGUAGES = "subtitle_languages"
    private const val KEY_EMBED_SUBTITLES = "embed_subtitles"
    private const val KEY_EMBED_METADATA = "embed_metadata"

    fun isQuickDownloadEnabled(context: Context): Boolean = preferences(context).getBoolean(KEY_QUICK_DOWNLOAD, true)

    fun setQuickDownloadEnabled(context: Context, enabled: Boolean) {
        preferences(context).edit().putBoolean(KEY_QUICK_DOWNLOAD, enabled).apply()
    }

    fun isSponsorBlockEnabled(context: Context): Boolean = preferences(context).getBoolean(KEY_SPONSORBLOCK, false)

    fun setSponsorBlockEnabled(context: Context, enabled: Boolean) {
        preferences(context).edit().putBoolean(KEY_SPONSORBLOCK, enabled).apply()
    }

    fun sponsorBlockCategories(context: Context): String =
        preferences(context).getString(KEY_SPONSORBLOCK_CATEGORIES, "sponsor,intro,outro,selfpromo")?.trim()
            .takeUnless { it.isNullOrBlank() } ?: "sponsor,intro,outro,selfpromo"

    fun areSubtitlesEnabled(context: Context): Boolean = preferences(context).getBoolean(KEY_SUBTITLES, false)

    fun setSubtitlesEnabled(context: Context, enabled: Boolean) {
        preferences(context).edit().putBoolean(KEY_SUBTITLES, enabled).apply()
    }

    fun subtitleLanguages(context: Context): String =
        preferences(context).getString(KEY_SUBTITLE_LANGUAGES, "en.*,*-orig")?.trim()
            .takeUnless { it.isNullOrBlank() } ?: "en.*,*-orig"

    fun areSubtitlesEmbedded(context: Context): Boolean =
        preferences(context).getBoolean(KEY_EMBED_SUBTITLES, false)

    fun setSubtitlesEmbedded(context: Context, enabled: Boolean) {
        preferences(context).edit().putBoolean(KEY_EMBED_SUBTITLES, enabled).apply()
    }

    fun isMetadataEmbeddingEnabled(context: Context): Boolean =
        preferences(context).getBoolean(KEY_EMBED_METADATA, true)

    fun setMetadataEmbeddingEnabled(context: Context, enabled: Boolean) {
        preferences(context).edit().putBoolean(KEY_EMBED_METADATA, enabled).apply()
    }

    fun applyArguments(context: Context, arguments: List<String>): List<String> {
        var delimiter = arguments.indexOf("--")
        if (delimiter < 0) return arguments

        val enhanced = arguments.toMutableList()
        fun addOption(option: String, vararg values: String) {
            if (option in enhanced) return
            val additions = listOf(option) + values.toList()
            enhanced.addAll(delimiter, additions)
            delimiter += additions.size
        }

        if (isSponsorBlockEnabled(context)) {
            addOption("--sponsorblock-remove", sponsorBlockCategories(context))
        }
        if (areSubtitlesEnabled(context)) {
            addOption("--write-sub")
            addOption("--write-auto-sub")
            if (!enhanced.contains("--sub-langs")) {
                enhanced.add(delimiter, "--sub-langs")
                enhanced.add(delimiter + 1, subtitleLanguages(context))
            }
            if (areSubtitlesEmbedded(context)) addOption("--embed-sub")
        }
        if (isMetadataEmbeddingEnabled(context)) addOption("--embed-metadata")

        return enhanced
    }

    private fun preferences(context: Context) =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
}
