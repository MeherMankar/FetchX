package dev.mydownloader.bridge

import android.content.Context
import dev.mydownloader.bridge.uniffi.CoreConfig
import dev.mydownloader.bridge.uniffi.DownloadService
import java.io.File

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

    /**
     * Initializes the Rust bridge and returns the [BridgeDownloadRepository]
     * that wraps it. Idempotent — subsequent calls return the existing instance.
     */
    fun initialize(context: Context): BridgeDownloadRepository {
        repository?.let { return it }
        synchronized(this) {
            repository?.let { return it }

            // Load the native library. This must be done before constructing DownloadService.
            System.loadLibrary("mydownloader_android_bridge")

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

            val svc = DownloadService(config)
            service = svc

            val repo = BridgeDownloadRepository(svc)
            repository = repo
            return repo
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
