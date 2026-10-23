package dev.fetchx.downloads.storage

import android.content.Context
import android.net.Uri
import android.os.Environment
import androidx.core.content.edit
import java.io.File

/**
 * Manages the download storage location for FetchX.
 *
 * Default: app-specific external Downloads directory — no dangerous permissions required.
 * Custom: user-selected SAF directory with persisted URI permission.
 *
 * Security: all filenames are sanitized before use. Remote-supplied filenames are never
 * trusted directly. Path traversal sequences are stripped.
 */
class StorageManager(private val context: Context) {

    private val prefs by lazy {
        context.getSharedPreferences("fetchx_storage", Context.MODE_PRIVATE)
    }

    companion object {
        private const val KEY_CUSTOM_URI = "custom_download_uri"
        private const val MAX_FILENAME_LENGTH = 200
    }

    /**
     * Returns the effective download directory.
     * Uses custom SAF URI if set and permission is still valid; falls back to default.
     */
    fun resolveDownloadDirectoryPath(): String {
        val customUri = prefs.getString(KEY_CUSTOM_URI, null)
        if (customUri != null) {
            val uri = Uri.parse(customUri)
            val hasPerm = context.contentResolver.persistedUriPermissions
                .any { it.uri == uri && it.isWritePermission }
            if (hasPerm) return customUri
            // Permission revoked — clear stale pref
            prefs.edit { remove(KEY_CUSTOM_URI) }
        }
        return defaultDownloadDirectory().absolutePath
    }

    /** Returns /storage/emulated/0/Android/data/dev.fetchx.app.debug/files/Download, creating it if needed.
     *
     * Uses app-scoped external storage (getExternalFilesDir) which is always writable
     * without special permissions. Downloads are scanned into MediaStore on completion
     * so they appear in the public Downloads/FetchX folder in Files apps.
     */
    fun defaultDownloadDirectory(): File {
        val dir = context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)
            ?: context.filesDir.resolve("downloads")
        dir.mkdirs()
        return dir
    }

    /**
     * Returns a destination file confined to the FetchX download directory.
     * Throws if the resolved path escapes the base directory (path traversal protection).
     */
    fun confinedDestination(filename: String): File {
        val base = defaultDownloadDirectory().canonicalFile
        val target = File(base, filename).canonicalFile
        require(target.path.startsWith(base.path + File.separator) || target == base) {
            "Filename escapes the FetchX download directory: $filename"
        }
        return target
    }

    /** Stores a user-selected SAF URI and takes a persisted permission. */
    fun setCustomDirectory(uri: Uri) {
        context.contentResolver.takePersistableUriPermission(
            uri,
            android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION or
                android.content.Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
        )
        prefs.edit { putString(KEY_CUSTOM_URI, uri.toString()) }
    }

    /** Reverts to the default directory. */
    fun clearCustomDirectory() {
        prefs.getString(KEY_CUSTOM_URI, null)?.let { raw ->
            runCatching {
                context.contentResolver.releasePersistableUriPermission(
                    Uri.parse(raw),
                    android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION or
                        android.content.Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                )
            }
            prefs.edit { remove(KEY_CUSTOM_URI) }
        }
    }

    /**
     * Sanitizes a filename received from the network.
     * Strips path traversal, null bytes, reserved characters, and excessive length.
     * Never returns blank — falls back to "download".
     */
    fun sanitizeFileName(raw: String): String {
        if (raw.isBlank()) return "download"
        val stripped = raw.trim()
            .replace('/', '_').replace('\\', '_')
            .replace('\u0000', '_')
            .replace("..", "_")
            .trimStart('.')
            .replace(Regex("[<>:\"|?*]"), "_")
            .take(MAX_FILENAME_LENGTH)
        return stripped.ifBlank { "download" }
    }

    /**
     * Returns a non-colliding filename within [dir].
     * "file.zip" -> "file (1).zip" -> "file (2).zip" etc.
     */
    fun uniqueFileName(dir: File, desiredName: String): String {
        val safe = sanitizeFileName(desiredName)
        if (!File(dir, safe).exists()) return safe
        val dot = safe.lastIndexOf('.')
        val base = if (dot > 0) safe.substring(0, dot) else safe
        val ext  = if (dot > 0) safe.substring(dot) else ""
        var n = 1
        while (File(dir, "$base ($n)$ext").exists()) n++
        return "$base ($n)$ext"
    }
}
