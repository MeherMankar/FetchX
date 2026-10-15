package dev.fetchx.app

import android.app.Application
import android.content.Intent
import android.os.Build
import android.util.Log
import dev.fetchx.app.service.DownloadForegroundService
import dev.fetchx.bridge.BridgeInitializer
import dev.fetchx.browser.GeckoRuntimeHolder
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val TAG = "FetchXApp"

/**
 * Application class for FetchX Android.
 *
 * Initialises the Rust bridge (UniFFI) on startup and shuts it down
 * when the process terminates.
 *
 * The bridge owns the Rust Tokio runtime and CoreServices. All download
 * operations during the app's lifetime go through [BridgeInitializer.getRepository].
 */
class FetchXApp : Application() {

    private val appScope = CoroutineScope(Dispatchers.IO)

    override fun onCreate() {
        super.onCreate()
        // Initialize the Rust bridge. This starts the Tokio runtime and CoreServices.
        // The returned repository is the single source of truth for download data.
        val ytDlpRuntime = try {
            AndroidYtDlpRuntime.start(this)
        } catch (it: Throwable) {
            Log.e(TAG, "Embedded yt-dlp runtime unavailable: $it", it)
            null
        }
        try {
            BridgeInitializer.initialize(this, ytDlpRuntime)
        } catch (it: Throwable) {
            Log.e(TAG, "Bridge initialization failed — app will degrade gracefully: $it", it)
        }
        startDownloadServiceIfNeeded()
    }

    fun startDownloadServiceIfNeeded() {
        val repo = BridgeInitializer.repositoryOrNull() ?: return
        appScope.launch {
            val shouldStart = withContext(Dispatchers.IO) { repo.hasRelevantDownloads() }
            if (!shouldStart) {
                Log.i(TAG, "No active or queued downloads — skipping foreground service start")
                return@launch
            }
            val intent = Intent(this@FetchXApp, DownloadForegroundService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                startForegroundService(intent)
            } else {
                startService(intent)
            }
        }
    }

    override fun onTerminate() {
        // Note: onTerminate() is only called in emulator/test environments.
        // In production, use ProcessLifecycleOwner for clean shutdown.
        // The bridge's own shutdown is idempotent and safe to call here.
        GeckoRuntimeHolder.shutdownExtensions()
        BridgeInitializer.shutdown()
        super.onTerminate()
    }
}
