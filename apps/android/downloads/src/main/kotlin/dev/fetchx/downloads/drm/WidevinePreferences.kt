package dev.fetchx.downloads.drm

import android.content.Context

/**
 * Persists Widevine-related user settings using [android.content.SharedPreferences].
 *
 * Currently stores the filesystem path to a user-supplied `.wvd` device file.
 * When a path is present, [WidevineManager] can use it as a fallback device
 * identity instead of the platform CDM.
 */
object WidevinePreferences {

    private const val PREFS_NAME = "fetchx_widevine_preferences"
    private const val KEY_WVD_FILE_PATH = "wvd_file_path"

    /**
     * Returns the user-configured `.wvd` file path, or null if none is set.
     */
    fun getWvdFilePath(context: Context): String? =
        preferences(context).getString(KEY_WVD_FILE_PATH, null)
            ?.takeIf { it.isNotBlank() }

    /**
     * Stores the path to the user's `.wvd` device file.
     *
     * Pass null or a blank string to clear the setting.
     */
    fun setWvdFilePath(context: Context, path: String?) {
        val prefs = preferences(context).edit()
        if (path.isNullOrBlank()) {
            prefs.remove(KEY_WVD_FILE_PATH)
        } else {
            prefs.putString(KEY_WVD_FILE_PATH, path.trim())
        }
        prefs.apply()
    }

    private fun preferences(context: Context) =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
}
