package dev.fetchx.browser

import android.util.Log
import org.mozilla.geckoview.GeckoResult
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoSession
import org.mozilla.geckoview.WebRequestError
import org.mozilla.geckoview.AllowOrDeny
import java.util.UUID

private const val TAG = "TabManager"

/**
 * Manages the set of browser GeckoSessions (one per tab).
 *
 * Ownership:
 * - One process-level GeckoRuntime (from GeckoRuntimeHolder) is shared by all sessions.
 * - Each tab owns exactly one GeckoSession.
 * - TabManager owns all sessions; callers receive TabState snapshots only.
 * - Sessions are closed when a tab is removed or the manager is disposed.
 *
 * Threading: all public methods must be called from the main thread.
 */
class TabManager(
    private val runtime: GeckoRuntime,
    private val trackerBlockingController: TrackerBlockingController,
) {

    private val store = TabStore()
    private val sessions = mutableMapOf<String, GeckoSession>()
    private val historyStore = HistoryStore()

    /** Callback to notify observers of state changes. */
    var onStateChanged: (() -> Unit)? = null

    /** Callback invoked on the main thread when a video URL/response is detected. */
    var onVideoDetected: ((DetectedVideo) -> Unit)? = null

    // ──Public API ─────────────────────────────────────────────────────────

    /** Creates a new tab in the given space, makes it active, and returns its ID. */
    fun createTab(initialUrl: String = "about:blank", spaceId: String = "default"): String {
        val id = UUID.randomUUID().toString()
        val session = buildSession(id)
        session.open(runtime)
        trackerBlockingController.registerTab(id)
        sessions[id] = session
        val normalizedUrl = normalizeUrl(initialUrl)
        store.addTab(TabState(id = id, url = normalizedUrl, spaceId = spaceId))
        session.loadUri(normalizedUrl)
        Log.i(TAG, "Created tab $id")
        onStateChanged?.invoke()
        return id
    }

    /** Closes the tab with [tabId]. If it was the active tab, selects an adjacent tab. */
    fun closeTab(tabId: String) {
        if (store.findTab(tabId) == null) return
        historyStore.removeTab(tabId)
        trackerBlockingController.removeTab(tabId)
        sessions[tabId]?.close()
        sessions.remove(tabId)
        store.removeTab(tabId)
        Log.i(TAG, "Closed tab $tabId, remaining: ${store.tabCount()}")
        // Ensure at least one tab always exists.
        if (store.tabCount() == 0) {
            createTab() // will call onStateChanged internally
        } else {
            onStateChanged?.invoke()
        }
    }

    /** Switches the active tab to [tabId]. */
    fun selectTab(tabId: String) {
        if (!store.setActive(tabId)) return
        Log.i(TAG, "Selected tab $tabId")
        onStateChanged?.invoke()
    }

    /** Returns the active tab's GeckoSession, or null if there are no tabs. */
    fun activeSession(): GeckoSession? = sessions[store.activeTabId()]

    /** Returns the active tab's state, or null if there are no tabs. */
    fun activeTabState(): TabState? = store.activeTab()

    /** Returns immutable snapshots of all tab states. */
    fun allTabStates(): List<TabState> = store.allTabs()

    /** Returns the active tab ID. */
    fun activeTabId(): String? = store.activeTabId()

    /** Count of open tabs. */
    fun tabCount(): Int = store.tabCount()

    // ──Navigation on active tab ───────────────────────────────────────────

    fun navigate(
        rawUrl: String,
        searchEngine: SearchEngine = SearchEngine.DUCKDUCKGO,
    ) {
        val url = normalizeUrl(rawUrl, searchEngine)
        val tabId = store.activeTabId() ?: return
        val session = sessions[tabId]
        Log.i(TAG, "navigate raw='$rawUrl' → url='$url' tabId=$tabId sessionOpen=${session != null} isOpen=${session?.isOpen}")
        session?.loadUri(url)
        store.updateTab(tabId) { it.copy(url = url, errorText = null) }
        onStateChanged?.invoke()
    }

    fun goBack() { activeSession()?.goBack() }
    fun goForward() { activeSession()?.goForward() }
    fun reload() { activeSession()?.reload() }

    // ──History API ────────────────────────────────────────────────────────

    /** Returns history entries for the active tab, newest first. */
    fun historyForActiveTab(): List<HistoryEntry> {
        val activeId = store.activeTabId() ?: return emptyList()
        return historyStore.entriesForTab(activeId)
    }

    /** Returns all history entries across all tabs, newest first by timestamp. */
    fun allHistory(): List<HistoryEntry> = historyStore.allEntries()

    /** Clears all history across all tabs and notifies observers. */
    fun clearAllHistory() {
        historyStore.clearAll()
        onStateChanged?.invoke()
    }

    /** Clears history for the active tab only and notifies observers. */
    fun clearTabHistory() {
        val activeId = store.activeTabId() ?: return
        historyStore.clearTab(activeId)
        onStateChanged?.invoke()
    }

    // ──Cleanup ────────────────────────────────────────────────────────────

    /** Closes all sessions. Call from BrowserViewModel.onCleared. */
    fun dispose() {
        historyStore.clearAll()
        trackerBlockingController.clearTabs()
        sessions.values.forEach { it.close() }
        sessions.clear()
        store.clear()
        Log.i(TAG, "TabManager disposed")
    }

    // ──Internal helpers ───────────────────────────────────────────────────

    private fun updateTabState(tabId: String, update: (TabState) -> TabState) {
        store.updateTab(tabId, update)
        onStateChanged?.invoke()
    }

    /**
     * If [uri] is a bare domain (no www. prefix), returns the same URL with www. added.
     * Returns null if the URL already has www., has a subdomain, or isn't a simple https domain.
     * Example: https://pornhub.com/ → https://www.pornhub.com/
     */
    private fun tryAddWww(uri: String): String? {
        return try {
            val parsed = java.net.URI(uri)
            val host = parsed.host ?: return null
            // Only retry for https, only if host is bare (exactly one dot, no subdomain)
            if (parsed.scheme != "https") return null
            if (host.startsWith("www.")) return null
            val dots = host.count { it == '.' }
            if (dots != 1) return null // skip subdomains like cdn.example.com
            val newHost = "www.$host"
            java.net.URI(parsed.scheme, parsed.userInfo, newHost, parsed.port, parsed.path, parsed.query, parsed.fragment).toString()
        } catch (_: Exception) {
            null
        }
    }

    private fun buildSession(tabId: String): GeckoSession {
        val s = GeckoSession()
        s.settings.useTrackingProtection = true
        // Use a standard Firefox for Android User-Agent. GeckoView's default UA
        // exposes the embedder and gets ERROR_NET_RESET from CDNs/WAFs (Cloudflare etc.)
        // that reject non-Firefox GeckoView embedders. Firefox 137 on Android 13.
        s.settings.userAgentOverride =
            "Mozilla/5.0 (Android 13; Mobile; rv:137.0) Gecko/137.0 Firefox/137.0"

        s.contentBlockingDelegate = object : org.mozilla.geckoview.ContentBlocking.Delegate {
            override fun onContentBlocked(
                session: GeckoSession,
                event: org.mozilla.geckoview.ContentBlocking.BlockEvent,
            ) {
                // Diagnostic log — only logs category int and whether it was blocking.
                // Does NOT log the URI to avoid leaking URLs.
                Log.i(TAG, "onContentBlocked tab=$tabId isBlocking=${event.isBlocking} category=${event.antiTrackingCategory}")
                val accepted = recordTrackerBlockedEvent(
                    controller = trackerBlockingController,
                    tabId = tabId,
                    isBlocking = event.isBlocking,
                    antiTrackingCategory = event.antiTrackingCategory,
                    requestUri = event.uri,
                )
                Log.i(TAG, "onContentBlocked accepted=$accepted tab=$tabId")
            }
        }

        s.contentDelegate = object : GeckoSession.ContentDelegate {
            override fun onTitleChange(session: GeckoSession, title: String?) {
                val currentUrl = store.findTab(tabId)?.url ?: ""
                historyStore.updateTitle(tabId, currentUrl, title)
                updateTabState(tabId) { it.copy(title = title) }
            }
        }

        s.navigationDelegate = object : GeckoSession.NavigationDelegate {
            override fun onLoadRequest(
                session: GeckoSession,
                request: GeckoSession.NavigationDelegate.LoadRequest,
            ): GeckoResult<AllowOrDeny>? {
                val url = request.uri
                if (VideoInterceptor.isVideoUrl(url)) {
                    VideoInterceptor.detect(tabId, url, null, null)
                        ?.let { onVideoDetected?.invoke(it) }
                }
                return null  // never block — always allow
            }

            override fun onLocationChange(
                session: GeckoSession,
                url: String?,
                perms: MutableList<GeckoSession.PermissionDelegate.ContentPermission>,
                hasUserGesture: Boolean,
            ) {
                Log.i(TAG, "onLocationChange tab=$tabId url=$url")
                updateTabState(tabId) { it.copy(url = url ?: "", errorText = null) }
            }

            override fun onCanGoBack(session: GeckoSession, canGoBack: Boolean) {
                updateTabState(tabId) { it.copy(canGoBack = canGoBack) }
            }

            override fun onCanGoForward(session: GeckoSession, canGoForward: Boolean) {
                updateTabState(tabId) { it.copy(canGoForward = canGoForward) }
            }

            override fun onLoadError(
                session: GeckoSession,
                uri: String?,
                error: WebRequestError,
            ): GeckoResult<String?>? {
                Log.e(TAG, "onLoadError tab=$tabId uri=$uri category=${error.category} code=${error.code}")
                // Auto-retry with www. prefix for bare domains (e.g. https://pornhub.com → https://www.pornhub.com)
                if (uri != null && error.category == WebRequestError.ERROR_CATEGORY_NETWORK) {
                    val wwwUrl = tryAddWww(uri)
                    if (wwwUrl != null) {
                        Log.i(TAG, "Retrying with www: $wwwUrl")
                        session.loadUri(wwwUrl)
                        store.updateTab(tabId) { it.copy(url = wwwUrl, errorText = null) }
                        onStateChanged?.invoke()
                        return null
                    }
                }
                updateTabState(tabId) {
                    it.copy(
                        errorText = "Could not load page. Check your connection.",
                        isLoading = false,
                    )
                }
                return null
            }
        }

        s.progressDelegate = object : GeckoSession.ProgressDelegate {
            override fun onPageStart(session: GeckoSession, url: String) {
                Log.i(TAG, "onPageStart tab=$tabId url=$url")
                updateTabState(tabId) { it.copy(isLoading = true, errorText = null) }
            }

            override fun onPageStop(session: GeckoSession, success: Boolean) {
                if (success) {
                    // Record navigation ONLY on successful page load.
                    val currentUrl = store.findTab(tabId)?.url ?: ""
                    if (currentUrl.isNotBlank() && currentUrl != "about:blank") {
                        historyStore.recordNavigation(tabId, currentUrl, System.currentTimeMillis())
                    }
                }
                updateTabState(tabId) { it.copy(isLoading = false) }
            }
        }

        return s
    }
}
