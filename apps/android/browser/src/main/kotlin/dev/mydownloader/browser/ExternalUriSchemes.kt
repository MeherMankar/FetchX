package dev.fetchx.browser

/**
 * Single source of truth for the set of external URI schemes that
 * [TabManager] intercepts and hands off to the platform instead of loading
 * them in GeckoView.
 *
 * Both [TabManager.onLoadRequest] and the `MagnetUriRouter` test helper in
 * `TabManagerMagnetTest` call this function, so the tests cannot silently
 * diverge from production routing.
 *
 * `magnet:` is intentionally NOT included here: it is matched earlier in
 * [TabManager.onLoadRequest] and routed to FetchX torrent handling, so it
 * must never be treated as a generic external scheme.
 */
internal fun isExternalScheme(uri: String): Boolean {
    val lower = uri.lowercase()
    return lower.startsWith("mailto:") ||
        lower.startsWith("tel:") ||
        lower.startsWith("sms:") ||
        lower.startsWith("intent:") ||
        lower.startsWith("market:") ||
        lower.startsWith("geo:") ||
        lower.startsWith("whatsapp:")
}
