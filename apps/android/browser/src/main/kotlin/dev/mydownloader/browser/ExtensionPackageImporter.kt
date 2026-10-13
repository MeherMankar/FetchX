package dev.fetchx.browser

import android.content.ContentResolver
import android.content.Context
import android.net.Uri
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.util.UUID

internal class StagedExtensionPackage internal constructor(
    private val file: File,
) : AutoCloseable {
    private var isClosed = false
    val uri: String = file.toURI().toString()

    internal fun exists(): Boolean = file.exists()

    @Synchronized
    override fun close() {
        if (isClosed) return
        deleteTemporaryFile(file)
        isClosed = true
    }
}

internal class ExtensionPackageImporter(private val context: Context) {
    fun stage(uri: Uri): StagedExtensionPackage {
        validatePickerContentUri(uri.scheme, uri.authority)

        val cacheDirectory = context.cacheDir.canonicalFile
        val stagedFile = File.createTempFile("fetchx-extension-${UUID.randomUUID()}-", ".xpi", cacheDirectory)
        try {
            if (stagedFile.canonicalFile.parentFile != cacheDirectory) {
                throw SecurityException("The staged extension escaped the private cache directory")
            }
            val source = context.contentResolver.openInputStream(uri)
                ?: throw IOException("The selected extension file could not be opened")
            source.use { input ->
                FileOutputStream(stagedFile).use { output ->
                    val copied = copyBounded(input, output, MAX_PACKAGE_BYTES)
                    if (copied == 0L) {
                        throw IOException("The selected extension package is empty")
                    }
                }
            }
            return StagedExtensionPackage(stagedFile)
        } catch (error: IOException) {
            deleteTemporaryFile(stagedFile)
            throw error
        } catch (error: SecurityException) {
            deleteTemporaryFile(stagedFile)
            throw error
        }
    }

    private companion object {
        const val MAX_PACKAGE_BYTES = 25L * 1024L * 1024L
    }
}

internal fun copyBounded(input: InputStream, output: OutputStream, maxBytes: Long): Long {
    require(maxBytes > 0) { "Maximum package size must be positive" }
    val buffer = ByteArray(COPY_BUFFER_BYTES)
    var total = 0L
    while (true) {
        val count = input.read(buffer)
        if (count < 0) break
        if (count == 0) {
            val singleByte = input.read()
            if (singleByte < 0) break
            if (total == maxBytes) {
                throw IOException("The extension package exceeds the size limit")
            }
            output.write(singleByte)
            total += 1
            continue
        }
        if (total + count > maxBytes) {
            throw IOException("The extension package exceeds the size limit")
        }
        output.write(buffer, 0, count)
        total += count
    }
    return total
}

private const val COPY_BUFFER_BYTES = 8 * 1024

internal fun validatePickerContentUri(scheme: String?, authority: String?) {
    if (!scheme.equals(ContentResolver.SCHEME_CONTENT, ignoreCase = true) || authority.isNullOrBlank()) {
        throw SecurityException("Only user-selected content URIs are accepted")
    }
}

private fun deleteTemporaryFile(file: File) {
    if (file.exists() && !file.delete()) {
        throw IOException("Could not remove the staged extension package")
    }
}
