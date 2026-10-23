package dev.fetchx.app.storage

import android.content.Context
import dev.fetchx.bridge.PublicDownloadStorage
import java.io.IOException
import java.nio.file.FileVisitResult
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.SimpleFileVisitor
import java.nio.file.attribute.BasicFileAttributes

data class FetchXStorageUsage(
    val downloadsBytes: Long,
    val browserDataBytes: Long,
    val browserCacheBytes: Long,
    val temporaryBytes: Long,
    val databaseBytes: Long,
    val otherAppDataBytes: Long,
    val customSafLocationConfigured: Boolean,
) {
    val totalBytes: Long
        get() = downloadsBytes + browserDataBytes + browserCacheBytes +
            temporaryBytes + databaseBytes + otherAppDataBytes
}

internal data class StorageLocations(
    val filesDir: Path,
    val downloadsDir: Path,
    val stagingDir: Path? = null,
    val publishedDownloadsBytes: Long = 0L,
    val cacheDirs: List<Path>,
    val databaseFile: Path,
    val customSafLocationConfigured: Boolean,
)

class FetchXStorageManager(context: Context) {
    private val context = context.applicationContext

    @Throws(IOException::class, SecurityException::class)
    fun getStorageUsage(): FetchXStorageUsage {
        val filesDir = context.filesDir.toPath()
        val externalFilesDir = context.getExternalFilesDir(null)?.toPath()
        val downloadsDir = externalFilesDir ?: filesDir.resolve("downloads")
        val customSafLocationConfigured = context
            .getSharedPreferences("fetchx_storage", Context.MODE_PRIVATE)
            .contains("custom_download_uri")

        return calculateStorageUsage(
            StorageLocations(
                filesDir = filesDir,
                downloadsDir = downloadsDir,
                stagingDir = filesDir.resolve("downloads/public-staging"),
                publishedDownloadsBytes = PublicDownloadStorage(context).publicDownloadsBytes(),
                cacheDirs = listOfNotNull(
                    context.cacheDir.toPath(),
                    context.externalCacheDir?.toPath(),
                ),
                databaseFile = filesDir.resolve("fetchx.db"),
                customSafLocationConfigured = customSafLocationConfigured,
            ),
        )
    }
}

@Throws(IOException::class)
internal fun calculateStorageUsage(locations: StorageLocations): FetchXStorageUsage {
    val filesDir = locations.filesDir.toAbsolutePath().normalize()
    val downloadsDir = locations.downloadsDir.toAbsolutePath().normalize()
    val stagingDir = locations.stagingDir?.toAbsolutePath()?.normalize()
    val browserProfileDir = filesDir.resolve("mozilla")
    val runtimeDir = filesDir.resolve("runtime")
    val databaseFiles = listOf(
        locations.databaseFile,
        locations.databaseFile.resolveSibling("${locations.databaseFile.fileName}-wal"),
        locations.databaseFile.resolveSibling("${locations.databaseFile.fileName}-shm"),
    )
    val workspaces = findOwnedWorkspaces(downloadsDir)

    val (browserDataBytes, profileCacheBytes) = profileUsage(browserProfileDir)
    val internalExclusions = mutableSetOf(browserProfileDir, runtimeDir)
    internalExclusions.addAll(databaseFiles)
    if (downloadsDir.startsWith(filesDir)) internalExclusions.add(downloadsDir)
    stagingDir?.takeIf { it.startsWith(filesDir) }?.let(internalExclusions::add)

    return FetchXStorageUsage(
        downloadsBytes = directorySize(downloadsDir, workspaces.toSet()) +
            (stagingDir?.let(::directorySize) ?: 0L) +
            locations.publishedDownloadsBytes,
        browserDataBytes = browserDataBytes,
        browserCacheBytes = profileCacheBytes +
            locations.cacheDirs.sumOf { directorySize(it) },
        temporaryBytes = directorySize(runtimeDir) +
            workspaces.sumOf { directorySize(it) },
        databaseBytes = databaseFiles.sumOf { fileSize(it) },
        otherAppDataBytes = directorySize(filesDir, internalExclusions),
        customSafLocationConfigured = locations.customSafLocationConfigured,
    )
}

private fun findOwnedWorkspaces(downloadsDir: Path): List<Path> {
    if (Files.notExists(downloadsDir, LinkOption.NOFOLLOW_LINKS)) return emptyList()
    val workspaces = mutableListOf<Path>()
    Files.newDirectoryStream(downloadsDir).use { entries ->
        for (entry in entries) {
            val name = entry.fileName.toString()
            if (
                name.startsWith(".fetchx-ytdlp-") ||
                name.startsWith(".fetchx-ffmpeg-")
            ) {
                if (Files.isDirectory(entry, LinkOption.NOFOLLOW_LINKS)) workspaces.add(entry)
            }
        }
    }
    return workspaces
}

private fun profileUsage(profileDir: Path): Pair<Long, Long> {
    if (Files.notExists(profileDir, LinkOption.NOFOLLOW_LINKS)) return 0L to 0L

    var profileBytes = 0L
    var cacheBytes = 0L
    Files.walkFileTree(
        profileDir,
        object : SimpleFileVisitor<Path>() {
            override fun preVisitDirectory(
                dir: Path,
                attrs: BasicFileAttributes,
            ): FileVisitResult {
                if (dir != profileDir && dir.fileName.toString() == "cache2") {
                    cacheBytes += directorySize(dir)
                    return FileVisitResult.SKIP_SUBTREE
                }
                return FileVisitResult.CONTINUE
            }

            override fun visitFile(file: Path, attrs: BasicFileAttributes): FileVisitResult {
                if (attrs.isRegularFile) profileBytes += attrs.size()
                return FileVisitResult.CONTINUE
            }
        },
    )
    return profileBytes to cacheBytes
}

@Throws(IOException::class)
private fun directorySize(root: Path, excludedRoots: Set<Path> = emptySet()): Long {
    if (Files.notExists(root, LinkOption.NOFOLLOW_LINKS)) return 0L
    val normalizedRoot = root.toAbsolutePath().normalize()
    val normalizedExclusions = excludedRoots.mapTo(mutableSetOf()) {
        it.toAbsolutePath().normalize()
    }
    var size = 0L
    Files.walkFileTree(
        normalizedRoot,
        object : SimpleFileVisitor<Path>() {
            override fun preVisitDirectory(
                dir: Path,
                attrs: BasicFileAttributes,
            ): FileVisitResult {
                if (dir != normalizedRoot && dir in normalizedExclusions) {
                    return FileVisitResult.SKIP_SUBTREE
                }
                return FileVisitResult.CONTINUE
            }

            override fun visitFile(file: Path, attrs: BasicFileAttributes): FileVisitResult {
                if (file !in normalizedExclusions && attrs.isRegularFile) {
                    size = Math.addExact(size, attrs.size())
                }
                return FileVisitResult.CONTINUE
            }
        },
    )
    return size
}

private fun fileSize(path: Path): Long =
    if (Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) Files.size(path) else 0L
