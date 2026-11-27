package dev.fetchx.app.settings

import android.content.Context

/**
 * Typed accessor for all app-level SharedPreferences keys.
 *
 * SharedPreferences file: "fetchx_app_settings"
 */
object AppPreferences {

    private const val PREFS_FILE = "fetchx_app_settings"

    // Keys
    private const val KEY_SUBTITLE_ENABLED = "subtitle_enabled"
    private const val KEY_SUBTITLE_LANGS = "subtitle_langs"
    private const val KEY_ACCENT_COLOR_ARGB = "accent_color_argb"
    private const val KEY_POST_QUEUE_ACTION = "post_queue_action"
    private const val KEY_WEBHOOK_ENABLED = "webhook_enabled"
    private const val KEY_WEBHOOK_URL = "webhook_url"
    private const val KEY_WEBHOOK_ON_COMPLETE = "webhook_on_complete"
    private const val KEY_WEBHOOK_ON_FAIL = "webhook_on_fail"
    private const val KEY_WEBHOOK_SECRET = "webhook_secret"
    private const val KEY_UPNP_ENABLED = "upnp_enabled"

    // Post-queue action values
    const val POST_QUEUE_NOTHING = "nothing"
    const val POST_QUEUE_SCREEN_OFF = "screen_off"

    // Accent color sentinel (means "use system default")
    const val ACCENT_DEFAULT: Long = -1L

    private fun prefs(context: Context) =
        context.getSharedPreferences(PREFS_FILE, Context.MODE_PRIVATE)

    // ── Subtitle ─────────────────────────────────────────────────────────

    fun isSubtitleEnabled(context: Context): Boolean =
        prefs(context).getBoolean(KEY_SUBTITLE_ENABLED, false)

    fun setSubtitleEnabled(context: Context, enabled: Boolean) {
        prefs(context).edit().putBoolean(KEY_SUBTITLE_ENABLED, enabled).apply()
    }

    fun getSubtitleLangs(context: Context): String =
        prefs(context).getString(KEY_SUBTITLE_LANGS, "en") ?: "en"

    fun setSubtitleLangs(context: Context, langs: String) {
        prefs(context).edit().putString(KEY_SUBTITLE_LANGS, langs).apply()
    }

    // ── Accent color ──────────────────────────────────────────────────────

    fun getAccentColor(context: Context): Long =
        prefs(context).getLong(KEY_ACCENT_COLOR_ARGB, ACCENT_DEFAULT)

    fun setAccentColor(context: Context, argb: Long) {
        prefs(context).edit().putLong(KEY_ACCENT_COLOR_ARGB, argb).apply()
    }

    // ── Post-queue action ─────────────────────────────────────────────────

    fun getPostQueueAction(context: Context): String =
        prefs(context).getString(KEY_POST_QUEUE_ACTION, POST_QUEUE_NOTHING) ?: POST_QUEUE_NOTHING

    fun setPostQueueAction(context: Context, action: String) {
        prefs(context).edit().putString(KEY_POST_QUEUE_ACTION, action).apply()
    }

    // ── Webhook ───────────────────────────────────────────────────────────

    fun isWebhookEnabled(context: Context): Boolean =
        prefs(context).getBoolean(KEY_WEBHOOK_ENABLED, false)

    fun setWebhookEnabled(context: Context, enabled: Boolean) {
        prefs(context).edit().putBoolean(KEY_WEBHOOK_ENABLED, enabled).apply()
    }

    fun getWebhookUrl(context: Context): String =
        prefs(context).getString(KEY_WEBHOOK_URL, "") ?: ""

    fun setWebhookUrl(context: Context, url: String) {
        prefs(context).edit().putString(KEY_WEBHOOK_URL, url).apply()
    }

    fun isWebhookOnComplete(context: Context): Boolean =
        prefs(context).getBoolean(KEY_WEBHOOK_ON_COMPLETE, true)

    fun setWebhookOnComplete(context: Context, enabled: Boolean) {
        prefs(context).edit().putBoolean(KEY_WEBHOOK_ON_COMPLETE, enabled).apply()
    }

    fun isWebhookOnFail(context: Context): Boolean =
        prefs(context).getBoolean(KEY_WEBHOOK_ON_FAIL, false)

    fun setWebhookOnFail(context: Context, enabled: Boolean) {
        prefs(context).edit().putBoolean(KEY_WEBHOOK_ON_FAIL, enabled).apply()
    }

    fun getWebhookSecret(context: Context): String =
        prefs(context).getString(KEY_WEBHOOK_SECRET, "") ?: ""

    fun setWebhookSecret(context: Context, secret: String) {
        prefs(context).edit().putString(KEY_WEBHOOK_SECRET, secret).apply()
    }

    // ── UPnP ──────────────────────────────────────────────────────────────

    fun isUpnpEnabled(context: Context): Boolean =
        prefs(context).getBoolean(KEY_UPNP_ENABLED, false)

    fun setUpnpEnabled(context: Context, enabled: Boolean) {
        prefs(context).edit().putBoolean(KEY_UPNP_ENABLED, enabled).apply()
    }
}
