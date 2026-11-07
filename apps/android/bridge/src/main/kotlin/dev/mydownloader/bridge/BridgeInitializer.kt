package dev.fetchx.bridge

import android.content.Context
import android.util.Log
import dev.fetchx.bridge.uniffi.CoreConfig
import dev.fetchx.bridge.uniffi.DownloadService
import dev.fetchx.bridge.uniffi.YtDlpRuntime
import java.io.File

private const val TAG = "FetchXBridge"

/**
 * Manages the lifecycle of the Rust bridge ([DownloadService]).
 *
 * Call [initialize] exactly once from [android.app.Application.onCreate].
 * The Rust core initializes its Tokio runtime and CoreServices at this point.
 *
 * Call [shutdown] when the app process is terminating (e.g., from a
 * ProcessLifecycleOwner observer or onTerminate). Calling [shutdown] more
 * than once is safe.
 *
 * Ownership rules:
 * - [BridgeInitializer] owns the [DownloadService] (and transitively the
 *   Rust Tokio runtime and CoreServices).
 * - [BridgeDownloadRepository] borrows the service via a shared reference.
 * - After [shutdown], any call on the repository will receive a
 *   MobileApiError.Runtime indicating the service is shut down.
 */
object BridgeInitializer {

    @Volatile
    private var service: DownloadService? = null

    @Volatile
    private var repository: BridgeDownloadRepository? = null

    @Volatile
    private var publicDownloadStorage: PublicDownloadStorage? = null

    /** Non-null if the last [initialize] call failed; null on success. */
    @Volatile
    var initError: Throwable? = null

    /**
     * Initializes the Rust bridge and returns the [BridgeDownloadRepository]
     * that wraps it. Idempotent — subsequent calls return the existing instance.
     */
    fun initialize(context: Context, ytDlpRuntime: YtDlpRuntime? = null): BridgeDownloadRepository {
        repository?.let { return it }
        synchronized(this) {
            repository?.let { return it }

            // Step 1: pre-load libjnidispatch.so via Android's native loader BEFORE any JNA
            // class is touched. JNA normally loads it from the classpath JAR, but on Android
            // the .so lives in the APK's lib/ directory — Android's System.loadLibrary can find
            // it there, but JNA's classpath extraction cannot. Loading it first lets JNA's
            // own initialization skip the extraction and use the already-loaded library.
            try {
                System.loadLibrary("jnidispatch")
                Log.i(TAG, "libjnidispatch loaded")
            } catch (e: UnsatisfiedLinkError) {
                Log.w(TAG, "libjnidispatch not pre-loaded (may be bundled inside JNA AAR): $e")
                // Non-fatal — JNA may still find it via its own mechanism
            }

            // Step 2: Load the Rust bridge native library.
            try {
                System.loadLibrary("fetchx_android_bridge")
                Log.i(TAG, "Native library loaded successfully")
            } catch (it: UnsatisfiedLinkError) {
                Log.e(TAG, "Failed to load native library: $it")
                initError = it
                throw RuntimeException("Native library load failed", it)
            }

            val dataDir = context.filesDir
            // Rust's default root remains private for implicit torrent destinations.
            // Public HTTP destinations are staged by BridgeDownloadRepository.
            val dlDir = context.getExternalFilesDir(null)
                ?: File(dataDir, "downloads")
            dlDir.mkdirs()
            val runtimeDir = File(dataDir, "runtime")
            runtimeDir.mkdirs()

            // Use aria2c if already extracted from a previous run (fast path, no I/O).
            // If not yet extracted, trigger background extraction for the next launch.
            // This avoids 69 MB asset decompression during bridge init on first run.
            // NOTE: On Android 10+, SELinux blocks execute_no_trans for binaries in
            // app data directories. aria2c bundling is disabled; NativeTorrent is used instead.
            val aria2Path: String? = null
            Log.i(TAG, "aria2c bundling disabled — using NativeTorrent engine")
            // Background extraction disabled (SELinux blocks execution from /data/data/)
            // Aria2Bundler.extractAsync(context) { ... }

            val config = CoreConfig(
                databasePath = File(dataDir, "fetchx.db").absolutePath,
                downloadsDir = dlDir.absolutePath,
                runtimeDir = runtimeDir.absolutePath,
                ytDlpExecutablePath = null,
                aria2ExecutablePath = aria2Path,
            )

            try {
                val svc = DownloadService(config, ytDlpRuntime)
                service = svc

                val storage = PublicDownloadStorage(context)
                val repo = BridgeDownloadRepository(svc, storage)
                repository = repo
                publicDownloadStorage = storage
                initError = null
                Log.i(TAG, "Bridge initialized successfully")
                return repo
            } catch (e: Exception) {
                initError = e
                Log.e(TAG, "Bridge initialization failed: $e")
                throw e
            }
        }
    }

    /**
     * Returns the initialized repository if the bridge is ready, or null if
     * initialization failed or has not been called. Never throws.
     */
    fun repositoryOrNull(): BridgeDownloadRepository? = repository

    fun publicDownloadStorageOrNull(): PublicDownloadStorage? = publicDownloadStorage

    fun extractDownloadLinks(text: String): List<String> =
        service?.extractDownloadLinks(text) ?: emptyList()

    fun suggestDownloadFilename(url: String): String =
        service?.suggestDownloadFilename(url) ?: "download"

    fun normalizeDownloadUrl(url: String): String =
        service?.normalizeDownloadUrl(url) ?: url.trim()

    fun isDuplicateDownloadUrl(url: String, existingUrls: List<String>): Boolean =
        service?.isDuplicateDownloadUrl(url, existingUrls) ?: false

    fun matchDownloadCategory(filename: String, url: String? = null, pageUrl: String? = null): String? =
        service?.matchDownloadCategory(filename, url, pageUrl)

    /**
     * Returns the initialized repository, or throws if [initialize] was not called first.
     */
    fun getRepository(): BridgeDownloadRepository =
        repository ?: error("BridgeInitializer.initialize() has not been called")

    /**
     * Gracefully shuts down the Rust core. Safe to call multiple times.
     * After this call the repository will return errors for all operations.
     */
    fun shutdown() {
        synchronized(this) {
            service?.shutdown()
            service = null
            repository = null
            publicDownloadStorage = null
        }
    }
}
