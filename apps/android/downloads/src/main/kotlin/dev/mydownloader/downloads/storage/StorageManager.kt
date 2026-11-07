package dev.fetchx.downloads.storage

import android.content.Context
import android.net.Uri
import android.os.Environment
import androidx.core.content.edit
import dev.fetchx.bridge.BridgeInitializer
import java.io.File

/**
 * Manages the download storage location for FetchX.
 *
 * Default: the public FetchX directory under the system Downloads folder.
 * Custom: user-selected SAF directory with persisted URI permission.
 *
 * Security: all filenames are sanitized before use. Remote-supplied filenames are never
 * trusted directly. Path traversal sequences are stripped.
 */
internal fun defaultPublicFetchXDirectory(baseDirectory: File = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)): File {
    val dir = File(baseDirectory, "FetchX")
    if (!dir.isDirectory && !dir.mkdirs() && !dir.isDirectory) {
        throw IllegalStateException("Could not create the public FetchX download directory.")
    }
    return dir
}

class StorageManager(private val context: Context) {

    private val prefs by lazy {
        context.getSharedPreferences("fetchx_storage", Context.MODE_PRIVATE)
    }

    companion object {
        private const val KEY_CUSTOM_URI = "custom_download_uri"
        private const val KEY_SMART_FOLDERS = "smart_folders"
        private const val MAX_FILENAME_LENGTH = 200
    }

    fun isSmartFoldersEnabled(): Boolean =
        prefs.getBoolean(KEY_SMART_FOLDERS, false)

    fun setSmartFoldersEnabled(enabled: Boolean) {
        prefs.edit { putBoolean(KEY_SMART_FOLDERS, enabled) }
    }

    fun resolveSmartDownloadDirectory(baseDirectory: String, fileName: String, mimeType: String?): String {
        if (!isSmartFoldersEnabled()) return baseDirectory
        val folder = smartFolderName(fileName, mimeType)
        val baseUri = Uri.parse(baseDirectory)
        if (baseUri.scheme == "content") {
            val child = findChildDocument(baseUri, folder)
                ?: contentResolverCreateDirectory(baseUri, folder)
            return child?.toString() ?: baseDirectory
        }
        val directory = File(baseDirectory, folder)
        if (!directory.exists() && !directory.mkdirs() && !directory.isDirectory) {
            return baseDirectory
        }
        return directory.absolutePath
    }

    private fun findChildDocument(parent: Uri, displayName: String): Uri? {
        val children = android.provider.DocumentsContract.buildChildDocumentsUriUsingTree(
            parent,
            android.provider.DocumentsContract.getTreeDocumentId(parent),
        )
        context.contentResolver.query(
            children,
            arrayOf(
                android.provider.DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                android.provider.DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                android.provider.DocumentsContract.Document.COLUMN_MIME_TYPE,
            ),
            null,
            null,
            null,
        )?.use { cursor ->
            val idIndex = cursor.getColumnIndexOrThrow(
                android.provider.DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            )
            val nameIndex = cursor.getColumnIndexOrThrow(
                android.provider.DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            )
            val mimeIndex = cursor.getColumnIndexOrThrow(
                android.provider.DocumentsContract.Document.COLUMN_MIME_TYPE,
            )
            while (cursor.moveToNext()) {
                if (
                    cursor.getString(nameIndex) == displayName &&
                    cursor.getString(mimeIndex) == android.provider.DocumentsContract.Document.MIME_TYPE_DIR
                ) {
                    val documentId = cursor.getString(idIndex)
                    return android.provider.DocumentsContract.buildDocumentUriUsingTree(parent, documentId)
                }
            }
        }
        return null
    }

    private fun contentResolverCreateDirectory(parent: Uri, displayName: String): Uri? =
        android.provider.DocumentsContract.createDocument(
            context.contentResolver,
            parent,
            android.provider.DocumentsContract.Document.MIME_TYPE_DIR,
            displayName,
        )

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

    /** Returns the public default FetchX directory under the device's Downloads root.
     *
     * The normal FetchX destination is /storage/emulated/0/Download/FetchX. This keeps the
     * app on the public Downloads tree while still using the existing MediaStore publication
     * workflow for final delivery and scoped storage compliance.
     */
    fun defaultDownloadDirectory(): File = defaultPublicFetchXDirectory()

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
    fun uniqueFileName(
        dir: File,
        desiredName: String,
        reservedFileNames: Set<String> = emptySet(),
    ): String = uniqueAvailableFileName(dir, sanitizeFileName(desiredName), reservedFileNames)
}

internal fun uniqueAvailableFileName(
    dir: File,
    safeName: String,
    reservedFileNames: Set<String> = emptySet(),
): String {
    if (safeName !in reservedFileNames && !File(dir, safeName).exists()) return safeName
    val dot = safeName.lastIndexOf('.')
    val base = if (dot > 0) safeName.substring(0, dot) else safeName
    val extension = if (dot > 0) safeName.substring(dot) else ""
    var index = 1
    while ("$base ($index)$extension" in reservedFileNames || File(dir, "$base ($index)$extension").exists()) {
        index++
    }
    return "$base ($index)$extension"
}

internal fun smartFolderName(fileName: String, mimeType: String? = null): String {
    val mime = mimeType?.lowercase().orEmpty()
    val extension = fileName.substringAfterLast('.', "").lowercase().let { if (it.isBlank()) "" else ".$it" }
    val sharedCategory = BridgeInitializer.matchDownloadCategory(fileName)
    if (sharedCategory != null) {
        return when (sharedCategory) {
            "Compressed" -> "Archives"
            "Music" -> "Audio"
            "Programs" -> "Others"
            else -> sharedCategory
        }
    }
    return when {
        mime.startsWith("video/") ||
            extension in setOf(".mp4", ".mkv", ".avi", ".flv", ".mov", ".webm", ".m4v") -> "Videos"
        mime.startsWith("audio/") ||
            extension in setOf(".mp3", ".m4a", ".opus", ".wav", ".flac", ".ogg", ".aac") -> "Audio"
        mime.contains("zip") || mime.contains("rar") || mime.contains("compressed") ||
            extension in setOf(".zip", ".rar", ".7z", ".tar", ".gz", ".bz2", ".xz") -> "Archives"
        mime.contains("pdf") || mime.contains("document") || mime.startsWith("text/") ||
            extension in setOf(".pdf", ".doc", ".docx", ".txt", ".xlsx", ".pptx", ".epub", ".rtf") -> "Documents"
        else -> "Others"
    }
}
