package dev.fetchx.browser

import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.webkit.URLUtil
import androidx.annotation.OptIn
import org.mozilla.geckoview.GeckoResult
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoSession
import org.mozilla.geckoview.GeckoSessionSettings
import org.mozilla.geckoview.ExperimentalGeckoViewApi
import org.mozilla.geckoview.WebRequestError
import org.mozilla.geckoview.AllowOrDeny
import java.net.URI
import java.net.URISyntaxException
import java.util.UUID

private const val TAG = "TabManager"

enum class FindDirection {
    FIRST,
    NEXT,
    PREVIOUS,
}

internal class RestoredTabLocationGuard {
    private val pendingTabIds = mutableSetOf<String>()

    fun track(tabId: String, restoredUrl: String) {
        if (restoredUrl.isNotBlank() && restoredUrl != "about:blank") {
            pendingTabIds.add(tabId)
        }
    }

    fun shouldIgnore(tabId: String, url: String?, hasUserGesture: Boolean): Boolean {
        if (tabId !in pendingTabIds) return false
        if (url == "about:blank" && !hasUserGesture) return true
        pendingTabIds.remove(tabId)
        return false
    }

    fun clear(tabId: String) {
        pendingTabIds.remove(tabId)
    }

    fun clear() {
        pendingTabIds.clear()
    }
}

data class PageFindResult(
    val found: Boolean,
    val current: Int,
    val total: Int,
)

internal fun isHttpOrHttpsPageUrl(url: String): Boolean {
    val parsed = try {
        URI(url)
    } catch (_: URISyntaxException) {
        return false
    }
    val scheme = parsed.scheme ?: return false
    return parsed.isAbsolute &&
        !parsed.isOpaque &&
        (scheme.equals("http", ignoreCase = true) ||
            scheme.equals("https", ignoreCase = true)) &&
        !parsed.host.isNullOrBlank()
}

private fun applyDesktopSiteSettings(session: GeckoSession, enabled: Boolean) {
    val settings = session.settings
    settings.userAgentMode = if (enabled) {
        GeckoSessionSettings.USER_AGENT_MODE_DESKTOP
    } else {
        GeckoSessionSettings.USER_AGENT_MODE_MOBILE
    }
    settings.viewportMode = if (enabled) {
        GeckoSessionSettings.VIEWPORT_MODE_DESKTOP
    } else {
        GeckoSessionSettings.VIEWPORT_MODE_MOBILE
    }
    settings.userAgentOverride = if (enabled) {
        null
    } else {
        "Mozilla/5.0 (Android ${Build.VERSION.RELEASE}; Mobile; rv:158.0) Gecko/158.0 Firefox/158.0"
    }
}

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
    private val pendingPermissionResults = OneShotPermissionDecisions()
    private val restoredTabLocationGuard = RestoredTabLocationGuard()

    /** Callback to notify observers of state changes. */
    var onStateChanged: (() -> Unit)? = null

    /** Callback for lightweight per-tab GeckoSession navigation capability updates. */
    var onNavigationStateChanged: ((String, TabState) -> Unit)? = null

    /** Callback invoked on the main thread when a video URL/response is detected. */
    var onVideoDetected: ((DetectedVideo) -> Unit)? = null

    /** Callback invoked when a direct download / external file response is received from GeckoView. */
    var onDownloadRequested: ((url: String, filename: String?, mimeType: String?, contentLength: Long?) -> Unit)? = null

    /** Callback invoked when an external app URI scheme (mailto:, tel:, intent:, etc.) is requested. */
    var onExternalUriRequested: ((String) -> Unit)? = null

    /** Callback invoked when a magnet: URI is intercepted; takes priority over [onExternalUriRequested]. */
    var onMagnetRequested: ((String) -> Unit)? = null

    /** Callback invoked when a new page navigation clears the detected-video list. */
    var onVideoDetectionsCleared: (() -> Unit)? = null

    /** Callback invoked for GeckoView content/media permission requests. */
    var onSitePermissionRequested: ((origin: String, permission: String, detail: String?, resolve: (Boolean) -> Unit) -> Unit)? = null

    /** Callback invoked when GeckoView requires Android runtime permissions. */
    var onAndroidPermissionsRequested: ((permissions: Array<out String>, resolve: (Boolean) -> Unit) -> Unit)? = null

    // ──Public API ─────────────────────────────────────────────────────────

    /** Restores tabs from a saved snapshot and selects the active tab. */
    fun restoreTabs(savedTabs: List<TabState>, activeId: String?) {
        if (savedTabs.isEmpty()) return
        sessions.values.forEach { it.close() }
        sessions.clear()
        loadedTabIds.clear()
        restoredTabLocationGuard.clear()
        store.clear()

        val restoredActiveId = resolveRestoredActiveTabId(savedTabs, activeId)
        for (tab in savedTabs) {
            restoredTabLocationGuard.track(tab.id, tab.url)
            val session = buildSession(tab.id, tab.desktopSiteEnabled)
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
        restoredTabLocationGuard.clear(tabId)
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
            restoredTabLocationGuard.clear(tab.id)
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

    internal fun tabStoreIdentityForTest(): Pair<List<String>, String?> =
        store.allTabs().map { it.id } to store.activeTabId()

    /** Count of open tabs. */
    fun tabCount(): Int = store.tabCount()

    // ──Navigation on active tab ───────────────────────────────────────────

    fun navigate(
        rawUrl: String,
        searchEngine: SearchEngine = SearchEngine.DUCKDUCKGO,
    ) {
        val url = normalizeUrl(rawUrl, searchEngine)
        val tabId = store.activeTabId() ?: return
        restoredTabLocationGuard.clear(tabId)
        val session = sessions[tabId]
        Log.i(TAG, "Navigating tab $tabId")
        session?.loadUri(url)
        store.updateTab(tabId) { it.copy(url = url, errorText = null) }
        onStateChanged?.invoke()
    }

    fun goBack(): Boolean {
        val tabId = store.activeTabId() ?: return false
        val session = sessions[tabId] ?: return false
        restoredTabLocationGuard.clear(tabId)
        return SessionNavigationActions.goBackIfAvailable(store.activeTab()) {
            session.goBack()
        }
    }

    fun goForward(): Boolean {
        val tabId = store.activeTabId() ?: return false
        val session = sessions[tabId] ?: return false
        restoredTabLocationGuard.clear(tabId)
        return SessionNavigationActions.goForwardIfAvailable(store.activeTab()) {
            session.goForward()
        }
    }

    fun reload() {
        store.activeTabId()?.let(restoredTabLocationGuard::clear)
        activeSession()?.reload()
    }

    fun stopLoading() { activeSession()?.stop() }

    fun setDesktopSiteEnabled(tabId: String, enabled: Boolean): Boolean {
        val tab = store.findTab(tabId) ?: return false
        if (!isHttpOrHttpsPageUrl(tab.url)) return false
        val session = sessions[tabId] ?: return false
        applyDesktopSiteSettings(session, enabled)
        if (!store.updateTab(tabId) { it.copy(desktopSiteEnabled = enabled) }) return false
        restoredTabLocationGuard.clear(tabId)
        session.reload()
        onStateChanged?.invoke()
        return true
    }

    fun findInPage(
        tabId: String,
        query: String,
        direction: FindDirection,
        onResult: (PageFindResult?, String?) -> Unit,
    ) {
        val session = sessions[tabId]
        if (session == null) {
            onResult(null, "The page is no longer available.")
            return
        }
        if (query.isBlank()) {
            clearFindInPage(tabId)
            onResult(null, null)
            return
        }

        val flags = when (direction) {
            FindDirection.FIRST -> 0
            FindDirection.NEXT -> GeckoSession.FINDER_FIND_FORWARD
            FindDirection.PREVIOUS -> GeckoSession.FINDER_FIND_BACKWARDS
        }
        try {
            session.finder.displayFlags = GeckoSession.FINDER_DISPLAY_HIGHLIGHT_ALL
            session.finder.find(query, flags).accept(
                { result ->
                    if (sessions[tabId] === session && store.findTab(tabId) != null) {
                        if (result == null) {
                            onResult(null, "Find in Page returned no result.")
                        } else {
                            onResult(PageFindResult(result.found, result.current, result.total), null)
                        }
                    }
                },
                { error ->
                    Log.e(TAG, "GeckoView Find in Page failed for tab $tabId", error)
                    onResult(null, "Find in Page is unavailable.")
                },
            )
        } catch (error: RuntimeException) {
            Log.e(TAG, "Could not start GeckoView Find in Page for tab $tabId", error)
            onResult(null, "Find in Page is unavailable.")
        }
    }

    fun clearFindInPage(tabId: String) {
        val session = sessions[tabId] ?: return
        try {
            session.finder.clear()
        } catch (error: RuntimeException) {
            Log.e(TAG, "Could not clear GeckoView Find in Page for tab $tabId", error)
        }
    }

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

    /** Removes a saved history entry across the per-tab and global snapshots. */
    fun removeHistoryEntry(id: String): Boolean {
        val removed = historyStore.removeEntry(id)
        if (removed) onStateChanged?.invoke()
        return removed
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
        restoredTabLocationGuard.clear()
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

    private fun requestSitePermissionThenAndroid(
        origin: String,
        categories: List<String>,
        detail: String?,
        complete: (Boolean) -> Unit,
    ) {
        val androidPermissions = when {
            categories.all { it in setOf("camera", "microphone", "location") } ->
                androidPermissionsForCategories(categories)
            categories.all {
                it in setOf(
                    "notifications",
                    "persistent storage",
                    "virtual reality features",
                    "silent media autoplay",
                    "media autoplay with sound",
                    "protected media playback",
                    "permission to bypass tracking protection",
                    "third-party storage access",
                    "local device access",
                    "local network access",
                )
            } -> emptyList()
            else -> null
        }
        if (androidPermissions == null) {
            complete(false)
            return
        }
        val siteRequest = onSitePermissionRequested
        if (siteRequest == null) {
            complete(false)
            return
        }
        siteRequest(
            displayPermissionOrigin(origin),
            categories.joinToString(" and "),
            detail,
        ) { allowed ->
            if (!allowed) {
                complete(false)
            } else if (androidPermissions.isEmpty()) {
                complete(true)
            } else {
                val androidRequest = onAndroidPermissionsRequested
                if (androidRequest == null) {
                    complete(false)
                } else {
                    androidRequest(androidPermissions.toTypedArray(), complete)
                }
            }
        }
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

    @OptIn(markerClass = [ExperimentalGeckoViewApi::class])
    private fun buildSession(tabId: String, desktopSiteEnabled: Boolean = false): GeckoSession {
        val s = GeckoSession()
        val cfController = CloudflareBypassController()
        bypassControllers[tabId] = cfController
        s.settings.useTrackingProtection = true
        applyDesktopSiteSettings(s, desktopSiteEnabled)

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

        s.permissionDelegate = object : GeckoSession.PermissionDelegate {
            override fun onContentPermissionRequest(
                session: GeckoSession,
                permission: GeckoSession.PermissionDelegate.ContentPermission,
            ): GeckoResult<Int> {
                if (permission.value == GeckoSession.PermissionDelegate.ContentPermission.VALUE_ALLOW ||
                    permission.value == GeckoSession.PermissionDelegate.ContentPermission.VALUE_DENY
                ) {
                    if (permission.value == GeckoSession.PermissionDelegate.ContentPermission.VALUE_ALLOW &&
                        permission.permission == GeckoSession.PermissionDelegate.PERMISSION_GEOLOCATION
                    ) {
                        pendingPermissionResults.remember(permission.uri, listOf("location"), allowed = true)
                    }
                    return GeckoResult.fromValue(permission.value)
                }
                if (!shouldPromptForContentPermission(permission.permission, permission.value)) {
                    Log.w(TAG, "Rejecting unknown content permission value=${permission.value} type=${permission.permission}")
                    return GeckoResult.fromValue(GeckoSession.PermissionDelegate.ContentPermission.VALUE_DENY)
                }
                val description = sitePermissionDescription(permission.permission)
                    ?: return GeckoResult.fromValue(GeckoSession.PermissionDelegate.ContentPermission.VALUE_DENY)
                val result = GeckoResult<Int>()
                val categories = listOf(description.first)
                val priorDecision = pendingPermissionResults.consume(permission.uri, categories)
                if (priorDecision != null) {
                    result.complete(
                        if (priorDecision) {
                            GeckoSession.PermissionDelegate.ContentPermission.VALUE_ALLOW
                        } else {
                            GeckoSession.PermissionDelegate.ContentPermission.VALUE_DENY
                        },
                    )
                    return result
                }
                permission.notifyShown()
                if (description.first == "location") {
                    val siteRequest = onSitePermissionRequested
                    if (siteRequest == null) {
                        result.complete(GeckoSession.PermissionDelegate.ContentPermission.VALUE_DENY)
                    } else {
                        siteRequest(
                            displayPermissionOrigin(permission.uri),
                            description.first,
                            description.second,
                        ) { allowed ->
                            pendingPermissionResults.remember(permission.uri, categories, allowed)
                            result.complete(
                                if (allowed) {
                                    GeckoSession.PermissionDelegate.ContentPermission.VALUE_ALLOW
                                } else {
                                    GeckoSession.PermissionDelegate.ContentPermission.VALUE_DENY
                                },
                            )
                        }
                    }
                } else {
                    requestSitePermissionThenAndroid(
                        origin = permission.uri,
                        categories = categories,
                        detail = description.second,
                    ) { allowed ->
                        result.complete(
                            if (allowed) {
                                GeckoSession.PermissionDelegate.ContentPermission.VALUE_ALLOW
                            } else {
                                GeckoSession.PermissionDelegate.ContentPermission.VALUE_DENY
                            },
                        )
                    }
                }
                return result
            }

            override fun onMediaPermissionRequest(
                session: GeckoSession,
                uri: String,
                video: Array<GeckoSession.PermissionDelegate.MediaSource>?,
                audio: Array<GeckoSession.PermissionDelegate.MediaSource>?,
                callback: GeckoSession.PermissionDelegate.MediaCallback,
            ) {
                val videoSource = video?.firstOrNull {
                    it.source == GeckoSession.PermissionDelegate.MediaSource.SOURCE_CAMERA
                }
                val audioSource = audio?.firstOrNull {
                    it.source == GeckoSession.PermissionDelegate.MediaSource.SOURCE_MICROPHONE
                }
                val videoRequested = !video.isNullOrEmpty()
                val audioRequested = !audio.isNullOrEmpty()
                if ((videoRequested && videoSource == null) || (audioRequested && audioSource == null) ||
                    (!videoRequested && !audioRequested)
                ) {
                    callback.reject()
                    return
                }

                val requested = buildList {
                    if (videoSource != null) add("camera")
                    if (audioSource != null) add("microphone")
                }
                val priorDecision = pendingPermissionResults.consume(uri, requested)
                if (priorDecision != null) {
                    if (priorDecision) {
                        callback.grant(videoSource, audioSource)
                    } else {
                        callback.reject()
                    }
                    return
                }
                val details = listOfNotNull(videoSource?.name, audioSource?.name)
                    .distinct()
                    .takeIf { it.isNotEmpty() }
                    ?.joinToString()
                    ?.let { "Available device: $it" }
                requestSitePermissionThenAndroid(uri, requested, details) { allowed ->
                    if (allowed) {
                        callback.grant(videoSource, audioSource)
                    } else {
                        callback.reject()
                    }
                }
            }

            override fun onAndroidPermissionsRequest(
                session: GeckoSession,
                permissions: Array<out String>?,
                callback: GeckoSession.PermissionDelegate.Callback,
            ) {
                val requested = permissions.orEmpty()
                val categories = androidPermissionCategories(requested)
                if (categories == null) {
                    Log.w(TAG, "Rejecting unsupported Android permission set")
                    callback.reject()
                    return
                }
                val currentUrl = store.findTab(tabId)?.url.orEmpty()
                if (categories == listOf("location")) {
                    val siteDecision = pendingPermissionResults.consume(currentUrl, categories)
                    if (siteDecision != true) {
                        callback.reject()
                        return
                    }
                    val androidRequest = onAndroidPermissionsRequested
                    if (androidRequest == null) {
                        callback.reject()
                    } else {
                        androidRequest(requested) { granted ->
                            if (granted) callback.grant() else callback.reject()
                        }
                    }
                    return
                }
                requestSitePermissionThenAndroid(
                    origin = currentUrl,
                    categories = categories,
                    detail = null,
                ) { granted ->
                    pendingPermissionResults.remember(currentUrl, categories, granted)
                    if (granted) callback.grant() else callback.reject()
                }
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
                val metadata = browserDownloadMetadata(response.headers)
                val isVideoMime = VideoInterceptor.isVideoMimeType(metadata.mimeType)
                val isAttachment = disposition != null &&
                    disposition.lowercase(java.util.Locale.ROOT).contains("attachment")

                if (isVideoMime && !isAttachment) {
                    VideoInterceptor.detect(
                        tabId = tabId,
                        url = response.uri,
                        mimeType = metadata.mimeType,
                        contentLength = metadata.contentLength,
                    )?.let { onVideoDetected?.invoke(it) }
                } else {
                    onDownloadRequested?.invoke(
                        response.uri,
                        filename,
                        metadata.mimeType,
                        metadata.contentLength,
                    )
                }
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
                val openerTab = store.findTab(tabId)
                val newSession = buildSession(newId, openerTab?.desktopSiteEnabled == true)
                trackerBlockingController.registerTab(newId)
                sessions[newId] = newSession
                val normalized = if (uri.isNotBlank()) normalizeUrl(uri) else "about:blank"
                store.addTab(
                    TabState(
                        id = newId,
                        url = normalized,
                        spaceId = currentSpaceId,
                        desktopSiteEnabled = openerTab?.desktopSiteEnabled == true,
                    ),
                )
                Handler(Looper.getMainLooper()).post {
                    if (sessions[newId] === newSession) {
                        onStateChanged?.invoke()
                    }
                }
                return GeckoResult.fromValue(newSession)
            }

            override fun onLocationChange(
                session: GeckoSession,
                url: String?,
                perms: MutableList<GeckoSession.PermissionDelegate.ContentPermission>,
                hasUserGesture: Boolean,
            ) {
                if (restoredTabLocationGuard.shouldIgnore(tabId, url, hasUserGesture)) {
                    cfController.onLocationChange(url.orEmpty())
                    return
                }
                updateTabState(tabId) { it.copy(url = url ?: "", errorText = null) }
                val nonBlankUrl = !url.isNullOrBlank() && url != "about:blank"
                if (nonBlankUrl) onVideoDetectionsCleared?.invoke()
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
                restoredTabLocationGuard.clear(tabId)
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
