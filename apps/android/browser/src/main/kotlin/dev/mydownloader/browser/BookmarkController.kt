package dev.fetchx.browser

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/**
 * Pure-JVM bookmark state controller.
 *
 * Contains the bookmark business logic that would otherwise live directly in
 * [BrowserViewModel].  By isolating it here:
 *
 * - The logic is JVM-unit-testable without Android runtime dependencies.
 * - [BrowserViewModel] delegates to this class and merges its state into
 *   [BrowserUiState].
 *
 * Privacy: no bookmark URLs or titles are logged anywhere in this class.
 * No Android imports. No SQLite — in-memory only for Phase 20H.
 */
class BookmarkController(
    internal val store: BookmarkStore = BookmarkStore(),
) {

    data class State(
        val bookmarks: List<Bookmark> = emptyList(),
        val isCurrentPageBookmarked: Boolean = false,
        // showBookmarks is managed by BrowserViewModel/BrowserUiState, not here.
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    // ── Toggle ──────────────────────────────────────────────────────────

    /**
     * Adds or removes a bookmark for the given [url].
     *
     * Returns false (no-op) when:
     * - [url] is blank or "about:blank"
     * - [isLoading] is true — prevents bookmarking a stale URL during navigation
     */
    fun toggleBookmark(url: String, title: String, isLoading: Boolean): Boolean {
        if (url.isBlank() || url == "about:blank") return false
        if (isLoading) return false
        if (store.isBookmarked(url)) {
            store.findByUrl(url)?.let { store.removeById(it.id) }
        } else {
            store.add(url, title)
        }
        refreshForUrl(url)
        return true
    }

    // ── Open / close dialog ─────────────────────────────────────────────
    // Note: showBookmarks is owned by BrowserViewModel/BrowserUiState.
    // These methods are intentionally absent from BookmarkController.

    // ── Mutate ──────────────────────────────────────────────────────────

    fun removeBookmark(id: String, currentUrl: String = "") {
        store.removeById(id)
        // Re-evaluate isCurrentPageBookmarked so the star icon updates immediately.
        refreshForUrl(currentUrl)
    }

    fun updateBookmark(id: String, newTitle: String, currentUrl: String = "") {
        store.update(id, newTitle)
        refreshForUrl(currentUrl)
    }

    // ── State refresh ───────────────────────────────────────────────────

    /**
     * Re-evaluates [isCurrentPageBookmarked] for [currentUrl] and refreshes the
     * bookmark list.  Called by [BrowserViewModel.rebuildState] when the active tab
     * changes.
     */
    fun refreshForUrl(currentUrl: String) {
        val isBookmarked = if (currentUrl.isNotBlank() && currentUrl != "about:blank")
            store.isBookmarked(currentUrl) else false
        _state.update {
            it.copy(
                bookmarks = store.all(),
                isCurrentPageBookmarked = isBookmarked,
            )
        }
    }

    /** Returns the current state snapshot. */
    fun currentState(): State = _state.value
}
