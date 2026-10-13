package dev.fetchx.browser

import java.util.UUID

/**
 * Global in-memory bookmark store.
 *
 * Rules:
 * - Bookmarks are global browser data, NOT per-tab.
 * - Duplicate URLs are rejected (same URL → no-op on add).
 * - Blank and non-http(s) URLs are rejected.
 * - Bookmarks are ordered newest first (most-recently-added at index 0).
 * - All state is in-memory only for Phase 20H.
 * - No Android imports — JVM-testable.
 *
 * Privacy:
 * - URLs and titles are NOT logged.
 * - Private browsing pages must not be persisted here (enforcement is the caller's responsibility).
 */
class BookmarkStore {

    private val _bookmarks = mutableListOf<Bookmark>()

    /**
     * Adds a bookmark if the URL is valid and not already bookmarked.
     * Returns the new [Bookmark], or null if the URL is invalid or already bookmarked.
     */
    fun add(url: String, title: String, nowMs: Long = System.currentTimeMillis()): Bookmark? {
        val clean = url.trim()
        if (!isValidUrl(clean)) return null
        if (_bookmarks.any { it.url == clean }) return null  // duplicate — silent no-op
        val b = Bookmark(
            id = UUID.randomUUID().toString(),
            url = clean,
            title = resolveTitle(title, clean),
            createdAtMs = nowMs,
            updatedAtMs = nowMs,
        )
        _bookmarks.add(0, b)  // newest first
        return b
    }

    /**
     * Removes the bookmark with [id].
     * Returns true if a bookmark was removed, false if [id] was not found.
     */
    fun removeById(id: String): Boolean = _bookmarks.removeAll { it.id == id }

    /**
     * Removes the bookmark whose URL matches [url] (after trimming).
     * Returns true if a bookmark was removed, false if no matching bookmark was found.
     */
    fun removeByUrl(url: String): Boolean = _bookmarks.removeAll { it.url == url.trim() }

    /**
     * Updates the title of the bookmark identified by [id].
     * Returns true if the bookmark was found and the title was updated.
     * Returns false if [id] is unknown or [newTitle] is blank.
     */
    fun update(id: String, newTitle: String): Boolean {
        if (newTitle.isBlank()) return false
        val index = _bookmarks.indexOfFirst { it.id == id }
        if (index == -1) return false
        val existing = _bookmarks[index]
        _bookmarks[index] = existing.copy(
            title = newTitle,
            updatedAtMs = System.currentTimeMillis(),
        )
        return true
    }

    /** Returns all bookmarks, newest first. */
    fun all(): List<Bookmark> = _bookmarks.toList()

    /** Returns the bookmark for [url] (trimmed), or null if not bookmarked. */
    fun findByUrl(url: String): Bookmark? = _bookmarks.find { it.url == url.trim() }

    /** Returns true if [url] is currently bookmarked. */
    fun isBookmarked(url: String): Boolean = findByUrl(url) != null

    /** Removes all bookmarks. */
    fun clearAll() { _bookmarks.clear() }

    /** Total number of bookmarks currently stored. */
    fun count(): Int = _bookmarks.size

    // ── Private helpers ───────────────────────────────────────────────────

    private fun isValidUrl(url: String): Boolean =
        url.isNotBlank() && (url.startsWith("http://") || url.startsWith("https://"))

    private fun resolveTitle(title: String, url: String): String {
        if (title.isNotBlank()) return title
        return try {
            java.net.URI(url).host?.takeIf { it.isNotBlank() } ?: url
        } catch (_: Exception) {
            url
        }
    }
}
