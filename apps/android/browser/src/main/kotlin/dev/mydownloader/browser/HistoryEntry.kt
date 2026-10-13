package dev.fetchx.browser

/**
 * A single browser history entry — one meaningful completed navigation.
 *
 * Intentionally simple: no GeckoView types, no Android imports.
 * [id] is stable for the lifetime of this entry.
 * [title] may be updated after initial creation when the page reports its title.
 * [timestampMs] is epoch-milliseconds at time of navigation completion.
 */
data class HistoryEntry(
    val id: String,          // UUID stable identifier
    val url: String,
    val title: String?,
    val timestampMs: Long,
)
