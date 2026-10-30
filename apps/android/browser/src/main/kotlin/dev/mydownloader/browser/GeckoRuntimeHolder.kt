package dev.fetchx.browser

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import dev.fetchx.bridge.BridgeInitializer
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.json.JSONObject
import org.mozilla.geckoview.ContentBlocking
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoRuntimeSettings
import org.mozilla.geckoview.GeckoSession
import org.mozilla.geckoview.WebExtension

private const val TAG = "GeckoRuntimeHolder"
private const val FETCHX_REMOTE_EXTENSION_ID = "fetchx-remote@fetchx.dev"
private const val FETCHX_RPC_NATIVE_APP = "browser"
private const val FETCHX_REMOTE_POPUP_PATH = "popup.html"
private const val MAX_BROWSER_RPC_BYTES = 1_048_576

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
    private val browserRpcScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val fetchxRemoteActionInvoker = FetchxRemoteActionInvoker()
    @Volatile
    private var fetchxRemotePopupSession: GeckoSession? = null
    @Volatile
    private var onFetchxRemotePopupSessionChanged: ((GeckoSession?) -> Unit)? = null

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
            installFetchxRemoteExtension(forRuntime)
        }
        extensionManager ?: error("Extension manager initialization failed")
    }

    /** Releases FetchX-owned delegates and references without uninstalling user extensions. */
    fun shutdownExtensions() {
        synchronized(this) {
            extensionManager?.shutdown()
            extensionManager = null
            extensionManagerRuntime = null
            fetchxRemoteActionInvoker.clear()
            closeFetchxRemotePopup()
        }
    }

    fun setFetchxRemotePopupSessionListener(listener: ((GeckoSession?) -> Unit)?) {
        onFetchxRemotePopupSessionChanged = listener
        listener?.invoke(fetchxRemotePopupSession)
    }

    fun clickFetchxRemoteAction(): Boolean {
        val invoked = fetchxRemoteActionInvoker.invoke()
        Log.i(TAG, "FetchX Remote action requested; registered=$invoked")
        return invoked
    }

    fun closeFetchxRemotePopup() {
        val popupSession = fetchxRemotePopupSession ?: return
        fetchxRemotePopupSession = null
        popupSession.close()
        onFetchxRemotePopupSessionChanged?.invoke(null)
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
                                    val filename = message.optString("filename", "").takeIf { it.isNotBlank() }
                                    if (url.isNotBlank()) {
                                        val detected = VideoInterceptor.detect(
                                            tabId = "",
                                            url = url,
                                            mimeType = null,
                                            contentLength = null,
                                        )?.let { d ->
                                            // Use filename from JS (has quality label e.g. "720p.m3u8")
                                            if (filename != null) d.copy(suggestedFilename = filename) else d
                                        }
                                        detected?.let { onVideoDetectedFromExtension?.invoke(it) }
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

    private fun installFetchxRemoteExtension(rt: GeckoRuntime) {
        val extUri = "resource://android/assets/fetchx_remote_ext/"
        rt.webExtensionController
            .ensureBuiltIn(extUri, FETCHX_REMOTE_EXTENSION_ID)
            .accept(
                { extension ->
                    if (extension == null) {
                        Log.e(TAG, "ensureBuiltIn returned null FetchX RPC extension")
                        return@accept
                    }
                    Handler(Looper.getMainLooper()).post {
                        if (!isTrustedFetchxRemoteExtension(extension.id)) {
                            Log.e(TAG, "FetchX RPC extension ID did not match the trusted ID")
                            return@post
                        }
                        extension.setActionDelegate(
                            object : WebExtension.ActionDelegate {
                                override fun onBrowserAction(
                                    extension: WebExtension,
                                    session: GeckoSession?,
                                    action: WebExtension.Action,
                                ) {
                                    if (fetchxRemoteActionInvoker.register(
                                            extension.id,
                                            clickAction = { action.click() },
                                            openPopup = { openFetchxRemotePopup(rt, extension) },
                                        )
                                    ) {
                                        Log.i(
                                            TAG,
                                            "FetchX Remote browser action registered (${extension.metaData.version})",
                                        )
                                    }
                                }

                                override fun onTogglePopup(
                                    extension: WebExtension,
                                    action: WebExtension.Action,
                                ): org.mozilla.geckoview.GeckoResult<GeckoSession>? {
                                    Log.i(TAG, "FetchX Remote popup toggle requested")
                                    return openFetchxRemotePopup(rt, extension)
                                }

                                override fun onOpenPopup(
                                    extension: WebExtension,
                                    action: WebExtension.Action,
                                ): org.mozilla.geckoview.GeckoResult<GeckoSession>? =
                                    openFetchxRemotePopup(rt, extension)
                            },
                        )
                        extension.setMessageDelegate(
                            createFetchxRemoteMessageDelegate(extension),
                            FETCHX_RPC_NATIVE_APP,
                        )
                        Log.i(
                            TAG,
                            "FetchX Remote native-message delegate registered; " +
                                "permissions=${extension.metaData.requiredPermissions.joinToString()}",
                        )
                    }
                },
                { error ->
                    Log.e(TAG, "Failed to register FetchX RPC extension", error)
                },
            )
    }

    private fun openFetchxRemotePopup(
        runtime: GeckoRuntime,
        extension: WebExtension,
    ): org.mozilla.geckoview.GeckoResult<GeckoSession>? {
        if (!isTrustedFetchxRemoteExtension(extension.id)) return null

        fetchxRemotePopupSession?.let { return org.mozilla.geckoview.GeckoResult.fromValue(it) }
        val popupUrl = extension.metaData.baseUrl + FETCHX_REMOTE_POPUP_PATH
        if (!popupUrl.startsWith("moz-extension://")) {
            Log.e(TAG, "FetchX Remote extension has an invalid popup URL")
            return null
        }
        val popupSession = GeckoSession().also {
            it.open(runtime)
            it.loadUri(popupUrl)
        }
        fetchxRemotePopupSession = popupSession
        onFetchxRemotePopupSessionChanged?.invoke(popupSession)
        Log.i(TAG, "Opening FetchX Remote extension popup")
        return org.mozilla.geckoview.GeckoResult.fromValue(popupSession)
    }

    private fun createFetchxRemoteMessageDelegate(
        extension: WebExtension,
    ): WebExtension.MessageDelegate =
        object : WebExtension.MessageDelegate {
            override fun onMessage(
                nativeApp: String,
                message: Any,
                sender: WebExtension.MessageSender,
            ): org.mozilla.geckoview.GeckoResult<Any>? {
                Log.i(TAG, "Received FetchX RPC message for app=$nativeApp")
                if (!isTrustedFetchxRemoteNativeMessage(extension.id, nativeApp)) return null
                val request = message as? JSONObject
                    ?: return org.mozilla.geckoview.GeckoResult.fromValue(rpcErrorPayload("invalid request"))
                val encodedRequest = request.toString()
                if (encodedRequest.toByteArray(Charsets.UTF_8).size > MAX_BROWSER_RPC_BYTES) {
                    return org.mozilla.geckoview.GeckoResult.fromValue(rpcErrorPayload("request is too large"))
                }
                val result = org.mozilla.geckoview.GeckoResult<Any>()
                browserRpcScope.launch {
                    try {
                        val repository = BridgeInitializer.repositoryOrNull()
                            ?: throw IllegalStateException("FetchX core is unavailable")
                        Log.i(TAG, "Dispatching FetchX RPC to Android bridge")
                        val responseJson = repository.dispatchBrowserRpc(encodedRequest)
                        Handler(Looper.getMainLooper()).post {
                            result.complete(responseJson)
                        }
                        Log.i(TAG, "Completed FetchX RPC through Android bridge")
                    } catch (cancelled: CancellationException) {
                        throw cancelled
                    } catch (error: Exception) {
                        Log.e(TAG, "Could not dispatch trusted browser RPC", error)
                        Handler(Looper.getMainLooper()).post {
                            result.complete(rpcErrorPayload("FetchX RPC is unavailable"))
                        }
                    }
                }
                return result
            }
        }

    private fun rpcErrorPayload(message: String): String =
        JSONObject()
            .put("jsonrpc", "2.0")
            .put("id", JSONObject.NULL)
            .put(
                "error",
                JSONObject()
                    .put("code", -32010)
                    .put("message", message),
            )
            .toString()

    private fun createRuntime(context: Context): GeckoRuntime {
        val privacy = geckoPrivacyConfiguration(
            trackerBlockingEnabled = true,
            settings = PrivacySettings(),
        )
        val settings = GeckoRuntimeSettings.Builder()
            // ── Performance ───────────────────────────────────────────────
            // Disable console log forwarding to avoid per-page JS log overhead
            .consoleOutput(false)
            // Enable JavaScript (JIT is on by default when JS is enabled)
            .javaScriptEnabled(true)
            // ── Security ──────────────────────────────────────────────────
            .allowInsecureConnections(privacy.allowInsecureConnections)
            .contentBlocking(
                ContentBlocking.Settings.Builder()
                    .antiTracking(privacy.antiTrackingCategories)
                    .cookieBehavior(privacy.cookieBehavior)
                    .build(),
            )
            .build()
            .setFingerprintingProtection(privacy.fingerprintingProtectionEnabled)
            // Match system dark/light mode — avoids a style-recalculation flash
            .also { s -> s.setPreferredColorScheme(GeckoRuntimeSettings.COLOR_SCHEME_SYSTEM) }
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

internal fun isTrustedFetchxRemoteExtension(extensionId: String): Boolean =
    extensionId == FETCHX_REMOTE_EXTENSION_ID

internal fun isTrustedFetchxRemoteNativeMessage(extensionId: String, nativeApp: String): Boolean =
    isTrustedFetchxRemoteExtension(extensionId) && nativeApp == FETCHX_RPC_NATIVE_APP

internal class FetchxRemoteActionInvoker {
    @Volatile
    private var actions: Pair<() -> Unit, () -> Unit>? = null

    fun register(
        extensionId: String,
        clickAction: () -> Unit,
        openPopup: () -> Unit,
    ): Boolean {
        if (!isTrustedFetchxRemoteExtension(extensionId)) return false
        actions = clickAction to openPopup
        return true
    }

    fun invoke(): Boolean {
        val (clickAction, openPopup) = actions ?: return false
        clickAction()
        openPopup()
        return true
    }

    fun clear() {
        actions = null
    }
}
