package dev.mydownloader.bridge

import android.content.Context
import android.util.Log
import dev.mydownloader.bridge.uniffi.CoreConfig
import dev.mydownloader.bridge.uniffi.DownloadService
import java.io.File

private const val TAG = "MyDownloaderBridge"

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

    /** Non-null if the last [initialize] call failed; null on success. */
    @Volatile
    var initError: Throwable? = null

    /**
     * Initializes the Rust bridge and returns the [BridgeDownloadRepository]
     * that wraps it. Idempotent — subsequent calls return the existing instance.
     */
    fun initialize(context: Context): BridgeDownloadRepository {
        repository?.let { return it }
        synchronized(this) {
            repository?.let { return it }

            // Load the native library. This must be done before constructing DownloadService.
            try {
                System.loadLibrary("mydownloader_android_bridge")
                Log.i(TAG, "Native library loaded successfully")
            } catch (it: UnsatisfiedLinkError) {
                Log.e(TAG, "Failed to load native library: $it")
                initError = it
                throw RuntimeException("Native library load failed", it)
            }

            val dataDir = context.filesDir
            val dlDir = context.getExternalFilesDir(null)
                ?: File(dataDir, "downloads")
            dlDir.mkdirs()
            val runtimeDir = File(dataDir, "runtime")
            runtimeDir.mkdirs()

            val config = CoreConfig(
                databasePath = File(dataDir, "mydownloader.db").absolutePath,
                downloadsDir = dlDir.absolutePath,
                runtimeDir = runtimeDir.absolutePath,
            )

            try {
                val svc = DownloadService(config)
                service = svc

                val repo = BridgeDownloadRepository(svc)
                repository = repo
                Log.i(TAG, "Bridge initialized successfully")
                return repo
            } catch (e: Exception) {
                Log.e(TAG, "Bridge initialization failed: $e")
                throw e
            }
        }
    }

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
        }
    }
}
