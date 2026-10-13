package dev.fetchx.browser

/**
 * Immutable snapshot of a single browser tab's observable state.
 * The GeckoSession is NOT part of this data class — it is managed by TabManager.
 */
data class TabState(
    val id: String,           // stable UUID, never changes for the lifetime of this tab
    val url: String = "https://example.com",
    val title: String? = null,
    val isLoading: Boolean = false,
    val canGoBack: Boolean = false,
    val canGoForward: Boolean = false,
    val errorText: String? = null,
)

