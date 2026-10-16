package dev.fetchx.browser

import java.util.UUID

/**
 * Per-tab in-memory navigation history.
 *
 * Rules:
 * - History records completed navigations only (not onPageStart events).
 * - Error navigations (onLoadError) must NOT add an entry.
 * - Consecutive duplicate URLs are not added again (avoids anchor / pushState spam).
 * - Title updates amend the most recent matching entry.
 * - Max entries capped to avoid unbounded growth.
 * - All history is in-memory only — no persistence in Phase 20G.
 *
 * No Android imports. JVM-testable.
 */
class HistoryStore(private val maxEntries: Int = 200) {

    // tabId → ordered list, newest first
    private val _history = mutableMapOf<String, MutableList<HistoryEntry>>()
    private val _globalHistory = mutableListOf<HistoryEntry>()

    /** Records a completed navigation for [tabId]. Ignores duplicates of the last URL. */
    fun recordNavigation(tabId: String, url: String, timestampMs: Long) {
        if (url.isBlank() || url == "about:blank") return
        val entries = _history.getOrPut(tabId) { mutableListOf() }
        // Skip if this URL is identical to the most recent entry.
        if (entries.firstOrNull()?.url == url) return
        val entry = HistoryEntry(id = UUID.randomUUID().toString(), url = url, title = null, timestampMs = timestampMs)
        entries.add(0, entry)
        // Trim to max.
        while (entries.size > maxEntries) entries.removeAt(entries.size - 1)

        // Also retain in global browsing history
        if (_globalHistory.firstOrNull()?.url != url) {
            _globalHistory.add(0, entry)
            while (_globalHistory.size > maxEntries * 2) _globalHistory.removeAt(_globalHistory.size - 1)
        }
    }

    /** Updates the title of the most recent entry for [tabId] matching [url]. */
    fun updateTitle(tabId: String, url: String, title: String?) {
        if (title.isNullOrBlank()) return
        val entries = _history[tabId]
        if (entries != null) {
            val index = entries.indexOfFirst { it.url == url }
            if (index != -1) {
                entries[index] = entries[index].copy(title = title)
            }
        }
        val globalIndex = _globalHistory.indexOfFirst { it.url == url }
        if (globalIndex != -1) {
            _globalHistory[globalIndex] = _globalHistory[globalIndex].copy(title = title)
        }
    }

    /** Returns entries for [tabId], newest first. */
    fun entriesForTab(tabId: String): List<HistoryEntry> =
        _history[tabId]?.toList() ?: emptyList()

    /** Returns all entries across all tabs, newest first by timestamp. */
    fun allEntries(): List<HistoryEntry> =
        (_globalHistory.ifEmpty { _history.values.flatten() })
            .sortedByDescending { it.timestampMs }

    /** Clears history for [tabId] only. */
    fun clearTab(tabId: String) {
        _history.remove(tabId)
    }

    /** Clears all history. */
    fun clearAll() {
        _history.clear()
        _globalHistory.clear()
    }

    /** Removes history for [tabId] when a tab is closed, keeping global browsing history intact. */
    fun removeTab(tabId: String) {
        _history.remove(tabId)
    }

    /** Returns the total number of entries across all tabs. */
    fun totalCount(): Int =
        if (_globalHistory.isNotEmpty()) _globalHistory.size else _history.values.sumOf { it.size }
}

