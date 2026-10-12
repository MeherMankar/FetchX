package dev.mydownloader.app

import android.app.Application
import android.util.Log
import dev.mydownloader.bridge.BridgeInitializer

private const val TAG = "MyDownloaderApp"

/**
 * Application class for MyDownloader Android.
 *
 * Initialises the Rust bridge (UniFFI) on startup and shuts it down
 * when the process terminates.
 *
 * The bridge owns the Rust Tokio runtime and CoreServices. All download
 * operations during the app's lifetime go through [BridgeInitializer.getRepository].
 */
class MyDownloaderApp : Application() {

    override fun onCreate() {
        super.onCreate()
        // Initialize the Rust bridge. This starts the Tokio runtime and CoreServices.
        // The returned repository is the single source of truth for download data.
        try {
            BridgeInitializer.initialize(this)
        } catch (it: Throwable) {
            Log.e(TAG, "Bridge initialization failed — app will degrade gracefully: $it", it)
        }
    }

    override fun onTerminate() {
        // Note: onTerminate() is only called in emulator/test environments.
        // In production, use ProcessLifecycleOwner for clean shutdown.
        // The bridge's own shutdown is idempotent and safe to call here.
        BridgeInitializer.shutdown()
        super.onTerminate()
    }
}
