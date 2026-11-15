package dev.fetchx.app.media

import android.content.Context

object MediaDownloadPreferences {
    private const val PREFS_NAME = "fetchx_media_download_preferences"
    private const val KEY_QUICK_DOWNLOAD = "quick_download"

    fun isQuickDownloadEnabled(context: Context): Boolean = preferences(context).getBoolean(KEY_QUICK_DOWNLOAD, true)

    fun setQuickDownloadEnabled(context: Context, enabled: Boolean) {
        preferences(context).edit().putBoolean(KEY_QUICK_DOWNLOAD, enabled).apply()
    }

    private fun preferences(context: Context) =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
}
