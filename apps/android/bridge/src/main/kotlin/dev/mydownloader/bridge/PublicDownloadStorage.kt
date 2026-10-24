package dev.fetchx.bridge

import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import java.io.File
import java.io.FileInputStream
import java.io.InputStream
import java.nio.charset.StandardCharsets
import java.net.URLConnection
import java.security.MessageDigest
import java.util.Base64

internal data class PublicMediaRow(
    val uri: String,
    val displayName: String,
    val relativePath: String,
    val size: Long,
    val isPending: Boolean,
    val ownerPackageName: String?,
)

internal interface PublicDownloadMediaStore {
    fun queryRows(): List<PublicMediaRow>
    fun find(uri: String): PublicMediaRow?
    fun insertPending(displayName: String, mimeType: String, relativePath: String): String
    fun write(uri: String, source: File): Long
    fun sha256(uri: String): String
    fun publish(uri: String): Boolean
    fun delete(uri: String): Boolean
}

internal interface PublicationStateStore {
    fun get(key: String): String?
    fun contains(key: String): Boolean
    fun entries(): Map<String, String>
    fun update(changes: Map<String, String?>): Boolean
}

internal data class PublicDownloadReservation(
    val name: String,
    val size: Long,
    val sha256: String,
)

internal fun encodePublicDownloadReservation(reservation: PublicDownloadReservation): String =
    "${reservation.size}:${reservation.sha256}:" +
        Base64.getEncoder().encodeToString(reservation.name.toByteArray(StandardCharsets.UTF_8))

internal fun decodePublicDownloadReservation(raw: String): PublicDownloadReservation {
    if (raw.startsWith("{")) {
        val name = Regex("\"name\"\\s*:\\s*\"([^\"]*)\"").find(raw)?.groupValues?.get(1)
            ?: throw IllegalArgumentException("Invalid saved public download reservation.")
        val size = Regex("\"size\"\\s*:\\s*(\\d+)").find(raw)?.groupValues?.get(1)?.toLong()
            ?: throw IllegalArgumentException("Invalid saved public download reservation.")
        val digest = Regex("\"sha256\"\\s*:\\s*\"([0-9a-fA-F]{64})\"").find(raw)?.groupValues?.get(1)
            ?: throw IllegalArgumentException("Invalid saved public download reservation.")
        return PublicDownloadReservation(sanitizePublicFileName(name), size, digest.lowercase())
    }
    val parts = raw.split(':', limit = 3)
    require(parts.size == 3) { "Invalid saved public download reservation." }
    val size = parts[0].toLongOrNull()
        ?: throw IllegalArgumentException("Invalid saved public download reservation.")
    val digest = parts[1].takeIf { it.matches(Regex("[0-9a-fA-F]{64}")) }?.lowercase()
        ?: throw IllegalArgumentException("Invalid saved public download reservation.")
    val name = String(Base64.getDecoder().decode(parts[2]), StandardCharsets.UTF_8)
    require(name == sanitizePublicFileName(name)) { "Invalid saved public download reservation." }
    return PublicDownloadReservation(name, size, digest)
}

private class SharedPreferencesPublicationState(
    private val preferences: android.content.SharedPreferences,
) : PublicationStateStore {
    override fun get(key: String): String? = preferences.getString(key, null)

    override fun contains(key: String): Boolean = preferences.contains(key)

    override fun entries(): Map<String, String> =
        preferences.all.mapNotNull { (key, value) -> (value as? String)?.let { key to it } }.toMap()

    override fun update(changes: Map<String, String?>): Boolean =
        preferences.edit().apply {
            changes.forEach { (key, value) ->
                if (value == null) remove(key) else putString(key, value)
            }
        }.commit()
}

private class AndroidPublicDownloadMediaStore(context: Context) : PublicDownloadMediaStore {
    private val resolver = context.contentResolver
    private val packageName = context.packageName

    override fun queryRows(): List<PublicMediaRow> {
        val projection = arrayOf(
            MediaStore.MediaColumns._ID,
            MediaStore.MediaColumns.DISPLAY_NAME,
            MediaStore.MediaColumns.RELATIVE_PATH,
            MediaStore.MediaColumns.SIZE,
            MediaStore.MediaColumns.IS_PENDING,
            MediaStore.MediaColumns.OWNER_PACKAGE_NAME,
        )
        val cursor = resolver.query(
            MediaStore.Downloads.EXTERNAL_CONTENT_URI,
            projection,
            "${MediaStore.MediaColumns.RELATIVE_PATH} = ?",
            arrayOf("${Environment.DIRECTORY_DOWNLOADS}/FetchX/"),
            "${MediaStore.MediaColumns.DISPLAY_NAME} ASC",
        ) ?: throw IllegalStateException("Could not query public FetchX downloads.")
        return cursor.use {
            val idIndex = it.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
            val nameIndex = it.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
            val pathIndex = it.getColumnIndexOrThrow(MediaStore.MediaColumns.RELATIVE_PATH)
            val sizeIndex = it.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
            val pendingIndex = it.getColumnIndexOrThrow(MediaStore.MediaColumns.IS_PENDING)
            val ownerIndex = it.getColumnIndex(MediaStore.MediaColumns.OWNER_PACKAGE_NAME)
            buildList {
                while (it.moveToNext()) {
                    val id = it.getLong(idIndex)
                    add(
                        PublicMediaRow(
                            uri = ContentUris.withAppendedId(MediaStore.Downloads.EXTERNAL_CONTENT_URI, id).toString(),
                            displayName = it.getString(nameIndex),
                            relativePath = it.getString(pathIndex),
                            size = it.getLong(sizeIndex),
                            isPending = it.getInt(pendingIndex) != 0,
                            ownerPackageName = if (ownerIndex >= 0) it.getString(ownerIndex) else packageName,
                        ),
                    )
                }
            }
        }
    }

    override fun find(uri: String): PublicMediaRow? {
        val safeUri = validatedUri(uri)
        val cursor = resolver.query(
            safeUri,
            arrayOf(
                MediaStore.MediaColumns.DISPLAY_NAME,
                MediaStore.MediaColumns.RELATIVE_PATH,
                MediaStore.MediaColumns.SIZE,
                MediaStore.MediaColumns.IS_PENDING,
                MediaStore.MediaColumns.OWNER_PACKAGE_NAME,
            ),
            null,
            null,
            null,
        ) ?: throw IllegalStateException("Could not inspect a public FetchX download.")
        return cursor.use {
            if (!it.moveToFirst()) return@use null
            val ownerIndex = it.getColumnIndex(MediaStore.MediaColumns.OWNER_PACKAGE_NAME)
            PublicMediaRow(
                uri = safeUri.toString(),
                displayName = it.getString(it.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)),
                relativePath = it.getString(it.getColumnIndexOrThrow(MediaStore.MediaColumns.RELATIVE_PATH)),
                size = it.getLong(it.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)),
                isPending = it.getInt(it.getColumnIndexOrThrow(MediaStore.MediaColumns.IS_PENDING)) != 0,
                ownerPackageName = if (ownerIndex >= 0) it.getString(ownerIndex) else packageName,
            )
        }
    }

    override fun insertPending(displayName: String, mimeType: String, relativePath: String): String {
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, displayName)
            put(MediaStore.MediaColumns.MIME_TYPE, mimeType)
            put(MediaStore.MediaColumns.RELATIVE_PATH, relativePath)
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        return resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)?.toString()
            ?: throw IllegalStateException("Android did not create a public download item.")
    }

    override fun write(uri: String, source: File): Long {
        val safeUri = validatedUri(uri)
        return FileInputStream(source).use { input ->
            resolver.openOutputStream(safeUri, "w")?.use { output ->
                input.copyTo(output, DEFAULT_BUFFER_SIZE)
            } ?: throw IllegalStateException("Android could not open the public download for writing.")
        }
    }

    override fun sha256(uri: String): String {
        val safeUri = validatedUri(uri)
        val input = resolver.openInputStream(safeUri)
            ?: throw IllegalStateException("Android could not verify the public download.")
        return input.use(::sha256Stream)
    }

    override fun publish(uri: String): Boolean {
        val safeUri = validatedUri(uri)
        val values = ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }
        return resolver.update(safeUri, values, null, null) == 1
    }

    override fun delete(uri: String): Boolean {
        val safeUri = validatedUri(uri)
        return resolver.delete(safeUri, null, null) == 1
    }

    private fun validatedUri(raw: String): Uri {
        val uri = Uri.parse(raw)
        val collection = MediaStore.Downloads.EXTERNAL_CONTENT_URI
        val segments = uri.pathSegments
        require(
            uri.scheme == collection.scheme &&
                uri.authority == collection.authority &&
                segments.size == collection.pathSegments.size + 1 &&
                segments.take(collection.pathSegments.size) == collection.pathSegments &&
                segments.last().toLongOrNull()?.let { it > 0L } == true,
        ) {
            "Refusing to use a URI outside the MediaStore Downloads collection."
        }
        return uri
    }

    companion object {
        private fun sha256Stream(input: InputStream): String {
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
    }
}

/**
 * Keeps Rust downloads on a private filesystem path and publishes finalized files to Downloads.
 */
class PublicDownloadStorage private constructor(
    private val context: Context?,
    private val stagingDirectory: File,
    private val publicDirectory: File,
    private val mediaStore: PublicDownloadMediaStore,
    private val state: PublicationStateStore,
    private val sdkInt: Int,
    private val appPackageName: String,
) {
    private val failedPublicationAttempts = mutableSetOf<String>()

    constructor(context: Context) : this(
        context.applicationContext,
        File(context.applicationContext.filesDir, "downloads/public-staging"),
        File(
            Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS),
            DIRECTORY_NAME,
        ),
        AndroidPublicDownloadMediaStore(context.applicationContext),
        SharedPreferencesPublicationState(
            context.applicationContext.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE),
        ),
        Build.VERSION.SDK_INT,
        context.applicationContext.packageName,
    )

    internal constructor(
        stagingDirectory: File,
        publicDirectory: File,
        mediaStore: PublicDownloadMediaStore,
        state: PublicationStateStore,
        appPackageName: String = "dev.fetchx.test",
    ) : this(null, stagingDirectory, publicDirectory, mediaStore, state, Build.VERSION_CODES.Q, appPackageName)

    fun defaultDirectoryPath(): String {
        if (sdkInt < Build.VERSION_CODES.Q) {
            return context?.getExternalFilesDir(null)?.absolutePath
                ?: File(context?.filesDir ?: stagingDirectory.parentFile, "downloads").absolutePath
        }
        return publicDirectory.absolutePath
    }

    fun publicDirectoryPath(): String = publicDirectory.absolutePath

    fun privateTorrentDirectoryPath(): String =
        context?.getExternalFilesDir(null)?.absolutePath
            ?: File(context?.filesDir ?: stagingDirectory.parentFile, "downloads").absolutePath

    fun relativePublicDirectoryPath(): String = publicRelativeDirectoryPath(Environment.DIRECTORY_DOWNLOADS)

    fun publicDownloadsBytes(): Long =
        if (sdkInt < Build.VERSION_CODES.Q) 0L
        else queryRows().asSequence().filterNot { it.isPending }.sumOf { it.size }

    fun isPublicTarget(path: String): Boolean {
        if (sdkInt < Build.VERSION_CODES.Q) return false
        return try {
            val target = File(path).absoluteFile
            target.parentFile?.canonicalPath == File(publicDirectoryPath()).canonicalPath &&
                target.name.isNotBlank() && target.name != "." && target.name != ".."
        } catch (_: java.io.IOException) {
            false
        }
    }

    fun uniquePublicFileName(
        desiredName: String,
        ignoreStagingFileName: String? = null,
        reservedFileNames: Set<String> = emptySet(),
    ): String {
        val safeName = sanitizeFileName(desiredName)
        if (sdkInt < Build.VERSION_CODES.Q) {
            val directory = File(defaultDirectoryPath())
            return uniqueAvailablePublicFileName(safeName, reservedFileNames) {
                File(directory, it).exists()
            }
        }
        val existingNames = queryRows().mapTo(mutableSetOf()) { it.displayName }
        val persistedReservedNames = state.entries()
            .filterKeys { it.startsWith(RESERVATION_PREFIX) }
            .values
            .mapTo(mutableSetOf()) { raw -> decodePublicDownloadReservation(raw).name }
        return uniqueAvailablePublicFileName(safeName, reservedFileNames) {
            it in existingNames || it in persistedReservedNames ||
                (it != ignoreStagingFileName && hasStagingArtifacts(it))
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
        while (
            hasStagingArtifacts(stagingFile.name) ||
            state.contains(publicationKey(stagingFile.absolutePath)) ||
            state.contains(pendingKey(stagingFile.absolutePath)) ||
            state.contains(reservationKey(stagingFile.absolutePath))
        ) {
            val nextName = uniqueFileName(publicFileName) { name ->
                val path = File(canonicalRoot, name).absolutePath
                hasStagingArtifacts(name) ||
                    state.contains(publicationKey(path)) ||
                    state.contains(pendingKey(path)) ||
                    state.contains(reservationKey(path))
            }
            stagingFile = File(canonicalRoot, nextName).canonicalFile
            require(stagingFile.parentFile == canonicalRoot) {
                "The download staging path escapes its private directory."
            }
        }
        return stagingFile.absolutePath
    }

    private fun hasStagingArtifacts(fileName: String): Boolean {
        if (File(stagingDirectory, fileName).exists()) return true
        return stagingDirectory.listFiles()?.any {
            it.name == "$fileName.part" || it.name.startsWith("$fileName.part.")
        } == true
    }

    fun stageDestinationIfPublic(destination: String): String =
        if (isPublicTarget(destination)) stageDestination(destination) else destination

    fun torrentDirectoryIfPublic(destinationDirectory: String): String =
        if (sdkInt >= Build.VERSION_CODES.Q &&
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
        return publishedReference(stagingPath)?.let(Uri::parse)
    }

    @Synchronized
    fun publishCompleted(stagingPath: String): Uri? =
        publishCompletedReference(stagingPath, forceRetry = false)?.let(Uri::parse)

    @Synchronized
    fun retryPublication(stagingPath: String): Uri? =
        publishCompletedReference(stagingPath, forceRetry = true)?.let(Uri::parse)

    @Synchronized
    internal fun publishCompletedReference(stagingPath: String, forceRetry: Boolean = false): String? {
        require(isStagingPath(stagingPath)) {
            "Completed download is not in FetchX's private staging directory."
        }
        if (!forceRetry && stagingPath in failedPublicationAttempts) return null
        if (forceRetry) failedPublicationAttempts.remove(stagingPath)

        try {
            publishedReference(stagingPath)?.let {
                deleteStagingFile(stagingPath)
                failedPublicationAttempts.remove(stagingPath)
                return it
            }
            check(File(stagingPath).isFile) {
                "The completed staging file is missing."
            }
            val stagingFile = File(stagingPath).canonicalFile
            val fileSize = stagingFile.length()
            val digest = sha256(stagingFile)
            val relativePath = "$RELATIVE_DIRECTORY/"

            check(sdkInt >= Build.VERSION_CODES.Q) {
                "MediaStore public downloads require Android 10 or newer."
            }
            recoverInterruptedPublication(stagingPath, fileSize, digest)?.let {
                deleteStagingFile(stagingPath)
                failedPublicationAttempts.remove(stagingPath)
                return it
            }

            val reservation = reserveName(stagingPath, stagingFile.name, fileSize, digest)
            val uri = mediaStore.insertPending(
                displayName = reservation.name,
                mimeType = URLConnection.guessContentTypeFromName(reservation.name) ?: "application/octet-stream",
                relativePath = relativePath,
            )
            try {
                savePending(stagingPath, uri)
                val copied = mediaStore.write(uri, stagingFile)
                check(copied == fileSize) {
                    "Public download verification failed: expected $fileSize bytes, wrote $copied."
                }
                check(mediaStore.sha256(uri) == digest) {
                    "Public download verification failed: file contents do not match the staged download."
                }
                check(mediaStore.publish(uri)) {
                    "Android did not publish the completed download."
                }
                savePublished(stagingPath, uri)
                deleteStagingFile(stagingPath)
                failedPublicationAttempts.remove(stagingPath)
                return uri
            } catch (error: Exception) {
                try {
                    val row = mediaStore.find(uri)
                    if (row?.isPending == true) {
                        check(row.relativePath == relativePath && row.ownerPackageName == appPackageName) {
                            "Refusing to clean up a public download not owned by FetchX."
                        }
                        if (!mediaStore.delete(uri) && mediaStore.find(uri) != null) {
                            throw IllegalStateException("Could not remove the incomplete public download.")
                        }
                        check(state.update(mapOf(pendingKey(stagingPath) to null))) {
                            "Could not clear the incomplete publication marker."
                        }
                    }
                } catch (cleanupError: Exception) {
                    error.addSuppressed(cleanupError)
                }
                throw error
            }
        } catch (error: Exception) {
            failedPublicationAttempts.add(stagingPath)
            throw error
        }
    }

    @Synchronized
    fun reconcileCompleted(stagingPath: String): Uri? = publishCompleted(stagingPath)

    @Synchronized
    fun cleanupPublishedStagingArtifacts(stagingPath: String) {
        if (isStagingPath(stagingPath) && state.contains(publicationKey(stagingPath))) {
            deleteStagingFile(stagingPath)
        }
    }

    @Synchronized
    fun deletePublished(stagingPath: String) {
        val uri = publishedReference(stagingPath) ?: return
        check(mediaStore.delete(uri)) {
            "Could not remove the published FetchX download."
        }
        check(state.update(mapOf(publicationKey(stagingPath) to null))) {
            "Could not clear the published download state."
        }
    }

    @Synchronized
    fun forgetPublication(stagingPath: String) {
        check(
            state.update(
                mapOf(
                    publicationKey(stagingPath) to null,
                    pendingKey(stagingPath) to null,
                    reservationKey(stagingPath) to null,
                ),
            ),
        ) {
            "Could not clear the published download state."
        }
    }

    private fun queryRows(): List<PublicMediaRow> {
        if (sdkInt < Build.VERSION_CODES.Q) return emptyList()
        return mediaStore.queryRows()
            .filter { it.relativePath == "$RELATIVE_DIRECTORY/" }
    }

    private fun publishedReference(stagingPath: String): String? {
        if (!isStagingPath(stagingPath)) return null
        val key = publicationKey(stagingPath)
        val raw = state.get(key) ?: return null
        val row = mediaStore.find(raw)
        if (row == null) {
            check(state.update(mapOf(key to null))) {
                "Could not clear a missing public download reference."
            }
            return null
        }
        check(row.relativePath == "$RELATIVE_DIRECTORY/") {
            "The saved public download is outside FetchX's Downloads folder."
        }
        return raw.takeUnless { row.isPending }
    }

    private fun recoverInterruptedPublication(
        stagingPath: String,
        fileSize: Long,
        digest: String,
    ): String? {
        val pendingRaw = state.get(pendingKey(stagingPath))
        if (pendingRaw != null) {
            val row = mediaStore.find(pendingRaw)
            if (row != null) {
                check(row.relativePath == "$RELATIVE_DIRECTORY/") {
                    "The saved pending download is outside FetchX's Downloads folder."
                }
                if (!row.isPending && row.size == fileSize && mediaStore.sha256(pendingRaw) == digest) {
                    savePublished(stagingPath, pendingRaw)
                    return pendingRaw
                }
                if (row.isPending) {
                    deleteOwnedPending(pendingRaw, row)
                }
            }
            check(state.update(mapOf(pendingKey(stagingPath) to null))) {
                "Could not clear the interrupted publication marker."
            }
        }

        val reservation = readReservation(stagingPath) ?: return null
        val matchingRows = queryRows().filter { it.displayName == reservation.name }
        var nameOccupiedByDifferentContent = false
        for (row in matchingRows) {
            if (row.isPending) {
                if (row.ownerPackageName == appPackageName) {
                    deleteOwnedPending(row.uri, row)
                } else {
                    nameOccupiedByDifferentContent = true
                }
                continue
            }
            val sameContent = row.size == reservation.size && mediaStore.sha256(row.uri) == reservation.sha256
            if (sameContent) {
                savePublished(stagingPath, row.uri)
                return row.uri
            }
            nameOccupiedByDifferentContent = true
        }
        if (nameOccupiedByDifferentContent) {
            check(state.update(mapOf(reservationKey(stagingPath) to null))) {
                "Could not release an occupied public download name."
            }
        }
        return null
    }

    private fun reserveName(
        stagingPath: String,
        sourceName: String,
        size: Long,
        digest: String,
    ): PublicDownloadReservation {
        readReservation(stagingPath)?.let { reservation ->
            if (reservation.size == size && reservation.sha256 == digest) {
                val nameInUse = queryRows().any { it.displayName == reservation.name }
                if (!nameInUse) return reservation
            }
            check(state.update(mapOf(reservationKey(stagingPath) to null))) {
                "Could not release an unused public download name."
            }
        }
        val name = uniquePublicFileName(sourceName, ignoreStagingFileName = sourceName)
        val reservation = PublicDownloadReservation(name, size, digest)
        check(
            state.update(
                mapOf(
                    reservationKey(stagingPath) to encodePublicDownloadReservation(reservation),
                ),
            ),
        ) {
            "Could not persist the public download reservation."
        }
        return reservation
    }

    private fun readReservation(stagingPath: String): PublicDownloadReservation? {
        val raw = state.get(reservationKey(stagingPath)) ?: return null
        return decodePublicDownloadReservation(raw)
    }

    private fun deleteOwnedPending(uri: String, row: PublicMediaRow) {
        check(row.ownerPackageName == appPackageName && row.relativePath == "$RELATIVE_DIRECTORY/") {
            "Refusing to remove a pending download not owned by FetchX."
        }
        if (!mediaStore.delete(uri) && mediaStore.find(uri) != null) {
            throw IllegalStateException("Could not clean up an interrupted public download.")
        }
    }

    @Synchronized
    internal fun reconcileOrphanedPublications(activeCompletedStagingPaths: Set<String>) {
        val activeHashes = activeCompletedStagingPaths
            .filter(::isStagingPath)
            .mapTo(mutableSetOf()) { sha256(it.toByteArray()) }
        val activePendingUris = state.entries()
            .filterKeys { key -> key.startsWith(PENDING_PREFIX) && key.removePrefix(PENDING_PREFIX) in activeHashes }
            .values
            .toSet()
        val activeReservedNames = state.entries()
            .filterKeys { key -> key.startsWith(RESERVATION_PREFIX) && key.removePrefix(RESERVATION_PREFIX) in activeHashes }
            .values
            .mapTo(mutableSetOf()) { decodePublicDownloadReservation(it).name }

        queryRows().asSequence()
            .filter {
                it.isPending &&
                    it.ownerPackageName == appPackageName &&
                    it.uri !in activePendingUris &&
                    it.displayName !in activeReservedNames
            }
            .forEach { row -> deleteOwnedPending(row.uri, row) }

        val staleKeys = state.entries().keys
            .filter { key ->
                (key.startsWith(PENDING_PREFIX) || key.startsWith(RESERVATION_PREFIX)) &&
                    key.substringAfter('_') !in activeHashes
            }
        if (staleKeys.isNotEmpty()) {
            check(state.update(staleKeys.associateWith { null })) {
                "Could not clear stale public download publication state."
            }
        }
    }

    internal fun <T> withPublicationLock(action: () -> T): T = synchronized(this) { action() }

    private fun savePending(stagingPath: String, uri: String) {
        check(state.update(mapOf(pendingKey(stagingPath) to uri))) {
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

    private fun savePublished(stagingPath: String, uri: String) {
        check(
            state.update(
                mapOf(
                    publicationKey(stagingPath) to uri,
                    pendingKey(stagingPath) to null,
                    reservationKey(stagingPath) to null,
                ),
            ),
        ) {
            "Could not persist the published download location."
        }
    }

    private fun publicationKey(path: String) = "published_${sha256(path.toByteArray())}"
    private fun pendingKey(path: String) = "pending_${sha256(path.toByteArray())}"
    private fun reservationKey(path: String) = "reservation_${sha256(path.toByteArray())}"

    private fun uniqueFileName(desiredName: String, exists: (String) -> Boolean): String {
        return uniqueAvailablePublicFileName(desiredName, exists = exists)
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

    private companion object {
        const val TAG = "FetchXPublicStorage"
        const val PREFERENCES_NAME = "fetchx_public_downloads"
        const val DIRECTORY_NAME = "FetchX"
        const val PENDING_PREFIX = "pending_"
        const val RESERVATION_PREFIX = "reservation_"
        val RELATIVE_DIRECTORY = publicRelativeDirectoryPath(DOWNLOADS_DIRECTORY_NAME).removeSuffix("/")
        const val DOWNLOADS_DIRECTORY_NAME = "Download"
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

internal fun uniqueAvailablePublicFileName(
    desiredName: String,
    reservedFileNames: Set<String> = emptySet(),
    exists: (String) -> Boolean,
): String {
    if (desiredName !in reservedFileNames && !exists(desiredName)) return desiredName
    val dot = desiredName.lastIndexOf('.')
    val base = if (dot > 0) desiredName.substring(0, dot) else desiredName
    val extension = if (dot > 0) desiredName.substring(dot) else ""
    var index = 1
    while ("$base ($index)$extension" in reservedFileNames || exists("$base ($index)$extension")) index++
    return "$base ($index)$extension"
}
