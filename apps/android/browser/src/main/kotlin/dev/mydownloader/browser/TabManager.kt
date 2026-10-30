package dev.fetchx.browser

import android.os.Build
import android.util.Log
import android.webkit.URLUtil
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
    private val loadedTabIds = mutableSetOf<String>()
    private val historyStore = HistoryStore()
    private val bypassControllers = mutableMapOf<String, CloudflareBypassController>()

    /** Callback to notify observers of state changes. */
    var onStateChanged: (() -> Unit)? = null

    /** Callback for lightweight per-tab GeckoSession navigation capability updates. */
    var onNavigationStateChanged: ((String, TabState) -> Unit)? = null

    /** Callback invoked on the main thread when a video URL/response is detected. */
    var onVideoDetected: ((DetectedVideo) -> Unit)? = null

    /** Callback invoked when a direct download / external file response is received from GeckoView. */
    var onDownloadRequested: ((url: String, filename: String?) -> Unit)? = null

    /** Callback invoked when an external app URI scheme (mailto:, tel:, intent:, etc.) is requested. */
    var onExternalUriRequested: ((String) -> Unit)? = null

    /** Callback invoked when a magnet: URI is intercepted; takes priority over [onExternalUriRequested]. */
    var onMagnetRequested: ((String) -> Unit)? = null

    // ──Public API ─────────────────────────────────────────────────────────

    /** Restores tabs from a saved snapshot and selects the active tab. */
    fun restoreTabs(savedTabs: List<TabState>, activeId: String?) {
        if (savedTabs.isEmpty()) return
        sessions.values.forEach { it.close() }
        sessions.clear()
        loadedTabIds.clear()
        store.clear()

        val restoredActiveId = activeId?.takeIf { saved -> savedTabs.any { it.id == saved } }
            ?: savedTabs.last().id
        for (tab in savedTabs) {
            val session = buildSession(tab.id)
            session.open(runtime)
            trackerBlockingController.registerTab(tab.id)
            sessions[tab.id] = session
            store.addTab(tab)
        }
        store.setActive(restoredActiveId)
        loadRestoredTab(restoredActiveId)
        onStateChanged?.invoke()
    }

    /** Creates a new tab in the given space, makes it active, and returns its ID. */
    fun createTab(initialUrl: String = "about:blank", spaceId: String = "default"): String {
        val id = UUID.randomUUID().toString()
        val session = buildSession(id)
        session.open(runtime)
        trackerBlockingController.registerTab(id)
        sessions[id] = session
        loadedTabIds.add(id)
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
        sessions.remove(tabId)?.close()
        loadedTabIds.remove(tabId)
        bypassControllers.remove(tabId)?.reset()
        store.removeTab(tabId)
        Log.i(TAG, "Closed tab $tabId, remaining: ${store.tabCount()}")
        // Ensure at least one tab always exists.
        if (store.tabCount() == 0) {
            createTab() // will call onStateChanged internally
        } else {
            onStateChanged?.invoke()
        }
    }

    /** Closes all tabs in the given space, or across all spaces if spaceId is null. */
    fun closeAllTabs(spaceId: String? = null) {
        val tabsToClose = if (spaceId == null) store.allTabs() else store.allTabsForSpace(spaceId)
        tabsToClose.forEach { tab ->
            historyStore.removeTab(tab.id)
            trackerBlockingController.removeTab(tab.id)
            sessions.remove(tab.id)?.close()
            loadedTabIds.remove(tab.id)
            store.removeTab(tab.id)
        }
        if (store.tabCount() == 0) {
            createTab(spaceId = spaceId ?: "default")
        } else {
            onStateChanged?.invoke()
        }
    }

    /** Switches the active tab to [tabId]. */
    fun selectTab(tabId: String) {
        if (!store.setActive(tabId)) return
        loadRestoredTab(tabId)
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
        Log.i(TAG, "Navigating tab $tabId")
        session?.loadUri(url)
        store.updateTab(tabId) { it.copy(url = url, errorText = null) }
        onStateChanged?.invoke()
    }

    fun goBack(): Boolean {
        val session = activeSession() ?: return false
        return SessionNavigationActions.goBackIfAvailable(store.activeTab()) {
            session.goBack()
        }
    }

    fun goForward(): Boolean {
        val session = activeSession() ?: return false
        return SessionNavigationActions.goForwardIfAvailable(store.activeTab()) {
            session.goForward()
        }
    }

    fun reload() { activeSession()?.reload() }

    // ──History API ────────────────────────────────────────────────────────

    /** Returns history entries for the active tab, newest first. */
    fun historyForActiveTab(): List<HistoryEntry> {
        val activeId = store.activeTabId() ?: return emptyList()
        return historyStore.entriesForTab(activeId)
    }

    /** Returns all history entries across all tabs, newest first by timestamp. */
    fun allHistory(): List<HistoryEntry> = historyStore.allEntries()

    fun restoreHistory(entries: List<HistoryEntry>) {
        historyStore.restoreGlobalHistory(entries)
    }

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
        val openSessions = sessions.values.toList()
        sessions.clear()
        loadedTabIds.clear()
        openSessions.forEach { it.close() }
        bypassControllers.values.forEach { it.reset() }
        bypassControllers.clear()
        store.clear()
        Log.i(TAG, "TabManager disposed")
    }

    /** Returns the [CloudflareBypassController] for the given tab, if one exists. */
    fun cfControllerForTab(tabId: String): CloudflareBypassController? = bypassControllers[tabId]

    /** Returns the [CloudflareBypassController] for the currently active tab. */
    fun activeCfController(): CloudflareBypassController? = bypassControllers[store.activeTabId()]

    private fun loadRestoredTab(tabId: String) {
        if (!loadedTabIds.add(tabId)) return
        val url = store.findTab(tabId)?.url ?: return
        if (url.isNotBlank() && url != "about:blank") {
            sessions[tabId]?.loadUri(url)
        }
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
        val cfController = CloudflareBypassController()
        bypassControllers[tabId] = cfController
        s.settings.useTrackingProtection = true
        // Match the Firefox version to the embedded GeckoView release so sites do
        // not see an obsolete browser version or the GeckoView embedder token.
        s.settings.userAgentOverride =
            "Mozilla/5.0 (Android ${Build.VERSION.RELEASE}; Mobile; rv:158.0) Gecko/158.0 Firefox/158.0"

        s.contentBlockingDelegate = object : org.mozilla.geckoview.ContentBlocking.Delegate {
            override fun onContentBlocked(
                session: GeckoSession,
                event: org.mozilla.geckoview.ContentBlocking.BlockEvent,
            ) {
                val accepted = recordTrackerBlockedEvent(
                    controller = trackerBlockingController,
                    tabId = tabId,
                    isBlocking = event.isBlocking,
                    antiTrackingCategory = event.antiTrackingCategory,
                    requestUri = event.uri,
                )
            }
        }

        s.contentDelegate = object : GeckoSession.ContentDelegate {
            override fun onTitleChange(session: GeckoSession, title: String?) {
                val currentUrl = store.findTab(tabId)?.url ?: ""
                historyStore.updateTitle(tabId, currentUrl, title)
                updateTabState(tabId) { it.copy(title = title) }
                cfController.onTitleChange(title)
            }

            override fun onExternalResponse(session: GeckoSession, response: org.mozilla.geckoview.WebResponse) {
                Log.i(TAG, "External download response received")
                val disposition = response.headers["content-disposition"] ?: response.headers["Content-Disposition"]
                val contentType = response.headers["content-type"] ?: response.headers["Content-Type"]
                val filename = URLUtil.guessFileName(response.uri, disposition, contentType)
                    .takeIf { it.isNotBlank() }
                onDownloadRequested?.invoke(response.uri, filename)
            }
        }


        s.navigationDelegate = object : GeckoSession.NavigationDelegate {
            override fun onLoadRequest(
                session: GeckoSession,
                request: GeckoSession.NavigationDelegate.LoadRequest,
            ): GeckoResult<AllowOrDeny>? {
                val url = request.uri
                if (url.lowercase().startsWith("magnet:")) {
                    Log.i(TAG, "Intercepting magnet URI")
                    val handler = onMagnetRequested
                    if (handler != null) {
                        handler(url)
                    } else {
                        onExternalUriRequested?.invoke(url)
                    }
                    return GeckoResult.fromValue(AllowOrDeny.DENY)
                }
                if (isExternalScheme(url)) {
                    Log.i(TAG, "Handling external URI scheme")
                    onExternalUriRequested?.invoke(url)
                    return GeckoResult.fromValue(AllowOrDeny.DENY)
                }
                if (VideoInterceptor.isVideoUrl(url)) {
                    VideoInterceptor.detect(tabId, url, null, null)
                        ?.let { onVideoDetected?.invoke(it) }
                }
                return null  // never block — always allow
            }

            override fun onNewSession(
                session: GeckoSession,
                uri: String,
            ): GeckoResult<GeckoSession>? {
                Log.i(TAG, "Opening new tab from tab $tabId")
                val currentSpaceId = store.findTab(tabId)?.spaceId ?: "default"
                val newId = UUID.randomUUID().toString()
                val newSession = buildSession(newId)
                newSession.open(runtime)
                trackerBlockingController.registerTab(newId)
                sessions[newId] = newSession
                val normalized = if (uri.isNotBlank()) normalizeUrl(uri) else "about:blank"
                store.addTab(TabState(id = newId, url = normalized, spaceId = currentSpaceId))
                onStateChanged?.invoke()
                return GeckoResult.fromValue(newSession)
            }

            override fun onLocationChange(
                session: GeckoSession,
                url: String?,
                perms: MutableList<GeckoSession.PermissionDelegate.ContentPermission>,
                hasUserGesture: Boolean,
            ) {
                updateTabState(tabId) { it.copy(url = url ?: "", errorText = null) }
                cfController.onLocationChange(url ?: "")
            }

            override fun onCanGoBack(session: GeckoSession, canGoBack: Boolean) {
                Log.d(TAG, "navigation capability tab=$tabId back=$canGoBack session=${System.identityHashCode(session)}")
                updateNavigationState(session, tabId) { it.copy(canGoBack = canGoBack) }
            }

            override fun onCanGoForward(session: GeckoSession, canGoForward: Boolean) {
                Log.d(TAG, "navigation capability tab=$tabId forward=$canGoForward session=${System.identityHashCode(session)}")
                updateNavigationState(session, tabId) { it.copy(canGoForward = canGoForward) }
            }

            override fun onLoadError(
                session: GeckoSession,
                uri: String?,
                error: WebRequestError,
            ): GeckoResult<String?>? {
                Log.e(TAG, "Page load failed in tab $tabId category=${error.category} code=${error.code}")
                // Auto-retry with www. prefix for bare domains (e.g. https://pornhub.com → https://www.pornhub.com)
                if (uri != null && error.category == WebRequestError.ERROR_CATEGORY_NETWORK) {
                    val wwwUrl = tryAddWww(uri)
                    if (wwwUrl != null) {
                        Log.i(TAG, "Retrying page load with www prefix")
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
                        progress = 0,
                    )
                }
                return null
            }
        }

        s.progressDelegate = object : GeckoSession.ProgressDelegate {
            override fun onPageStart(session: GeckoSession, url: String) {
                updateTabState(tabId) { it.copy(isLoading = true, progress = 10, errorText = null) }
            }

            override fun onProgressChange(session: GeckoSession, progress: Int) {
                updateTabState(tabId) { it.copy(progress = progress) }
            }

            override fun onPageStop(session: GeckoSession, success: Boolean) {
                if (success) {
                    // Record navigation ONLY on successful page load.
                    val currentUrl = store.findTab(tabId)?.url ?: ""
                    if (currentUrl.isNotBlank() && currentUrl != "about:blank") {
                        historyStore.recordNavigation(tabId, currentUrl, System.currentTimeMillis())
                    }
                }
                updateTabState(tabId) { it.copy(isLoading = false, progress = 100) }
            }
        }

        return s
    }


    private fun updateNavigationState(
        session: GeckoSession,
        tabId: String,
        update: (TabState) -> TabState,
    ) {
        if (sessions[tabId] !== session) return
        if (!store.updateTab(tabId, update)) return
        val state = store.findTab(tabId) ?: return
        onNavigationStateChanged?.invoke(tabId, state)
    }
}
