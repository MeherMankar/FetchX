package dev.fetchx.browser

/**
 * Immutable snapshot of a single browser tab's observable state.
 * The GeckoSession is NOT part of this data class — it is managed by TabManager.
 */
data class TabState(
    val id: String,
    val url: String = "about:blank",
    val title: String? = null,
    val isLoading: Boolean = false,
    val canGoBack: Boolean = false,
    val canGoForward: Boolean = false,
    val errorText: String? = null,
    /** The Space (profile) this tab belongs to. */
    val spaceId: String = "default",
)
