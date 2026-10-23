package dev.fetchx.bridge

import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import org.json.JSONObject
import java.io.File
import java.io.FileInputStream
import java.io.InputStream
import java.net.URLConnection
import java.security.MessageDigest

/**
 * Keeps Rust downloads on a private filesystem path and publishes finalized files to Downloads.
 */
class PublicDownloadStorage(context: Context) {

    private val context = context.applicationContext
    private val resolver = this.context.contentResolver
    private val preferences = this.context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
    private val stagingDirectory = File(this.context.filesDir, "downloads/public-staging")

    fun defaultDirectoryPath(): String {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            return context.getExternalFilesDir(null)?.absolutePath
                ?: File(context.filesDir, "downloads").absolutePath
        }
        return File(
            Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS),
            DIRECTORY_NAME,
        ).absolutePath
    }

    fun publicDirectoryPath(): String = File(
        Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS),
        DIRECTORY_NAME,
    ).absolutePath

    fun privateTorrentDirectoryPath(): String =
        context.getExternalFilesDir(null)?.absolutePath
            ?: File(context.filesDir, "downloads").absolutePath

    fun relativePublicDirectoryPath(): String = publicRelativeDirectoryPath(Environment.DIRECTORY_DOWNLOADS)

    fun publicDownloadsBytes(): Long =
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) 0L
        else queryRows().asSequence().filterNot { it.isPending }.sumOf { it.size }

    fun isPublicTarget(path: String): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return false
        val target = File(path).absoluteFile
        return target.parentFile?.canonicalPath == File(publicDirectoryPath()).canonicalPath &&
            target.name.isNotBlank() && target.name != "." && target.name != ".."
    }

    fun uniquePublicFileName(
        desiredName: String,
        ignoreStagingFileName: String? = null,
    ): String {
        val safeName = sanitizeFileName(desiredName)
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            val directory = File(defaultDirectoryPath())
            return uniqueFileName(safeName) { File(directory, it).exists() }
        }
        val existingNames = queryRows().mapTo(mutableSetOf()) { it.displayName }
        return uniqueFileName(safeName) {
            it in existingNames ||
                (it != ignoreStagingFileName && File(stagingDirectory, it).exists())
        }
    }

    @Synchronized
    fun stageDestination(publicDestination: String): String {
        require(isPublicTarget(publicDestination)) {
            "Only a file directly inside the default Downloads folder can use public staging."
        }
        val publicFileName = sanitizeFileName(File(publicDestination).name)
        if (!stagingDirectory.exists() && !stagingDirectory.mkdirs()) {
            throw IllegalStateException("Could not create the private download staging directory.")
        }
        val canonicalRoot = stagingDirectory.canonicalFile
        var stagingFile = File(canonicalRoot, publicFileName).canonicalFile
        require(stagingFile.parentFile == canonicalRoot) {
            "The download staging path escapes its private directory."
        }
        if (stagingFile.exists()) {
            stagingFile = File(canonicalRoot, uniqueFileName(publicFileName) { File(canonicalRoot, it).exists() })
        }
        return stagingFile.absolutePath
    }

    fun stageDestinationIfPublic(destination: String): String =
        if (isPublicTarget(destination)) stageDestination(destination) else destination

    fun torrentDirectoryIfPublic(destinationDirectory: String): String =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
            File(destinationDirectory).absolutePath == File(publicDirectoryPath()).absolutePath
        ) {
            privateTorrentDirectoryPath()
        } else {
            destinationDirectory
        }

    fun isStagingPath(path: String): Boolean {
        val root = stagingDirectory.canonicalFile
        val candidate = try {
            File(path).canonicalFile
        } catch (_: java.io.IOException) {
            return false
        }
        return candidate.parentFile == root && candidate.name.isNotBlank()
    }

    fun displayDestination(path: String): String {
        if (!isStagingPath(path)) return path
        return publishedUri(path)?.toString() ?: File(publicDirectoryPath(), File(path).name).absolutePath
    }

    @Synchronized
    fun publishedUri(stagingPath: String): Uri? {
        if (!isStagingPath(stagingPath)) return null
        val raw = preferences.getString(publicationKey(stagingPath), null) ?: return null
        val uri = Uri.parse(raw)
        if (isPublished(uri)) return uri
        preferences.edit().remove(publicationKey(stagingPath)).commit()
        return null
    }

    @Synchronized
    fun publishCompleted(stagingPath: String): Uri {
        require(isStagingPath(stagingPath)) {
            "Completed download is not in FetchX's private staging directory."
        }
        publishedUri(stagingPath)?.let {
            deleteStagingFile(stagingPath)
            return it
        }
        check(File(stagingPath).isFile) {
            "The completed staging file is missing."
        }
        val stagingFile = File(stagingPath).canonicalFile
        val fileSize = stagingFile.length()
        val digest = sha256(stagingFile)
        val relativePath = "$RELATIVE_DIRECTORY/"

        check(Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            "MediaStore public downloads require Android 10 or newer."
        }
        recoverInterruptedPublication(stagingPath, fileSize, digest)?.let {
            deleteStagingFile(stagingPath)
            return it
        }

        val sameContent = queryRows().firstOrNull { row ->
            row.relativePath == relativePath &&
                row.size == fileSize &&
                row.displayName == stagingFile.name &&
                !row.isPending &&
                row.uri != null &&
                sha256(resolver.openInputStream(row.uri) ?: return@firstOrNull false) == digest
        }
        if (sameContent?.uri != null && !sameContent.isPending) {
            savePublished(stagingPath, sameContent.uri)
            deleteStagingFile(stagingPath)
            return sameContent.uri
        }

        val displayName = uniquePublicFileName(
            stagingFile.name,
            ignoreStagingFileName = stagingFile.name,
        )
        check(
            preferences.edit().putString(
                reservationKey(stagingPath),
                JSONObject()
                    .put("name", displayName)
                    .put("size", fileSize)
                    .put("sha256", digest)
                    .toString(),
            ).commit(),
        ) {
            "Could not persist the public download reservation."
        }
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, displayName)
            put(MediaStore.MediaColumns.MIME_TYPE, URLConnection.guessContentTypeFromName(displayName) ?: "application/octet-stream")
            put(MediaStore.MediaColumns.RELATIVE_PATH, relativePath)
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            ?: run {
                preferences.edit().remove(reservationKey(stagingPath)).commit()
                throw IllegalStateException("Android did not create a public download item.")
            }
        savePending(stagingPath, uri)

        try {
            val copied = FileInputStream(stagingFile).use { input ->
                resolver.openOutputStream(uri, "w")?.use { output ->
                    input.copyTo(output, DEFAULT_BUFFER_SIZE)
                } ?: throw IllegalStateException("Android could not open the public download for writing.")
            }
            check(copied == fileSize) {
                "Public download verification failed: expected $fileSize bytes, wrote $copied."
            }
            val writtenStream = resolver.openInputStream(uri)
                ?: throw IllegalStateException("Android could not verify the public download.")
            check(sha256(writtenStream) == digest) {
                "Public download verification failed: file contents do not match the staged download."
            }
            val published = ContentValues().apply {
                put(MediaStore.MediaColumns.IS_PENDING, 0)
            }
            check(resolver.update(uri, published, null, null) == 1) {
                "Android did not publish the completed download."
            }
            savePublished(stagingPath, uri)
            deleteStagingFile(stagingPath)
            return uri
        } catch (error: Exception) {
            val deleted = resolver.delete(uri, null, null)
            if (deleted == 1) {
                preferences.edit()
                    .remove(pendingKey(stagingPath))
                    .remove(reservationKey(stagingPath))
                    .commit()
            }
            throw error
        }
    }

    @Synchronized
    fun reconcileCompleted(stagingPath: String): Uri = publishCompleted(stagingPath)

    @Synchronized
    fun cleanupPublishedStagingArtifacts(stagingPath: String) {
        if (isStagingPath(stagingPath) && preferences.contains(publicationKey(stagingPath))) {
            deleteStagingFile(stagingPath)
        }
    }

    @Synchronized
    fun deletePublished(stagingPath: String) {
        val uri = publishedUri(stagingPath) ?: return
        check(resolver.delete(uri, null, null) == 1) {
            "Could not remove the published FetchX download."
        }
        preferences.edit().remove(publicationKey(stagingPath)).commit()
    }

    @Synchronized
    fun forgetPublication(stagingPath: String) {
        preferences.edit()
            .remove(publicationKey(stagingPath))
            .remove(pendingKey(stagingPath))
            .remove(reservationKey(stagingPath))
            .commit()
    }

    private fun queryRows(): List<MediaStoreRow> {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return emptyList()
        val projection = arrayOf(
            MediaStore.MediaColumns._ID,
            MediaStore.MediaColumns.DISPLAY_NAME,
            MediaStore.MediaColumns.RELATIVE_PATH,
            MediaStore.MediaColumns.SIZE,
            MediaStore.MediaColumns.IS_PENDING,
        )
        return resolver.query(
            MediaStore.Downloads.EXTERNAL_CONTENT_URI,
            projection,
            "${MediaStore.MediaColumns.RELATIVE_PATH} = ?",
            arrayOf("$RELATIVE_DIRECTORY/"),
            "${MediaStore.MediaColumns.DISPLAY_NAME} ASC",
        )?.use { cursor ->
            val idIndex = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
            val nameIndex = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
            val pathIndex = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns.RELATIVE_PATH)
            val sizeIndex = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
            val pendingIndex = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns.IS_PENDING)
            buildList {
                while (cursor.moveToNext()) {
                    val id = cursor.getLong(idIndex)
                    add(
                        MediaStoreRow(
                            uri = ContentUris.withAppendedId(MediaStore.Downloads.EXTERNAL_CONTENT_URI, id),
                            displayName = cursor.getString(nameIndex),
                            relativePath = cursor.getString(pathIndex),
                            size = cursor.getLong(sizeIndex),
                            isPending = cursor.getInt(pendingIndex) != 0,
                        ),
                    )
                }
            }
        } ?: emptyList()
    }

    private fun isPublished(uri: Uri): Boolean {
        if (uri.scheme == "file") return File(uri.path.orEmpty()).isFile
        return resolver.query(
            uri,
            arrayOf(MediaStore.MediaColumns.IS_PENDING),
            null,
            null,
            null,
        )?.use { cursor ->
            cursor.moveToFirst() && cursor.getInt(0) == 0
        } ?: false
    }

    private fun uriExists(uri: Uri): Boolean =
        resolver.query(uri, arrayOf(MediaStore.MediaColumns._ID), null, null, null)?.use {
            it.moveToFirst()
        } ?: false

    private fun recoverInterruptedPublication(
        stagingPath: String,
        fileSize: Long,
        digest: String,
    ): Uri? {
        val pendingRaw = preferences.getString(pendingKey(stagingPath), null)
        if (pendingRaw != null) {
            val uri = Uri.parse(pendingRaw)
            if (isPublished(uri)) {
                val row = queryRows().firstOrNull { it.uri == uri }
                val stream = resolver.openInputStream(uri)
                    ?: throw IllegalStateException("Could not verify a recovered public download.")
                if (row != null && row.size == fileSize && sha256(stream) == digest) {
                    savePublished(stagingPath, uri)
                    return uri
                }
            }
            if (resolver.delete(uri, null, null) != 1 && uriExists(uri)) {
                throw IllegalStateException("Could not clean up an interrupted public download.")
            }
            check(preferences.edit().remove(pendingKey(stagingPath)).commit()) {
                "Could not clear the interrupted publication marker."
            }
        }

        val rawReservation = preferences.getString(reservationKey(stagingPath), null) ?: return null
        val reservation = JSONObject(rawReservation)
        val name = reservation.getString("name")
        val reservedSize = reservation.getLong("size")
        val reservedDigest = reservation.getString("sha256")
        val matchingRows = queryRows().filter {
            it.displayName == name && it.relativePath == "$RELATIVE_DIRECTORY/"
        }
        for (row in matchingRows) {
            if (row.isPending) {
                val uri = requireNotNull(row.uri)
                if (resolver.delete(uri, null, null) != 1 && uriExists(uri)) {
                    throw IllegalStateException("Could not clean up an interrupted public download.")
                }
                continue
            }
            val stream = row.uri?.let(resolver::openInputStream) ?: continue
            val sameContent = row.size == reservedSize && sha256(stream) == reservedDigest
            if (sameContent) {
                val uri = requireNotNull(row.uri)
                savePublished(stagingPath, uri)
                return uri
            }
        }
        check(
            preferences.edit()
                .remove(reservationKey(stagingPath))
                .remove(pendingKey(stagingPath))
                .commit(),
        ) {
            "Could not clear the interrupted publication reservation."
        }
        return null
    }

    private fun savePending(stagingPath: String, uri: Uri) {
        check(preferences.edit().putString(pendingKey(stagingPath), uri.toString()).commit()) {
            "Could not persist the pending public download."
        }
    }

    private fun deleteStagingFile(stagingPath: String) {
        val root = stagingDirectory.canonicalFile
        listOf(
            File(stagingPath),
            File("$stagingPath.part"),
            File("$stagingPath.part.meta"),
        ).forEach { artifact ->
            val canonical = artifact.canonicalFile
            if (canonical.parentFile != root) {
                Log.e(TAG, "Refusing to remove a staging artifact outside the private staging directory.")
            } else if (canonical.exists() && !canonical.delete()) {
                Log.w(TAG, "Published download but could not remove a private staging artifact.")
            }
        }
    }

    private fun savePublished(stagingPath: String, uri: Uri) {
        check(
            preferences.edit()
                .putString(publicationKey(stagingPath), uri.toString())
                .remove(pendingKey(stagingPath))
                .remove(reservationKey(stagingPath))
                .commit(),
        ) {
            "Could not persist the published download location."
        }
    }

    private fun publicationKey(path: String) = "published_${sha256(path.toByteArray())}"
    private fun pendingKey(path: String) = "pending_${sha256(path.toByteArray())}"
    private fun reservationKey(path: String) = "reservation_${sha256(path.toByteArray())}"

    private fun uniqueFileName(desiredName: String, exists: (String) -> Boolean): String {
        return uniquePublicFileName(desiredName, exists)
    }

    private fun sanitizeFileName(raw: String): String {
        return sanitizePublicFileName(raw)
    }

    private fun sha256(file: File): String = FileInputStream(file).use { input ->
        sha256(input)
    }

    private fun sha256(input: InputStream): String {
        val digest = MessageDigest.getInstance("SHA-256")
        val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
        input.use { stream ->
            while (true) {
                val count = stream.read(buffer)
                if (count < 0) break
                digest.update(buffer, 0, count)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    private fun sha256(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    private data class MediaStoreRow(
        val uri: Uri?,
        val displayName: String,
        val relativePath: String,
        val size: Long,
        val isPending: Boolean,
    )

    private companion object {
        const val TAG = "FetchXPublicStorage"
        const val PREFERENCES_NAME = "fetchx_public_downloads"
        const val DIRECTORY_NAME = "FetchX"
        val RELATIVE_DIRECTORY = publicRelativeDirectoryPath(Environment.DIRECTORY_DOWNLOADS).removeSuffix("/")
        const val MAX_FILENAME_LENGTH = 200
    }
}

internal fun publicRelativeDirectoryPath(downloadsDirectoryName: String): String =
    "$downloadsDirectoryName/FetchX/"

internal fun sanitizePublicFileName(raw: String): String {
    val result = raw.trim()
        .replace('/', '_')
        .replace('\\', '_')
        .replace('\u0000', '_')
        .replace("..", "_")
        .trimStart('.')
        .replace(Regex("[<>:\"|?*]"), "_")
        .take(200)
    return result.ifBlank { "download" }
}

internal fun uniquePublicFileName(desiredName: String, exists: (String) -> Boolean): String {
    if (!exists(desiredName)) return desiredName
    val dot = desiredName.lastIndexOf('.')
    val base = if (dot > 0) desiredName.substring(0, dot) else desiredName
    val extension = if (dot > 0) desiredName.substring(dot) else ""
    var index = 1
    while (exists("$base ($index)$extension")) index++
    return "$base ($index)$extension"
}
