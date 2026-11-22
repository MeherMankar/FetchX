package dev.fetchx.app

import android.content.ClipboardManager
import android.content.Context
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Polls the system clipboard every [pollIntervalMs] milliseconds while active and emits
 * any newly detected HTTP/HTTPS URL via [detectedUrl].
 *
 * Emission only happens when the clipboard content changes **and** the new content
 * contains a URL that differs from the previously seen value. The flow resets to null
 * after each emission so callers can distinguish successive detections.
 *
 * Usage:
 * ```
 * val monitor = ClipboardMonitor(context)
 * // In a coroutine scope tied to the foreground lifecycle:
 * monitor.start()          // begin polling
 * monitor.detectedUrl.collect { url -> if (url != null) { /* handle */ } }
 * monitor.stop()           // stop polling (e.g. onPause)
 * ```
 */
class ClipboardMonitor(
    context: Context,
    private val pollIntervalMs: Long = 2_000L,
) {
    private val clipboard =
        context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    private val _detectedUrl = MutableStateFlow<String?>(null)

    /** Emits the last detected HTTP/HTTPS URL, or null between detections. */
    val detectedUrl: StateFlow<String?> = _detectedUrl.asStateFlow()

    @Volatile private var active = false
    private var lastSeen: String? = null

    /** Start polling. Must be called from a coroutine; suspends until [stop] is called. */
    suspend fun start() {
        active = true
        while (active) {
            val current = readClipboardUrl()
            if (current != null && current != lastSeen) {
                lastSeen = current
                _detectedUrl.value = current
            }
            delay(pollIntervalMs)
        }
    }

    /** Stop the polling loop started by [start]. */
    fun stop() {
        active = false
    }

    /** Reset the emitted URL (call after the caller has consumed the value). */
    fun consume() {
        _detectedUrl.value = null
    }

    // ── Internal helpers ───────────────────────────────────────────────────

    private fun readClipboardUrl(): String? {
        return try {
            if (!clipboard.hasPrimaryClip()) return null
            val item = clipboard.primaryClip?.getItemAt(0) ?: return null
            val text = item.coerceToText(null)?.toString()?.trim() ?: return null
            extractHttpUrl(text)
        } catch (_: Exception) {
            null
        }
    }

    private fun extractHttpUrl(text: String): String? =
        Regex("""https?://[^\s<>"']+""", RegexOption.IGNORE_CASE)
            .find(text.replace("&amp;", "&"))
            ?.value
            ?.trimEnd('.', ',', ';', ':', '!', '?', ')', ']', '}')
}
