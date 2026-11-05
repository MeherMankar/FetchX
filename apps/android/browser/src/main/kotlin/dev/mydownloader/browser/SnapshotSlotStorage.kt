package dev.fetchx.browser

import android.util.AtomicFile
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException

internal interface SnapshotSlotStorage {
    val identity: String

    @Throws(IOException::class)
    fun read(slot: Int): ByteArray?

    @Throws(IOException::class)
    fun writeAtomically(slot: Int, bytes: ByteArray)
}

internal class AtomicFileSnapshotSlotStorage(private val directory: File) : SnapshotSlotStorage {
    override val identity: String = directory.absoluteFile.toPath().normalize().toString()

    override fun read(slot: Int): ByteArray? {
        require(slot in 0..1)
        val baseFile = baseFile(slot)
        if (!baseFile.exists() && !File(baseFile.path + ".bak").exists()) return null

        return atomicFile(slot).openRead().use(::readBounded)
    }

    override fun writeAtomically(slot: Int, bytes: ByteArray) {
        require(slot in 0..1)
        if (bytes.size > MAX_SNAPSHOT_BYTES) {
            throw IOException("Browser snapshot exceeds the size limit")
        }
        if (!directory.exists() && !directory.mkdirs()) {
            throw IOException("Could not create browser snapshot directory")
        }

        val atomicFile = atomicFile(slot)
        var stream: FileOutputStream? = null
        try {
            stream = atomicFile.startWrite()
            stream.write(bytes)
            atomicFile.finishWrite(stream)
            stream = null
        } catch (error: IOException) {
            stream?.let(atomicFile::failWrite)
            throw error
        } catch (error: RuntimeException) {
            stream?.let(atomicFile::failWrite)
            throw error
        }
    }

    private fun readBounded(input: FileInputStream): ByteArray {
        val output = ByteArrayOutputStream()
        val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
        while (true) {
            val count = input.read(buffer)
            if (count < 0) break
            if (output.size() + count > MAX_SNAPSHOT_BYTES) {
                throw IOException("Browser snapshot exceeds the size limit")
            }
            output.write(buffer, 0, count)
        }
        return output.toByteArray()
    }

    private fun baseFile(slot: Int): File = File(directory, "$SNAPSHOT_FILE_PREFIX$slot")

    private fun atomicFile(slot: Int): AtomicFile = AtomicFile(baseFile(slot))

    companion object {
        private const val SNAPSHOT_FILE_PREFIX = "snapshot-"
        private const val MAX_SNAPSHOT_BYTES = 1_048_576
    }
}
