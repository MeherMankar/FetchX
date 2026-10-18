package dev.fetchx.bridge

import android.content.Context
import android.os.Build
import android.util.Log
import java.io.File

private const val TAG = "FetchXAria2"

/**
 * Extracts the bundled aria2c binary from APK assets into the app's
 * private files directory and returns its executable path.
 *
 * Strategy:
 * 1. If the extracted file already exists and is the right size, return immediately (fast path).
 * 2. Otherwise extract in the background on a separate thread.
 *    The caller gets null on first run and a path on subsequent runs.
 *
 * Size verification uses a companion `.size` marker file written after successful
 * extraction, avoiding the unreliable [java.io.InputStream.available] on compressed assets.
 */
object Aria2Bundler {

    private const val ASSET_DIR = "bin"
    private const val BINARY_NAME = "aria2c"
    // Known size of aria2c-arm64-v8a from devgianlu/aria2-android v1.37.0
    private val KNOWN_SIZES = mapOf(
        "arm64-v8a" to 69107592L,
        "x86_64"    to 68510008L,
        "armeabi-v7a" to 62578748L,
    )

    /**
     * Returns the path to the extracted aria2c binary if it is already ready,
     * or null if it hasn't been extracted yet (first run).
     *
     * Call [extractAsync] to trigger background extraction on first run.
     */
    fun pathIfReady(context: Context): String? {
        val abi = primaryAbi()
        val outFile = File(context.filesDir, "bin/$BINARY_NAME")
        val markerFile = File(context.filesDir, "bin/$BINARY_NAME.ready")
        return if (outFile.exists() && outFile.canExecute() && markerFile.exists()) {
            Log.i(TAG, "aria2c ready at ${outFile.absolutePath} (${outFile.length()} bytes)")
            outFile.absolutePath
        } else {
            null
        }
    }

    /**
     * Extracts the aria2c binary in a background thread. Safe to call repeatedly —
     * does nothing if already extracted. Calls [onReady] on the calling thread's
     * executor when extraction completes successfully.
     */
    fun extractAsync(context: Context, onReady: (String) -> Unit) {
        val abi = primaryAbi()
        val assetName = "$ASSET_DIR/$BINARY_NAME-$abi"
        val outFile = File(context.filesDir, "bin/$BINARY_NAME")
        val markerFile = File(context.filesDir, "bin/$BINARY_NAME.ready")

        // Already extracted — nothing to do.
        if (outFile.exists() && outFile.canExecute() && markerFile.exists()) {
            Log.i(TAG, "aria2c already extracted, skipping")
            return
        }

        Thread({
            try {
                val assetManager = context.assets
                val availableAssets = assetManager.list(ASSET_DIR) ?: emptyArray()
                if ("$BINARY_NAME-$abi" !in availableAssets) {
                    Log.w(TAG, "No bundled aria2c for ABI $abi (available: ${availableAssets.toList()})")
                    return@Thread
                }

                outFile.parentFile?.mkdirs()
                markerFile.delete() // clear any stale marker

                Log.i(TAG, "Extracting aria2c ($abi) from assets…")
                assetManager.open(assetName).use { input ->
                    outFile.outputStream().use { output ->
                        input.copyTo(output, bufferSize = 256 * 1024)
                    }
                }

                val extracted = outFile.length()
                val expectedSize = KNOWN_SIZES[abi]
                if (extracted == 0L) {
                    Log.e(TAG, "Extraction produced zero bytes — asset may be compressed in APK")
                    return@Thread
                }
                if (expectedSize != null && extracted != expectedSize) {
                    Log.e(TAG, "Extracted size $extracted != expected $expectedSize — corrupt?")
                    outFile.delete()
                    return@Thread
                }

                outFile.setExecutable(true, true)
                markerFile.writeText("ok") // write marker only after successful extraction
                Log.i(TAG, "aria2c extracted: ${outFile.absolutePath} ($extracted bytes)")
                onReady(outFile.absolutePath)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to extract aria2c: $e")
            }
        }, "aria2c-extractor").start()
    }

    /** Returns the primary ABI for this device. */
    fun primaryAbi(): String =
        if (Build.SUPPORTED_ABIS.isNotEmpty()) Build.SUPPORTED_ABIS[0] else "arm64-v8a"
}
