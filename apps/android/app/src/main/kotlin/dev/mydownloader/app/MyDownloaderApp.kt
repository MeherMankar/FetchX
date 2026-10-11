package dev.mydownloader.app

import android.app.Application

/**
 * Application class for MyDownloader Android.
 *
 * Initialises application-level singletons. In Phase 20B/20C this will
 * initialise the Rust bridge (UniFFI) and CoreServices adapter.
 */
class MyDownloaderApp : Application() {

    override fun onCreate() {
        super.onCreate()
        // Phase 20B: initialise Rust UniFFI bridge here
        // Phase 20C: initialise CoreServices Android adapter here
    }
}
