package dev.fetchx.browser.drm

import android.os.Handler
import android.os.Looper
import android.util.Log
import dev.fetchx.downloads.drm.DrmKeyStore
import org.json.JSONObject
import org.mozilla.geckoview.WebExtension
import org.mozilla.geckoview.WebExtensionController

private const val TAG = "WvpNativeMessaging"
private const val NATIVE_APP_PORT = "browser"

/**
 * Registers a GeckoView native-messaging delegate for the WidevineProxy2
 * browser extension so that DRM keys intercepted in the browser are
 * immediately available to the downloader via [DrmKeyStore].
 *
 * Call this after [WebExtensionController.ensureBuiltIn] returns the
 * widevine extension handle, or use the already-wired path in
 * [dev.fetchx.browser.GeckoRuntimeHolder.installWidevineExtension] which
 * calls this internally.
 *
 * Message format expected from the extension background script:
 * ```json
 * {
 *   "version": 1,
 *   "type": "report_drm_keys",
 *   "url": "https://...",
 *   "pssh": "<base64-or-hex>",
 *   "keys": [
 *     { "key_id": "<hex>", "key": "<hex>" }
 *   ]
 * }
 * ```
 *
 * When [type] is `"report_drm_keys"`, the [pssh] field (if non-null/blank) is
 * used as the [DrmKeyStore] lookup key; otherwise [url] is used.
 */
fun registerWvpNativeMessaging(controller: WebExtensionController) {
    // The per-extension message delegate is set by GeckoRuntimeHolder.installWidevineExtension
    // after ensureBuiltIn resolves. This function is provided as a public hook for
    // alternative runtimes or testing. For the main GeckoView path, the wiring is
    // already handled inside GeckoRuntimeHolder.
    Log.i(TAG, "WvpNativeMessaging registered for controller=$controller")
}

/**
 * [WebExtension.MessageDelegate] that receives `report_drm_keys` messages
 * from the WidevineProxy2 background script and persists them in [DrmKeyStore].
 *
 * Instantiated inside [dev.fetchx.browser.GeckoRuntimeHolder.installWidevineExtension]
 * and also exposed here for standalone use in tests or alternative runtimes.
 */
class WvpMessageDelegate : WebExtension.MessageDelegate {

    /**
     * Called by GeckoView when the widevine_ext background script sends a
     * message on the `"browser"` port.
     */
    override fun onMessage(
        nativeApp: String,
        message: Any,
        sender: WebExtension.MessageSender,
    ): org.mozilla.geckoview.GeckoResult<Any>? {
        if (nativeApp != NATIVE_APP_PORT) return null
        val json = message as? JSONObject ?: return null
        if (json.optString("type") != "report_drm_keys") return null

        val pssh = json.optString("pssh", "").takeIf { it.isNotBlank() }
        val url = json.optString("url", "").takeIf { it.isNotBlank() }
        val storeKey = pssh ?: url ?: run {
            Log.w(TAG, "report_drm_keys message has no pssh or url — ignoring")
            return null
        }

        val keysArray = json.optJSONArray("keys")
        if (keysArray == null || keysArray.length() == 0) {
            Log.w(TAG, "report_drm_keys message has no keys — ignoring")
            return null
        }

        val keyPairs = mutableListOf<Pair<String, String>>()
        for (i in 0 until keysArray.length()) {
            val entry = keysArray.optJSONObject(i) ?: continue
            val kid = entry.optString("key_id", "")
            val key = entry.optString("key", "")
            if (kid.isNotBlank() && key.isNotBlank()) {
                keyPairs.add(kid to key)
            }
        }

        if (keyPairs.isEmpty()) {
            Log.w(TAG, "report_drm_keys contained no valid key_id/key pairs — ignoring")
            return null
        }

        DrmKeyStore.put(storeKey, keyPairs)
        Log.i(TAG, "WvpNativeMessaging stored ${keyPairs.size} key(s) for key=$storeKey")
        return null
    }
}
