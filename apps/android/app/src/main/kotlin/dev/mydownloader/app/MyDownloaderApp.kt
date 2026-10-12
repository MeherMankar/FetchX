package dev.mydownloader.app

import android.app.Application
import dev.mydownloader.bridge.BridgeInitializer

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
        BridgeInitializer.initialize(this)
    }

    override fun onTerminate() {
        // Note: onTerminate() is only called in emulator/test environments.
        // In production, use ProcessLifecycleOwner for clean shutdown.
        // The bridge's own shutdown is idempotent and safe to call here.
        BridgeInitializer.shutdown()
        super.onTerminate()
    }
}
