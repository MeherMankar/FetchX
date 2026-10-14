package dev.fetchx.browser

import android.content.Context
import android.net.Uri
import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoView
import java.io.IOException
import java.lang.ref.WeakReference
import kotlin.coroutines.coroutineContext

private const val TAG = "BrowserViewModel"

data class BrowserProfile(
    val id: String,
    val name: String,
    val accent: String,
    val description: String,
)

data class SearchSuggestion(
    val title: String,
    val url: String,
    val category: String,
)

enum class SearchEngine(
    val displayName: String,
    val searchUrl: String,     // use %s for query placeholder
    val homepage: String,
    val isPrivate: Boolean = false,
) {
    GOOGLE("Google", "https://www.google.com/search?q=%s", "https://www.google.com"),
    BING("Bing", "https://www.bing.com/search?q=%s", "https://www.bing.com"),
    DUCKDUCKGO("DuckDuckGo", "https://duckduckgo.com/?q=%s", "https://duckduckgo.com", isPrivate = true),
    BRAVE("Brave Search", "https://search.brave.com/search?q=%s", "https://search.brave.com", isPrivate = true),
    STARTPAGE("Startpage", "https://www.startpage.com/search?q=%s", "https://www.startpage.com", isPrivate = true),
    ECOSIA("Ecosia", "https://www.ecosia.org/search?q=%s", "https://www.ecosia.org"),
    KAGI("Kagi", "https://kagi.com/search?q=%s", "https://kagi.com", isPrivate = true),
    MOJEEK("Mojeek", "https://www.mojeek.com/search?q=%s", "https://www.mojeek.com", isPrivate = true),
    WIKIPEDIA("Wikipedia", "https://en.wikipedia.org/w/index.php?search=%s", "https://en.wikipedia.org"),
    YOUTUBE("YouTube", "https://www.youtube.com/results?search_query=%s", "https://www.youtube.com"),
}

data class BrowserUiState(
    val tabs: List<TabState> = emptyList(),
    val activeTabId: String? = null,
    val urlInput: String = "about:blank",
    val showHistory: Boolean = false,
    val historyEntries: List<HistoryEntry> = emptyList(),
    val showBookmarks: Boolean = false,
    val bookmarks: List<Bookmark> = emptyList(),
    val isCurrentPageBookmarked: Boolean = false,
    val showTrackerDetails: Boolean = false,
    val trackerBlockingEnabled: Boolean = true,
    val activeTabBlockedRequests: Long = 0,
    val activeTabBlockedHosts: List<String> = emptyList(),
    val showPrivacySettings: Boolean = false,
    val privacySettings: PrivacySettings = PrivacySettings(),
    val showExtensions: Boolean = false,
    val extensionManagerState: ExtensionManagerState = ExtensionManagerState(isLoading = true),
    val isImportingExtension: Boolean = false,
    val showTabOverview: Boolean = false,
    val showBrowserSettings: Boolean = false,
    val showProfileSwitcher: Boolean = false,
    val readerModeAvailable: Boolean = false,
    val readerModeActive: Boolean = false,
    val profiles: List<BrowserProfile> = listOf(
        BrowserProfile("default", "Default", "#4F8EF7", "Daily browsing"),
        BrowserProfile("work", "Work", "#7C6AF2", "Productivity"),
        BrowserProfile("research", "Research", "#3AAE7A", "Study and reading"),
    ),
    val selectedProfileId: String = "default",
    val searchSuggestions: List<SearchSuggestion> = emptyList(),
    val selectedSearchEngine: SearchEngine = SearchEngine.DUCKDUCKGO,
    val showSearchEngineSelector: Boolean = false,
    // ── Zenium features ───────────────────────────────────────────────────
    val showSpaceSwitcher: Boolean = false,
    val showEssentials: Boolean = false,
    val essentials: List<Essential> = emptyList(),
    val showThemePicker: Boolean = false,
    val themeSeedColor: Long = 0xFF4F8EF7L,
    val compactMode: Boolean = false,
) {
    val activeTab: TabState? get() = tabs.find { it.id == activeTabId }
    val selectedProfile: BrowserProfile? get() = profiles.firstOrNull { it.id == selectedProfileId }
}

class BrowserViewModel internal constructor(
    internal val bookmarkController: BookmarkController = BookmarkController(),
    internal val trackerBlockingController: TrackerBlockingController = TrackerBlockingController(),
    internal val privacyController: PrivacyController = PrivacyController(),
    internal val essentialsStore: EssentialsStore = EssentialsStore(),
) : ViewModel() {

    private val _uiState = MutableStateFlow(BrowserUiState())
    val uiState: StateFlow<BrowserUiState> = _uiState.asStateFlow()

    private var tabManager: TabManager? = null
    private var geckoViewRef: WeakReference<GeckoView>? = null
    private var runtime: GeckoRuntime? = null
    private var applicationContext: Context? = null
    private var extensionManager: ExtensionManager? = null
    private var extensionStateListener: (() -> Unit)? = null

    init {
        trackerBlockingController.onStateChanged = ::syncTrackerBlockingState
        privacyController.onSettingsChanged = ::syncPrivacySettings
        syncPrivacySettings()
    }

    /**
     * Called from the AndroidView factory (and on recomposition).
     * First call: creates the TabManager and the initial tab.
     * Subsequent calls: re-attaches the active session to the (potentially new) GeckoView.
     */
    fun attachView(view: GeckoView, runtime: GeckoRuntime) {
        geckoViewRef = WeakReference(view)
        this.runtime = runtime
        applicationContext = view.context.applicationContext
        val manager = extensionManager ?: GeckoRuntimeHolder.getExtensionManager(runtime).also {
            extensionManager = it
        }
        val listener: () -> Unit = ::syncExtensionManagerState
        extensionStateListener = listener
        manager.onStateChanged = listener
        syncExtensionManagerState()
        applyPrivacySettings()
        val tm = tabManager ?: TabManager(runtime, trackerBlockingController).also { newTm ->
            tabManager = newTm
            newTm.onStateChanged = ::rebuildState
            newTm.createTab()
            Log.i(TAG, "TabManager initialized with first tab")
        }
        // Always attach the active session to the current view.
        tm.activeSession()?.let { view.setSession(it) }
        rebuildState()
    }

    private fun rebuildState() {
        val tm = tabManager ?: return
        val activeId = tm.activeTabId()
        val tabs = tm.allTabStates()
        val activeUrl = tabs.find { it.id == activeId }?.url ?: "about:blank"
        val historyEntries = tm.historyForActiveTab()
        bookmarkController.refreshForUrl(activeUrl)
        val bState = bookmarkController.state.value
        val trackerState = trackerBlockingController.state.value
        val trackerStats = trackerState.tabStats[activeId]
        val readerModeActive = activeUrl.startsWith("about:reader?url=")
        val readerModeAvailable = activeUrl.startsWith("http://") || activeUrl.startsWith("https://")
        val showEssentials = activeUrl == "about:blank" || activeUrl.isEmpty()
        _uiState.update {
            it.copy(
                tabs = tabs,
                activeTabId = activeId,
                urlInput = activeUrl.takeUnless { it == "about:blank" }.orEmpty(),
                historyEntries = historyEntries,
                isCurrentPageBookmarked = bState.isCurrentPageBookmarked,
                bookmarks = bState.bookmarks,
                trackerBlockingEnabled = trackerState.enabled,
                activeTabBlockedRequests = trackerStats?.blockedRequests ?: 0,
                activeTabBlockedHosts = trackerStats?.blockedHosts.orEmpty(),
                readerModeActive = readerModeActive,
                readerModeAvailable = readerModeAvailable && !readerModeActive,
                showEssentials = showEssentials,
                essentials = essentialsStore.mostVisited(tm.allHistory()),
            )
        }
    }

    // ──Tab actions ────────────────────────────────────────────────────────

    fun newTab(url: String = "about:blank") {
        tabManager?.createTab(url)
        reattachActiveSession()
    }

    fun closeTab(tabId: String) {
        tabManager?.closeTab(tabId)
        reattachActiveSession()
    }

    fun selectTab(tabId: String) {
        tabManager?.selectTab(tabId)
        reattachActiveSession()
    }

    // ──Navigation ─────────────────────────────────────────────────────────

    fun navigate(rawUrl: String) {
        tabManager?.navigate(rawUrl, _uiState.value.selectedSearchEngine)
    }

    fun goBack() { tabManager?.goBack() }
    fun goForward() { tabManager?.goForward() }
    fun reload() { tabManager?.reload() }

    // ──History ────────────────────────────────────────────────────────────

    fun openHistory() {
        _uiState.update { it.copy(showHistory = true) }
    }

    fun closeHistory() {
        _uiState.update { it.copy(showHistory = false) }
    }

    fun clearHistory() {
        tabManager?.clearAllHistory()
        _uiState.update { it.copy(historyEntries = emptyList(), showHistory = false) }
    }

    fun navigateToHistoryEntry(url: String) {
        closeHistory()
        tabManager?.navigate(url)
    }

    // ──Bookmarks ──────────────────────────────────────────────────────────

    /**
     * Toggles the bookmark state of the current active tab's URL.
     * No-op if the URL is blank, "about:blank", or the tab is still loading.
     */
    fun toggleBookmark() {
        val state = _uiState.value
        val tab = state.activeTab ?: return
        bookmarkController.toggleBookmark(tab.url, tab.title ?: "", tab.isLoading)
        syncBookmarkState()
    }

    fun openBookmarks() {
        _uiState.update { it.copy(showBookmarks = true) }
    }

    fun closeBookmarks() {
        _uiState.update { it.copy(showBookmarks = false) }
    }

    fun removeBookmark(id: String) {
        val currentUrl = _uiState.value.activeTab?.url ?: ""
        bookmarkController.removeBookmark(id, currentUrl)
        syncBookmarkState()
    }

    fun updateBookmark(id: String, newTitle: String) {
        bookmarkController.updateBookmark(id, newTitle, _uiState.value.activeTab?.url ?: "")
        syncBookmarkState()
    }

    fun navigateToBookmark(url: String) {
        closeBookmarks()
        tabManager?.navigate(url)
    }

    fun toggleReaderMode() {
        val session = tabManager?.activeSession() ?: return
        val active = !_uiState.value.readerModeActive
        _uiState.update { it.copy(readerModeActive = active) }
        if (active) {
            session.loadUri("about:reader?url=${_uiState.value.activeTab?.url?.let { java.net.URLEncoder.encode(it, "UTF-8") }}")
        } else {
            // Navigate back to the original URL from the reader URL
            val current = _uiState.value.activeTab?.url ?: return
            val original = if (current.startsWith("about:reader?url=")) {
                try { java.net.URLDecoder.decode(current.removePrefix("about:reader?url="), "UTF-8") }
                catch (_: Exception) { null }
            } else null
            if (original != null) session.loadUri(original)
        }
    }

    // ──Zenium features ────────────────────────────────────────────────────

    // Spaces
    fun openSpaceSwitcher() { _uiState.update { it.copy(showSpaceSwitcher = true) } }
    fun closeSpaceSwitcher() { _uiState.update { it.copy(showSpaceSwitcher = false) } }

    fun newTabInCurrentSpace(url: String = "about:blank") {
        val spaceId = _uiState.value.selectedProfileId
        tabManager?.createTab(url, spaceId)
        reattachActiveSession()
    }

    // Essentials
    fun navigateToEssential(url: String) { tabManager?.navigate(url) }
    // Theme color picker
    fun openThemePicker() { _uiState.update { it.copy(showThemePicker = true) } }
    fun closeThemePicker() { _uiState.update { it.copy(showThemePicker = false) } }
    fun setThemeSeedColor(colorLong: Long) {
        _uiState.update { it.copy(themeSeedColor = colorLong, showThemePicker = false) }
    }

    // Compact mode
    fun toggleCompactMode() { _uiState.update { it.copy(compactMode = !it.compactMode) } }

    fun openTrackerDetails() {
        _uiState.update { it.copy(showTrackerDetails = true) }
    }

    fun closeTrackerDetails() {
        _uiState.update { it.copy(showTrackerDetails = false) }
    }

    fun openTabOverview() {
        _uiState.update { it.copy(showTabOverview = true) }
    }

    fun closeTabOverview() {
        _uiState.update { it.copy(showTabOverview = false) }
    }

    fun openBrowserSettings() {
        _uiState.update { it.copy(showBrowserSettings = true) }
    }

    fun closeBrowserSettings() {
        _uiState.update { it.copy(showBrowserSettings = false) }
    }

    fun openProfileSwitcher() {
        _uiState.update { it.copy(showProfileSwitcher = true) }
    }

    fun closeProfileSwitcher() {
        _uiState.update { it.copy(showProfileSwitcher = false) }
    }

    fun switchProfile(profileId: String) {
        if (_uiState.value.profiles.none { it.id == profileId }) return
        _uiState.update { it.copy(selectedProfileId = profileId, showProfileSwitcher = false, showSpaceSwitcher = false) }
        reattachActiveSession()
    }

    fun setTrackerBlockingEnabled(enabled: Boolean) {
        trackerBlockingController.setEnabled(enabled)
        applyPrivacySettings()
    }

    fun openPrivacySettings() {
        _uiState.update { it.copy(showPrivacySettings = true) }
    }

    fun closePrivacySettings() {
        _uiState.update { it.copy(showPrivacySettings = false) }
    }

    fun setFingerprintingProtectionEnabled(enabled: Boolean) {
        privacyController.setFingerprintingProtectionEnabled(enabled)
        applyPrivacySettings()
    }

    fun setCryptominingProtectionEnabled(enabled: Boolean) {
        privacyController.setCryptominingProtectionEnabled(enabled)
        applyPrivacySettings()
    }

    fun setThirdPartyCookieIsolationEnabled(enabled: Boolean) {
        privacyController.setThirdPartyCookieIsolationEnabled(enabled)
        applyPrivacySettings()
    }

    fun setHttpsOnlyEnabled(enabled: Boolean) {
        privacyController.setHttpsOnlyEnabled(enabled)
        applyPrivacySettings()
    }

    fun openExtensions() {
        _uiState.update { it.copy(showExtensions = true) }
    }

    fun closeExtensions() {
        _uiState.update { it.copy(showExtensions = false) }
    }

    fun refreshExtensions() {
        extensionManager?.refresh()
    }

    fun installExtension(uri: Uri) {
        val manager = extensionManager
        val context = applicationContext
        if (manager == null || context == null) {
            _uiState.update {
                it.copy(extensionManagerState = it.extensionManagerState.copy(
                    error = "The browser extension service is not available.",
                ))
            }
            return
        }
        if (_uiState.value.isImportingExtension || manager.state.value.isBusy) return

        _uiState.update {
            it.copy(isImportingExtension = true, extensionManagerState = it.extensionManagerState.copy(error = null))
        }
        viewModelScope.launch(Dispatchers.IO) {
            var stagedPackage: StagedExtensionPackage? = null
            try {
                val staged = ExtensionPackageImporter(context).stage(uri)
                stagedPackage = staged
                coroutineContext.ensureActive()
                manager.install(staged)
                stagedPackage = null
            } catch (error: IOException) {
                manager.reportImportFailure()
            } catch (error: SecurityException) {
                manager.reportImportFailure()
            } finally {
                try {
                    stagedPackage?.close()
                } catch (error: IOException) {
                    manager.reportImportFailure(cleanupFailed = true)
                }
                _uiState.update { it.copy(isImportingExtension = false) }
            }
        }
    }

    fun enableExtension(id: ExtensionId) {
        extensionManager?.enable(id)
    }

    fun disableExtension(id: ExtensionId) {
        extensionManager?.disable(id)
    }

    fun removeExtension(id: ExtensionId) {
        extensionManager?.remove(id)
    }

    /**
     * Pulls the current bookmark state from [bookmarkController] and merges it
     * into [_uiState].  Safe to call when [tabManager] is null (e.g. in tests).
     */
    private fun syncBookmarkState() {
        val bState = bookmarkController.state.value
        _uiState.update {
            it.copy(
                bookmarks = bState.bookmarks,
                isCurrentPageBookmarked = bState.isCurrentPageBookmarked,
            )
        }
    }

    private fun syncTrackerBlockingState() {
        val trackerState = trackerBlockingController.state.value
        _uiState.update {
            val stats = trackerState.tabStats[it.activeTabId]
            it.copy(
                trackerBlockingEnabled = trackerState.enabled,
                activeTabBlockedRequests = stats?.blockedRequests ?: 0,
                activeTabBlockedHosts = stats?.blockedHosts.orEmpty(),
            )
        }
    }

    private fun syncPrivacySettings() {
        val settings = privacyController.settings.value
        _uiState.update { it.copy(privacySettings = settings) }
    }

    private fun syncExtensionManagerState() {
        val state = extensionManager?.state?.value ?: return
        _uiState.update { it.copy(extensionManagerState = state) }
    }

    private fun applyPrivacySettings() {
        if (runtime != null) {
            GeckoRuntimeHolder.applyPrivacySettings(
                trackerBlockingEnabled = trackerBlockingController.state.value.enabled,
                settings = privacyController.settings.value,
            )
        }
    }

    // ──Search engine ──────────────────────────────────────────────────────

    fun openSearchEngineSelector() {
        _uiState.update { it.copy(showSearchEngineSelector = true) }
    }

    fun closeSearchEngineSelector() {
        _uiState.update { it.copy(showSearchEngineSelector = false) }
    }

    fun selectSearchEngine(engine: SearchEngine) {
        _uiState.update { it.copy(selectedSearchEngine = engine, showSearchEngineSelector = false) }
    }

    // ──URL input sync ─────────────────────────────────────────────────────

    fun onUrlInputChanged(url: String) {
        val trimmed = url.trim()
        val suggestions = buildSuggestions(trimmed)
        _uiState.update { it.copy(urlInput = url, searchSuggestions = suggestions) }
    }

    fun syncUrlInput(url: String) {
        _uiState.update { it.copy(urlInput = url, searchSuggestions = emptyList()) }
    }

    fun applySuggestion(url: String) {
        val normalized = url.trim()
        if (normalized.isEmpty()) return
        _uiState.update { it.copy(urlInput = normalized, searchSuggestions = emptyList()) }
        tabManager?.navigate(normalized)
    }

    // ──Private ────────────────────────────────────────────────────────────

    private fun buildSuggestions(rawQuery: String): List<SearchSuggestion> {
        val query = rawQuery.trim().lowercase()
        val engine = _uiState.value.selectedSearchEngine
        if (query.isEmpty()) {
            return listOf(
                SearchSuggestion("Search with ${engine.displayName}", engine.homepage, "quick action"),
            )
        }

        val bookmarks = bookmarkController.state.value.bookmarks
        val history = tabManager?.allHistory() ?: emptyList()
        val results = mutableListOf<SearchSuggestion>()

        bookmarks.filter { it.title.contains(query, ignoreCase = true) || it.url.contains(query, ignoreCase = true) }
            .take(3)
            .forEach { results += SearchSuggestion(it.title, it.url, "bookmark") }

        history.filter { it.title?.contains(query, ignoreCase = true) == true || it.url.contains(query, ignoreCase = true) }
            .take(3)
            .forEach {
                val host = try {
                    java.net.URI(it.url).host
                } catch (_: java.net.URISyntaxException) {
                    null
                }
                val title = it.title?.takeIf(String::isNotBlank) ?: host?.removePrefix("www.") ?: "Visited site"
                results += SearchSuggestion(title, it.url, "history")
            }

        val encodedQuery = java.net.URLEncoder.encode(rawQuery.trim(), "UTF-8")
        if (query.length > 1) {
            val isAddress = looksLikeWebAddress(rawQuery)
            if (isAddress) {
                results += SearchSuggestion("Open website", normalizeUrl(rawQuery, engine), "website")
            } else {
                results += SearchSuggestion(
                    "Search “${rawQuery.trim()}” with ${engine.displayName}",
                    engine.searchUrl.replace("%s", encodedQuery),
                    "search · ${engine.displayName}",
                )
            }
        }

        return results.distinctBy { it.url }.take(5)
    }

    private fun reattachActiveSession() {
        val view = geckoViewRef?.get() ?: return
        tabManager?.activeSession()?.let { view.setSession(it) }
    }

    override fun onCleared() {
        super.onCleared()
        tabManager?.dispose()
        tabManager = null
        extensionManager?.let { manager ->
            if (manager.onStateChanged === extensionStateListener) {
                manager.onStateChanged = null
            }
        }
        extensionStateListener = null
        extensionManager = null
        applicationContext = null
        trackerBlockingController.onStateChanged = null
        privacyController.onSettingsChanged = null
        runtime = null
        Log.i(TAG, "BrowserViewModel cleared")
    }

    companion object {
        val Factory: ViewModelProvider.Factory = object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T =
                BrowserViewModel() as T
        }
    }
}

/**
 * Normalizes address-bar input, using the new-tab page for blank input.
 *
 * Examples:
 * - `example.com` → `https://example.com`
 * - `http://example.com` → unchanged
 * - `https://example.com` → unchanged
 * - blank or `about:blank` → `about:blank`
 *
 * Exposed as `internal` for unit testing.
 */
internal fun normalizeUrl(
    input: String,
    searchEngine: SearchEngine = SearchEngine.DUCKDUCKGO,
): String {
    val trimmed = input.trim()
    if (trimmed.isBlank() || trimmed == "about:blank") return "about:blank"
    if (trimmed.startsWith("http://") || trimmed.startsWith("https://")) return trimmed
    if (looksLikeWebAddress(trimmed)) return "https://$trimmed"

    val encodedQuery = java.net.URLEncoder.encode(trimmed, "UTF-8")
    return searchEngine.searchUrl.replace("%s", encodedQuery)
}

internal fun looksLikeWebAddress(input: String): Boolean {
    val trimmed = input.trim()
    if (trimmed.isEmpty() || trimmed.any(Char::isWhitespace)) return false
    val host = try {
        java.net.URI("https://$trimmed").host
    } catch (_: java.net.URISyntaxException) {
        null
    }
    return host != null && (trimmed.contains('.') || host == "localhost")
}
