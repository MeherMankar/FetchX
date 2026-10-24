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
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import dev.fetchx.core.download.DownloadState
import kotlinx.coroutines.flow.collect
import java.io.File
import java.io.IOException

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
    private var ownsDownloadRuntime = false

    override fun onCreate() {
        super.onCreate()
        val processName = currentProcessName()
        if (!isMainApplicationProcess(processName, packageName)) {
            Log.i(TAG, "Skipping download runtime initialization in process ${processName ?: "unknown"}")
            return
        }
        ownsDownloadRuntime = true
        Log.i(TAG, "Application.onCreate started")
        // Initialize the Rust bridge. This starts the Tokio runtime and CoreServices.
        // The returned repository is the single source of truth for download data.
        val ytDlpRuntime = try {
            Log.i(TAG, "Starting yt-dlp runtime")
            AndroidYtDlpRuntime.start(this)
        } catch (it: Throwable) {
            Log.e(TAG, "Embedded yt-dlp runtime unavailable: $it", it)
            null
        }
        Log.i(TAG, "Calling BridgeInitializer.initialize")
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
            try {
                repo.awaitInitialSnapshot()
                repo.observeDownloads().collect { downloads ->
                    val shouldStart = downloads.any {
                        it.state is DownloadState.Active
                            || it.state is DownloadState.Queued
                            || it.state is DownloadState.MetadataAcquiring
                            || it.state is DownloadState.Recovering
                            || it.state is DownloadState.Seeding
                    }
                    if (shouldStart) {
                        val intent = Intent(this@FetchXApp, DownloadForegroundService::class.java)
                        try {
                            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                                startForegroundService(intent)
                            } else {
                                startService(intent)
                            }
                        } catch (error: RuntimeException) {
                            Log.e(TAG, "Could not start download foreground service", error)
                        }
                    }
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                Log.e(TAG, "Could not observe downloads for foreground service startup", error)
            }
        }
    }

    override fun onTerminate() {
        // Note: onTerminate() is only called in emulator/test environments.
        // In production, use ProcessLifecycleOwner for clean shutdown.
        // The bridge's own shutdown is idempotent and safe to call here.
        if (ownsDownloadRuntime) {
            GeckoRuntimeHolder.shutdownExtensions()
            BridgeInitializer.shutdown()
        }
        super.onTerminate()
    }
}

internal fun isMainApplicationProcess(processName: String?, packageName: String): Boolean =
    processName == packageName

private fun currentProcessName(): String? =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
        Application.getProcessName()
    } else {
        try {
            File("/proc/self/cmdline")
                .readText(Charsets.UTF_8)
                .substringBefore('\u0000')
                .takeIf(String::isNotBlank)
        } catch (error: IOException) {
            Log.e(TAG, "Could not determine current process name", error)
            null
        }
    }
