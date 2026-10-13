package dev.fetchx.browser

import android.content.Context
import android.util.Log
import org.mozilla.geckoview.ContentBlocking
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoRuntimeSettings

private const val TAG = "GeckoRuntimeHolder"

/**
 * Process-level singleton for [GeckoRuntime].
 *
 * GeckoRuntime must be initialized exactly once per Android process.
 * Creating a second runtime will throw an exception. This object ensures
 * safe, lazy initialization via double-checked locking.
 *
 * Ownership: the Application owns the runtime for the lifetime of the process.
 * All [org.mozilla.geckoview.GeckoSession] instances share this single runtime.
 */
object GeckoRuntimeHolder {
    @Volatile
    private var runtime: GeckoRuntime? = null

    /**
     * Returns the existing [GeckoRuntime] or creates it if not yet initialized.
     * Safe to call from any thread; initialization is synchronized.
     */
    fun getOrCreate(context: Context): GeckoRuntime {
        return runtime ?: synchronized(this) {
            runtime ?: createRuntime(context.applicationContext).also {
                runtime = it
                Log.i(TAG, "GeckoRuntime created successfully")
            }
        }
    }

    private fun createRuntime(context: Context): GeckoRuntime {
        val settings = GeckoRuntimeSettings.Builder()
            .consoleOutput(true)
            .contentBlocking(
                ContentBlocking.Settings.Builder()
                    .antiTracking(TRACKER_CATEGORIES)
                    .build(),
            )
            .build()
        return GeckoRuntime.create(context, settings)
    }

    /**
     * Updates native tracker protection without replacing the runtime or sessions.
     * GeckoView's RuntimeSettings preference commits apply to subsequent requests.
     */
    fun setTrackerBlockingEnabled(enabled: Boolean) {
        runtime?.settings?.contentBlocking?.setAntiTracking(
            trackerAntiTrackingCategories(enabled),
        )
    }

    private val TRACKER_CATEGORIES = trackerAntiTrackingCategories(enabled = true)
}

internal fun trackerAntiTrackingCategories(enabled: Boolean): Int =
    if (enabled) {
        ContentBlocking.AntiTracking.AD or
            ContentBlocking.AntiTracking.ANALYTIC or
            ContentBlocking.AntiTracking.SOCIAL
    } else {
        ContentBlocking.AntiTracking.NONE
    }
