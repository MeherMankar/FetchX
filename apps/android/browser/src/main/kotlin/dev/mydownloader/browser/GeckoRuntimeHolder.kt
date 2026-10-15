package dev.fetchx.browser

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONObject
import org.mozilla.geckoview.ContentBlocking
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoRuntimeSettings
import org.mozilla.geckoview.WebExtension

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
     * Callback invoked when the background WebExtension detects a video URL.
     * Set by BrowserViewModel; cleared in onCleared.
     */
    @Volatile
    var onVideoDetectedFromExtension: ((DetectedVideo) -> Unit)? = null

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
            installVideoInterceptorExtension(forRuntime)
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

    private fun installVideoInterceptorExtension(rt: GeckoRuntime) {
        val extUri = "resource://android/assets/video_interceptor_ext/"
        val extId = "video-interceptor@fetchx.dev"
        rt.webExtensionController
            .ensureBuiltIn(extUri, extId)
            .accept(
                { extension ->
                    if (extension == null) {
                        Log.e(TAG, "ensureBuiltIn returned null extension")
                        return@accept
                    }
                    // Dispatch to main thread: GeckoView does not guarantee the accept
                    // callback runs on the main thread, and setMessageDelegate must be
                    // called from the main thread to avoid racing with session setup.
                    Handler(Looper.getMainLooper()).post {
                        val delegate = object : WebExtension.MessageDelegate {
                            override fun onMessage(
                                nativeApp: String,
                                message: Any,
                                sender: WebExtension.MessageSender,
                            ): org.mozilla.geckoview.GeckoResult<Any>? {
                                if (message is JSONObject) {
                                    val url = message.optString("url", "")
                                    if (url.isNotBlank()) {
                                        VideoInterceptor.detect(
                                            tabId = "",
                                            url = url,
                                            mimeType = null,
                                            contentLength = null,
                                        )?.let { onVideoDetectedFromExtension?.invoke(it) }
                                    }
                                }
                                return null
                            }
                        }
                        extension.setMessageDelegate(delegate, "browser")
                        Log.i(TAG, "Video interceptor extension registered")
                    }
                },
                { error ->
                    Log.e(TAG, "Failed to register video interceptor extension", error)
                },
            )
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
