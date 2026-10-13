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
    private var extensionManager: ExtensionManager? = null
    private var extensionManagerRuntime: GeckoRuntime? = null

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

    /**
     * Returns the runtime-owned extension service. It deliberately outlives browser tabs and
     * BrowserViewModel instances because GeckoView persists installed extensions with the runtime.
     */
    fun getExtensionManager(forRuntime: GeckoRuntime): ExtensionManager = synchronized(this) {
        check(runtime === forRuntime) { "Extension manager must use the process GeckoRuntime" }
        if (extensionManagerRuntime !== forRuntime) {
            extensionManager?.shutdown()
            extensionManager = ExtensionManager(
                GeckoExtensionGatewayImpl(forRuntime.webExtensionController),
            )
            extensionManagerRuntime = forRuntime
        }
        extensionManager ?: error("Extension manager initialization failed")
    }

    /** Releases FetchX-owned delegates and references without uninstalling user extensions. */
    fun shutdownExtensions() {
        synchronized(this) {
            extensionManager?.shutdown()
            extensionManager = null
            extensionManagerRuntime = null
        }
    }

    private fun createRuntime(context: Context): GeckoRuntime {
        val privacy = geckoPrivacyConfiguration(
            trackerBlockingEnabled = true,
            settings = PrivacySettings(),
        )
        val settings = GeckoRuntimeSettings.Builder()
            .consoleOutput(true)
            .allowInsecureConnections(privacy.allowInsecureConnections)
            .contentBlocking(
                ContentBlocking.Settings.Builder()
                    .antiTracking(privacy.antiTrackingCategories)
                    .cookieBehavior(privacy.cookieBehavior)
                    .build(),
            )
            .build()
            .setFingerprintingProtection(privacy.fingerprintingProtectionEnabled)
        return GeckoRuntime.create(context, settings)
    }

    /**
     * Applies browser-wide privacy settings to the shared runtime without replacing
     * it or any sessions. Runtime preference commits affect subsequent requests.
     */
    fun applyPrivacySettings(trackerBlockingEnabled: Boolean, settings: PrivacySettings) {
        val currentRuntime = runtime ?: return
        val privacy = geckoPrivacyConfiguration(trackerBlockingEnabled, settings)
        currentRuntime.settings.contentBlocking
            .setAntiTracking(privacy.antiTrackingCategories)
            .setCookieBehavior(privacy.cookieBehavior)
        currentRuntime.settings
            .setFingerprintingProtection(privacy.fingerprintingProtectionEnabled)
            .setAllowInsecureConnections(privacy.allowInsecureConnections)
    }
}

internal fun trackerAntiTrackingCategories(enabled: Boolean): Int =
    if (enabled) {
        ContentBlocking.AntiTracking.AD or
            ContentBlocking.AntiTracking.ANALYTIC or
            ContentBlocking.AntiTracking.SOCIAL
    } else {
        ContentBlocking.AntiTracking.NONE
    }
