package dev.mydownloader.browser

import android.util.Log
import org.mozilla.geckoview.GeckoResult
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoSession
import org.mozilla.geckoview.WebRequestError
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
class TabManager(private val runtime: GeckoRuntime) {

    private val store = TabStore()
    private val sessions = mutableMapOf<String, GeckoSession>()

    /** Callback to notify observers of state changes. */
    var onStateChanged: (() -> Unit)? = null

    // ── Public API ──────────────────────────────────────────────────────────

    /** Creates a new tab, makes it active, and returns its ID. */
    fun createTab(initialUrl: String = "https://example.com"): String {
        val id = UUID.randomUUID().toString()
        val session = buildSession(id)
        session.open(runtime)
        sessions[id] = session
        val normalizedUrl = normalizeUrl(initialUrl)
        store.addTab(TabState(id = id, url = normalizedUrl))
        session.loadUri(normalizedUrl)
        Log.i(TAG, "Created tab $id")
        onStateChanged?.invoke()
        return id
    }

    /** Closes the tab with [tabId]. If it was the active tab, selects an adjacent tab. */
    fun closeTab(tabId: String) {
        if (store.findTab(tabId) == null) return
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

    // ── Navigation on active tab ────────────────────────────────────────────

    fun navigate(rawUrl: String) {
        val url = normalizeUrl(rawUrl)
        val tabId = store.activeTabId() ?: return
        sessions[tabId]?.loadUri(url)
        store.updateTab(tabId) { it.copy(url = url, errorText = null) }
        onStateChanged?.invoke()
    }

    fun goBack() { activeSession()?.goBack() }
    fun goForward() { activeSession()?.goForward() }
    fun reload() { activeSession()?.reload() }

    // ── Cleanup ─────────────────────────────────────────────────────────────

    /** Closes all sessions. Call from BrowserViewModel.onCleared. */
    fun dispose() {
        sessions.values.forEach { it.close() }
        sessions.clear()
        store.clear()
        Log.i(TAG, "TabManager disposed")
    }

    // ── Internal helpers ────────────────────────────────────────────────────

    private fun updateTabState(tabId: String, update: (TabState) -> TabState) {
        store.updateTab(tabId, update)
        onStateChanged?.invoke()
    }

    private fun buildSession(tabId: String): GeckoSession {
        val s = GeckoSession()

        s.contentDelegate = object : GeckoSession.ContentDelegate {
            override fun onTitleChange(session: GeckoSession, title: String?) {
                updateTabState(tabId) { it.copy(title = title) }
            }
        }

        s.navigationDelegate = object : GeckoSession.NavigationDelegate {
            override fun onLocationChange(
                session: GeckoSession,
                url: String?,
                perms: MutableList<GeckoSession.PermissionDelegate.ContentPermission>,
                hasUserGesture: Boolean,
            ) {
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
                updateTabState(tabId) { it.copy(isLoading = true, errorText = null) }
            }

            override fun onPageStop(session: GeckoSession, success: Boolean) {
                updateTabState(tabId) { it.copy(isLoading = false) }
            }
        }

        return s
    }
}
