package dev.fetchx.browser

import android.content.Context
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.net.Uri
import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import dev.fetchx.browser.DurableBrowserTabStore
import dev.fetchx.browser.DurableBrowserTabStore.WriteResult
import dev.fetchx.browser.TabRestoreGuard
import dev.fetchx.browser.TabRestoreNavigationGate
import dev.fetchx.browser.TabSnapshotWriteGate
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import dev.fetchx.core.download.YtDlpFormat
import java.io.File
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoSession
import org.mozilla.geckoview.GeckoView
import java.io.IOException
import java.lang.ref.WeakReference
import java.util.ArrayDeque
import java.util.UUID
import kotlin.coroutines.coroutineContext

private const val TAG = "BrowserViewModel"

data class BrowserProfile(
    val id: String,
    val name: String,
    val accent: String,
    val description: String,
)

data class FindInPageState(
    val tabId: String,
    val pageUrl: String,
    val query: String = "",
    val currentMatch: Int = 0,
    val totalMatches: Int = 0,
    val found: Boolean = false,
    val error: String? = null,
)

data class SearchSuggestion(
    val title: String,
    val url: String,
    val category: String,
    val tabId: String? = null,
)

internal data class MediaDownloadRoute(
    val url: String,
    val isDirectMedia: Boolean,
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
    val isRestoringTabs: Boolean = false,
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
    val sitePermissionPrompt: SitePermissionPrompt? = null,
    val androidRuntimePermissionRequests: List<AndroidRuntimePermissionRequest> = emptyList(),
    val showPrivacySettings: Boolean = false,
    val privacySettings: PrivacySettings = PrivacySettings(),
    val showExtensions: Boolean = false,
    val fetchxRemotePopupSession: GeckoSession? = null,
    val extensionManagerState: ExtensionManagerState = ExtensionManagerState(isLoading = true),
    val isImportingExtension: Boolean = false,
    val firefoxAddonsCatalogState: FirefoxAddonsCatalogState = FirefoxAddonsCatalogState(),
    val showTabOverview: Boolean = false,
    val showBrowserSettings: Boolean = false,
    val showProfileSwitcher: Boolean = false,
    val findInPage: FindInPageState? = null,
    val profiles: List<BrowserProfile> = listOf(
        BrowserProfile("default", "Default", "#4F8EF7", "Daily browsing"),
        BrowserProfile("work", "Work", "#7C6AF2", "Productivity"),
        BrowserProfile("research", "Research", "#3AAE7A", "Study and reading"),
    ),
    val selectedProfileId: String = "default",
    val searchSuggestions: List<SearchSuggestion> = emptyList(),
    val isLoadingSearchSuggestions: Boolean = false,
    val searchSuggestionError: String? = null,
    val isLocalSearch: Boolean = false,
    val showLocalSearchEmpty: Boolean = false,
    val browserSuggestionsEnabled: Boolean = true,
    val selectedSearchEngine: SearchEngine = SearchEngine.DUCKDUCKGO,
    val showSearchEngineSelector: Boolean = false,
    // ── Zenium features ───────────────────────────────────────────────────
    val showSpaceSwitcher: Boolean = false,
    val showEssentials: Boolean = false,
    val essentials: List<Essential> = emptyList(),
    val showThemePicker: Boolean = false,
    val compactMode: Boolean = false,
    val detectedVideos: List<DetectedVideo> = emptyList(),
    val showVideoDownloadBar: Boolean = false,
    val pendingMediaVideo: DetectedVideo? = null,
    val mediaFormats: List<YtDlpFormat> = emptyList(),
    val isDiscoveringMediaFormats: Boolean = false,
    val mediaFormatError: String? = null,
    val mediaQueueError: String? = null,
    val isQueueingMedia: Boolean = false,
    val pendingDownloads: List<PendingDownload> = emptyList(),
    val magnetQueued: String? = null,
    val magnetError: String? = null,
    val cloudflareBypassState: CloudflareBypassState = CloudflareBypassState.Idle,
) {
    val activeTab: TabState? get() = tabs.find { it.id == activeTabId }
    val selectedProfile: BrowserProfile? get() = profiles.firstOrNull { it.id == selectedProfileId }
}

internal data class BrowserTabIdentityLayers(
    val tabStoreIds: List<String>,
    val tabStoreActiveId: String?,
    val tabManagerIds: List<String>,
    val tabManagerActiveId: String?,
    val viewModelIds: List<String>,
    val viewModelActiveId: String?,
)

data class PendingDownload(
    val url: String,
    val filename: String? = null,
    val mimeType: String? = null,
    val contentLength: Long? = null,
)

private data class PendingSitePermission(
    val prompt: SitePermissionPrompt,
    val resolve: (Boolean) -> Unit,
)

private data class PendingAndroidRuntimePermission(
    val request: AndroidRuntimePermissionRequest,
    val requestedPermissions: List<String>,
    val resolve: (Boolean) -> Unit,
)

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
    private var searchPreferences: SharedPreferences? = null
    private var browserDataPreferences: SharedPreferences? = null
    private var durableTabStore: DurableBrowserTabStore? = null
    private var tabRestoreJob: Job? = null
    private val tabRestoreGuard = TabRestoreGuard()
    private val tabSnapshotWriteGate = TabSnapshotWriteGate()
    private val tabRestoreNavigationGate = TabRestoreNavigationGate()
    private var tabManagerInitialized = false
    private var searchSuggestionJob: Job? = null
    private var findRequestGeneration = 0
    private var mediaFormatJob: Job? = null
    private var mediaFormatGeneration = 0
    private var extensionManager: ExtensionManager? = null
    private var extensionStateListener: (() -> Unit)? = null
    private val firefoxAddonsCatalog = FirefoxAddonsCatalog()
    private var firefoxAddonsSearchGeneration = 0
    private val permissionLock = Any()
    private val sitePermissionQueue = ArrayDeque<PendingSitePermission>()
    private val androidPermissionQueue = ArrayDeque<PendingAndroidRuntimePermission>()

    init {
        trackerBlockingController.onStateChanged = ::syncTrackerBlockingState
        privacyController.onSettingsChanged = ::syncPrivacySettings
        syncPrivacySettings()
    }

    /**
     * Called from the AndroidView factory (and on recomposition).
     * First call: creates the TabManager and asynchronously restores saved tabs.
     * Subsequent calls: re-attaches the active session to the (potentially new) GeckoView.
     */
    fun attachView(view: GeckoView, runtime: GeckoRuntime) {
        geckoViewRef = WeakReference(view)
        this.runtime = runtime
        applicationContext = view.context.applicationContext
        loadSearchSettings(view.context.applicationContext)
        loadSavedPrivacySettings()
        val savedBookmarks = loadSavedBookmarks()
        bookmarkController.loadBookmarks(savedBookmarks)
        val manager = extensionManager ?: GeckoRuntimeHolder.getExtensionManager(runtime).also {
            extensionManager = it
        }
        GeckoRuntimeHolder.onVideoDetectedFromExtension = ::onVideoDetected
        GeckoRuntimeHolder.setFetchxRemotePopupSessionListener { session ->
            _uiState.update { it.copy(fetchxRemotePopupSession = session) }
        }
        val listener: () -> Unit = ::syncExtensionManagerState
        extensionStateListener = listener
        manager.onStateChanged = listener
        syncExtensionManagerState()
        applyPrivacySettings()
        val tm = tabManager ?: TabManager(runtime, trackerBlockingController).also { newTm ->
            tabManager = newTm
            newTm.onStateChanged = {
                tabRestoreGuard.markStateChanged()
                rebuildState()
            }
            newTm.onNavigationStateChanged = { tabId, tab ->
                tabRestoreGuard.markStateChanged()
                updateNavigationState(tabId, tab)
            }
            newTm.onVideoDetected = ::onVideoDetected
            newTm.onDownloadRequested = { url, filename, mimeType, contentLength ->
                _uiState.update {
                    it.copy(
                        pendingDownloads = it.pendingDownloads + PendingDownload(
                            url = url,
                            filename = filename,
                            mimeType = mimeType,
                            contentLength = contentLength,
                        ),
                    )
                }
            }
            newTm.onExternalUriRequested = ::openExternalUri
            newTm.onMagnetRequested = ::onMagnetRequested
            newTm.onVideoDetectionsCleared = { clearDetectedVideos() }
            newTm.onSitePermissionRequested = ::enqueueSitePermission
            newTm.onAndroidPermissionsRequested = ::requestAndroidRuntimePermissions
        }
        if (!tabManagerInitialized && tabRestoreJob == null) {
            beginTabRestore(tm)
        } else {
            tm.activeSession()?.let { view.setSession(it) }
            if (tabManagerInitialized) rebuildState()
        }
    }

    private fun beginTabRestore(manager: TabManager) {
        val context = applicationContext ?: return
        val store = getDurableTabStore()
        val restoreVersion = tabRestoreGuard.version()
        tabSnapshotWriteGate.beginRestore()
        tabRestoreNavigationGate.beginRestore()
        _uiState.update { it.copy(isRestoringTabs = true) }
        tabRestoreJob = viewModelScope.launch {
            try {
                val (savedTabs, savedHistory) = withContext(Dispatchers.IO) {
                    loadSavedTabSnapshot(store, context) to loadSavedHistory(context)
                }
                if (!tabRestoreGuard.canApply(restoreVersion)) {
                    Log.w(TAG, "Skipping tab restore because browser state changed while loading")
                    completeTabRestore(manager)
                    return@launch
                }

                manager.restoreHistory(savedHistory)
                if (savedTabs.first.isNotEmpty()) {
                    manager.restoreTabs(savedTabs.first, savedTabs.second)
                } else {
                    manager.createTab()
                }
                completeTabRestore(manager)
                Log.i(TAG, "TabManager initialized")
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                Log.e(TAG, "Could not restore browser state; opening a new tab", error)
                if (tabRestoreGuard.canApply(restoreVersion)) {
                    manager.createTab()
                }
                completeTabRestore(manager)
            } finally {
                if (!tabRestoreGuard.isDisposed()) {
                    _uiState.update { it.copy(isRestoringTabs = false) }
                    tabSnapshotWriteGate.completeRestore(::persistCurrentTabs)
                }
                tabRestoreJob = null
            }
        }
    }

    private fun completeTabRestore(manager: TabManager) {
        tabManagerInitialized = true
        tabRestoreNavigationGate.completeRestore()?.let { url ->
            manager.navigate(url, _uiState.value.selectedSearchEngine)
        }
    }

    private fun rebuildState() {
        val tm = tabManager ?: return
        val activeId = tm.activeTabId()
        val previousActiveId = _uiState.value.activeTabId
        if (activeId != null && activeId != previousActiveId) {
            reattachActiveSession()
        }
        val tabs = tm.allTabStates()
        val activeUrl = tabs.find { it.id == activeId }?.url ?: "about:blank"
        val historyEntries = tm.allHistory()
        bookmarkController.refreshForUrl(activeUrl)
        val bState = bookmarkController.state.value
        val trackerState = trackerBlockingController.state.value
        val trackerStats = trackerState.tabStats[activeId]
        val showEssentials = activeUrl == "about:blank" || activeUrl.isEmpty()
        val findState = _uiState.value.findInPage
        if (findState != null &&
            (activeId != findState.tabId || activeUrl != findState.pageUrl)
        ) {
            findRequestGeneration += 1
            tm.clearFindInPage(findState.tabId)
        }
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
                findInPage = findState?.takeIf {
                    activeId == it.tabId && activeUrl == it.pageUrl
                },
                showEssentials = showEssentials,
                essentials = essentialsStore.mostVisited(tm.allHistory()),
            )
        }
        saveCurrentTabs(tabs, activeId)
        saveBookmarks(bState.bookmarks)
        saveHistory(tm.allHistory())
        // Keep cloudflareBypassState in sync with the active tab's controller.
        val cfState = tm.activeCfController()?.state?.value ?: CloudflareBypassState.Idle
        _uiState.update { it.copy(cloudflareBypassState = cfState) }
    }

    internal fun tabIdentityLayersForTest(): BrowserTabIdentityLayers? {
        val manager = tabManager ?: return null
        val ui = _uiState.value
        val (storeIds, storeActiveId) = manager.tabStoreIdentityForTest()
        return BrowserTabIdentityLayers(
            tabStoreIds = storeIds,
            tabStoreActiveId = storeActiveId,
            tabManagerIds = manager.allTabStates().map { it.id },
            tabManagerActiveId = manager.activeTabId(),
            viewModelIds = ui.tabs.map { it.id },
            viewModelActiveId = ui.activeTabId,
        )
    }

    private fun updateNavigationState(tabId: String, tab: TabState) {
        _uiState.update { state ->
            if (state.tabs.none { it.id == tabId }) return@update state
            state.copy(tabs = state.tabs.map { existing ->
                if (existing.id == tabId) tab else existing
            })
        }
        tabManager?.let { manager -> saveCurrentTabs(manager.allTabStates(), manager.activeTabId()) }
    }

    // ──Tab actions ────────────────────────────────────────────────────────

    fun newTab(url: String = "about:blank", spaceId: String = "default") {
        tabManager?.createTab(url, spaceId)
        reattachActiveSession()
    }

    fun closeTab(tabId: String) {
        tabManager?.closeTab(tabId)
        reattachActiveSession()
    }

    fun closeAllTabs(spaceId: String? = null) {
        tabManager?.closeAllTabs(spaceId)
        reattachActiveSession()
    }

    fun selectTab(tabId: String) {
        tabManager?.selectTab(tabId)
        reattachActiveSession()
    }

    fun clearPendingDownload() {
        _uiState.update { it.copy(pendingDownloads = it.pendingDownloads.drop(1)) }
    }

    /**
     * Invoked when a magnet: URI is intercepted by GeckoView.
     * Submits the magnet to the download engine and updates [BrowserUiState.magnetQueued] on
     * success or [BrowserUiState.magnetError] on failure.
     */
    fun onMagnetRequested(magnetUri: String) {
        val repo = dev.fetchx.bridge.BridgeInitializer.repositoryOrNull() ?: run {
            Log.w(TAG, "onMagnetRequested: bridge not initialized")
            _uiState.update { it.copy(magnetError = "The download service is not ready.") }
            return
        }
        viewModelScope.launch(Dispatchers.IO) {
            try {
                repo.addMagnet(magnetUri, dev.fetchx.core.download.DownloadPriority.NORMAL)
                Log.i(TAG, "Magnet download queued")
                _uiState.update { it.copy(magnetQueued = magnetUri, magnetError = null) }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (e: Exception) {
                Log.e(TAG, "Failed to queue magnet download", e)
                _uiState.update { it.copy(magnetError = e.message ?: "Could not queue magnet download.") }
            }
        }
    }

    fun clearMagnetState() {
        _uiState.update { it.copy(magnetQueued = null, magnetError = null) }
    }

    // ── Cloudflare bypass ─────────────────────────────────────────────────

    private var cfBypassTimeoutJob: Job? = null

    /** Returns the [CloudflareBypassController] for the active tab (for BrowserScreen use). */
    fun activeCfController(): CloudflareBypassController? = tabManager?.activeCfController()

    /**
     * Called by a [LaunchedEffect] in BrowserScreen that collects the active tab's bypass
     * controller StateFlow and pushes state updates here.
     */
    fun onCfBypassStateChanged(state: CloudflareBypassState) {
        _uiState.update { it.copy(cloudflareBypassState = state) }
        when (state) {
            is CloudflareBypassState.Bypassing -> {
                // Start a 30-second timeout — most challenges resolve in < 5 s.
                cfBypassTimeoutJob?.cancel()
                cfBypassTimeoutJob = viewModelScope.launch {
                    delay(30_000L)
                    tabManager?.activeCfController()
                        ?.onChallengeFailed("Timed out waiting for ${state.provider.displayName}")
                }
            }
            else -> {
                cfBypassTimeoutJob?.cancel()
                cfBypassTimeoutJob = null
            }
        }
    }

    /** Resets bypass state for the active tab and returns to Idle. */
    fun resetCloudflareBypass() {
        tabManager?.activeCfController()?.reset()
        cfBypassTimeoutJob?.cancel()
        cfBypassTimeoutJob = null
        _uiState.update { it.copy(cloudflareBypassState = CloudflareBypassState.Idle) }
    }

    // ──Navigation ─────────────────────────────────────────────────────────

    fun navigate(rawUrl: String) {
        clearDetectedVideos()
        if (tabRestoreNavigationGate.defer(rawUrl)) return
        tabManager?.navigate(rawUrl, _uiState.value.selectedSearchEngine)
    }

    fun goBack(): Boolean = tabManager?.goBack() == true
    fun goForward(): Boolean = tabManager?.goForward() == true
    fun reload() { tabManager?.reload() }
    fun stopLoading() { tabManager?.stopLoading() }

    fun openFindInPage(): Boolean {
        val tab = tabManager?.activeTabState() ?: return false
        val previous = _uiState.value.findInPage
        if (previous != null) tabManager?.clearFindInPage(previous.tabId)
        findRequestGeneration += 1
        _uiState.update {
            it.copy(
                showTabOverview = false,
                findInPage = FindInPageState(tab.id, tab.url),
            )
        }
        return true
    }

    fun updateFindQuery(query: String) {
        val findState = _uiState.value.findInPage ?: return
        findInPage(query, FindDirection.FIRST, findState)
    }

    fun findNextMatch() {
        val findState = _uiState.value.findInPage ?: return
        findInPage(findState.query, FindDirection.NEXT, findState)
    }

    fun findPreviousMatch() {
        val findState = _uiState.value.findInPage ?: return
        findInPage(findState.query, FindDirection.PREVIOUS, findState)
    }

    fun closeFindInPage() {
        val findState = _uiState.value.findInPage ?: return
        findRequestGeneration += 1
        tabManager?.clearFindInPage(findState.tabId)
        _uiState.update { it.copy(findInPage = null) }
    }

    private fun findInPage(
        query: String,
        direction: FindDirection,
        findState: FindInPageState,
    ) {
        if (findState.tabId != tabManager?.activeTabId()) {
            closeFindInPage()
            return
        }
        val generation = ++findRequestGeneration
        _uiState.update {
            val current = it.findInPage
            if (current?.tabId != findState.tabId) it
            else it.copy(
                findInPage = current.copy(
                    query = query,
                    currentMatch = 0,
                    totalMatches = 0,
                    found = false,
                    error = null,
                ),
            )
        }
        tabManager?.findInPage(findState.tabId, query, direction) { result, error ->
            if (generation != findRequestGeneration) return@findInPage
            _uiState.update { state ->
                val current = state.findInPage ?: return@update state
                if (current.tabId != findState.tabId || current.query != query) return@update state
                state.copy(
                    findInPage = current.copy(
                        currentMatch = result?.current ?: 0,
                        totalMatches = result?.total ?: 0,
                        found = result?.found == true,
                        error = error,
                    ),
                )
            }
        }
    }

    fun toggleDesktopSite() {
        val context = applicationContext ?: return
        val tab = tabManager?.activeTabState() ?: run {
            showBrowserActionMessage(context, "No active page.")
            return
        }
        val enabled = !tab.desktopSiteEnabled
        try {
            if (tabManager?.setDesktopSiteEnabled(tab.id, enabled) != true) {
                showBrowserActionMessage(context, "Desktop site is only available for web pages.")
            }
        } catch (error: RuntimeException) {
            Log.e(TAG, "Could not change desktop-site mode for tab ${tab.id}", error)
            showBrowserActionMessage(context, "Could not change desktop-site mode.")
        }
    }

    fun shareCurrentPage() {
        val context = applicationContext ?: return
        val tab = tabManager?.activeTabState()
        if (tab == null || !isHttpOrHttpsPageUrl(tab.url)) {
            showBrowserActionMessage(context, "This page cannot be shared.")
            return
        }
        val title = tab.title?.takeIf { it.isNotBlank() }
        val text = if (title == null || title == tab.url) tab.url else "$title\n${tab.url}"
        val sendIntent = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, text)
            if (title != null) putExtra(Intent.EXTRA_TITLE, title)
        }
        try {
            context.startActivity(
                Intent.createChooser(sendIntent, "Share page")
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        } catch (error: ActivityNotFoundException) {
            Log.w(TAG, "No Android share target is available", error)
            showBrowserActionMessage(context, "No app can share this page.")
        } catch (error: RuntimeException) {
            Log.e(TAG, "Could not open the Android share sheet", error)
            showBrowserActionMessage(context, "Could not share this page.")
        }
    }

    fun copyCurrentPageUrl() {
        val context = applicationContext ?: return
        val tab = tabManager?.activeTabState()
        if (tab == null || !isHttpOrHttpsPageUrl(tab.url)) {
            showBrowserActionMessage(context, "This page has no copyable web address.")
            return
        }
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        if (clipboard == null) {
            showBrowserActionMessage(context, "Clipboard is unavailable.")
            return
        }
        val label = tab.title?.takeIf { it.isNotBlank() } ?: "Page URL"
        try {
            clipboard.setPrimaryClip(ClipData.newPlainText(label, tab.url))
            showBrowserActionMessage(context, "URL copied")
        } catch (error: RuntimeException) {
            Log.e(TAG, "Could not copy the current page URL", error)
            showBrowserActionMessage(context, "Could not copy this page URL.")
        }
    }

    fun openCurrentPageExternally() {
        val context = applicationContext ?: return
        val tab = tabManager?.activeTabState()
        if (tab == null || !isHttpOrHttpsPageUrl(tab.url)) {
            showBrowserActionMessage(context, "This page cannot be opened externally.")
            return
        }
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse(tab.url))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            context.startActivity(intent)
        } catch (error: ActivityNotFoundException) {
            Log.w(TAG, "No external browser is available for ${tab.url}", error)
            showBrowserActionMessage(context, "No external browser can open this page.")
        } catch (error: RuntimeException) {
            Log.e(TAG, "Could not open the page externally", error)
            showBrowserActionMessage(context, "Could not open this page externally.")
        }
    }

    private fun showBrowserActionMessage(context: Context, message: String) {
        android.widget.Toast.makeText(context, message, android.widget.Toast.LENGTH_SHORT).show()
    }

    // ──History ────────────────────────────────────────────────────────────

    fun openHistory() {
        _uiState.update { it.copy(showHistory = true) }
    }

    fun closeHistory() {
        _uiState.update { it.copy(showHistory = false) }
    }

    fun clearHistory() {
        tabManager?.clearAllHistory()
        _uiState.update { it.copy(historyEntries = emptyList()) }
    }

    fun removeHistoryEntry(id: String) {
        tabManager?.removeHistoryEntry(id)
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

    // ──Zenium features ────────────────────────────────────────────────────

    // Spaces
    fun openSpaceSwitcher() { _uiState.update { it.copy(showSpaceSwitcher = true) } }
    fun closeSpaceSwitcher() { _uiState.update { it.copy(showSpaceSwitcher = false) } }

    fun newTabInCurrentSpace(url: String = "about:blank") {
        val spaceId = _uiState.value.selectedProfileId
        newTab(url, spaceId)
    }

    // Essentials
    fun navigateToEssential(url: String) { tabManager?.navigate(url) }
    // Theme color picker
    fun openThemePicker() { _uiState.update { it.copy(showThemePicker = true) } }
    fun closeThemePicker() { _uiState.update { it.copy(showThemePicker = false) } }

    // Compact mode
    fun toggleCompactMode() { _uiState.update { it.copy(compactMode = !it.compactMode) } }

    // ── Video detection ───────────────────────────────────────────────────

    fun onVideoDetected(video: DetectedVideo) {
        _uiState.update { state ->
            if (state.detectedVideos.any { it.url == video.url }) return@update state
            state.copy(
                detectedVideos = state.detectedVideos + video,
                showVideoDownloadBar = true,
            )
        }
    }

    fun dismissVideoBar() {
        _uiState.update { it.copy(showVideoDownloadBar = false) }
    }

    fun clearDetectedVideos() {
        _uiState.update { it.copy(detectedVideos = emptyList(), showVideoDownloadBar = false) }
    }

    fun requestMediaFormats(video: DetectedVideo) {
        mediaFormatGeneration += 1
        val generation = mediaFormatGeneration
        mediaFormatJob?.cancel()
        val repo = dev.fetchx.bridge.BridgeInitializer.repositoryOrNull() ?: run {
            _uiState.update {
                it.copy(
                    pendingMediaVideo = video,
                    isDiscoveringMediaFormats = false,
                    mediaFormats = emptyList(),
                    mediaFormatError = "The download service is not ready.",
                )
            }
            return
        }
        val pageUrl = _uiState.value.activeTab?.url
            ?.takeIf { it.startsWith("http") }
        _uiState.update {
            it.copy(
                pendingMediaVideo = video,
                mediaFormats = emptyList(),
                isDiscoveringMediaFormats = true,
                mediaFormatError = null,
            )
        }
        mediaFormatJob = viewModelScope.launch(Dispatchers.IO) {
            try {
                var formats: List<dev.fetchx.core.download.YtDlpFormat> = emptyList()
                var discoveryError: Exception? = null
                // 1. Try pageUrl first (allows yt-dlp to find all qualities/audio-merge formats)
                if (pageUrl != null) {
                    try {
                        formats = repo.discoverMediaFormats(pageUrl)
                    } catch (cancelled: CancellationException) {
                        throw cancelled
                    } catch (e: Exception) {
                        discoveryError = e
                        Log.w(TAG, "Format discovery on pageUrl failed, falling back to stream url", e)
                    }
                }
                // 2. If pageUrl discovery was empty or failed, try direct video stream url
                if (formats.isEmpty() && video.url.isNotBlank() && video.url != pageUrl) {
                    try {
                        formats = repo.discoverMediaFormats(video.url)
                    } catch (cancelled: CancellationException) {
                        throw cancelled
                    } catch (e: Exception) {
                        if (discoveryError == null) discoveryError = e
                        Log.w(TAG, "Format discovery on video.url failed", e)
                    }
                }
                _uiState.update { state ->
                    if (generation != mediaFormatGeneration) return@update state
                    state.copy(
                        mediaFormats = formats,
                        isDiscoveringMediaFormats = false,
                        mediaFormatError = if (formats.isEmpty() && discoveryError != null)
                            discoveryError.message ?: "Format discovery failed."
                        else null,
                    )
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                Log.e(TAG, "Failed to discover media formats", error)
                _uiState.update { state ->
                    if (generation != mediaFormatGeneration) return@update state
                    state.copy(
                        isDiscoveringMediaFormats = false,
                        mediaFormatError = error.message ?: "Format discovery failed.",
                    )
                }
            }
        }
    }

    fun closeMediaFormatPicker() {
        mediaFormatGeneration += 1
        mediaFormatJob?.cancel()
        mediaFormatJob = null
        _uiState.update {
            it.copy(
                pendingMediaVideo = null,
                mediaFormats = emptyList(),
                isDiscoveringMediaFormats = false,
                mediaFormatError = null,
                mediaQueueError = null,
                isQueueingMedia = false,
            )
        }
    }

    fun downloadDetectedVideo(
        video: DetectedVideo,
        formatId: String? = null,
        extension: String? = null,
        formatLabel: String? = null,
        videoTitle: String? = null,
        platformName: String? = null,
    ) {
        val repo = dev.fetchx.bridge.BridgeInitializer.repositoryOrNull() ?: run {
            Log.w(TAG, "downloadDetectedVideo: bridge not initialized")
            _uiState.update { it.copy(mediaQueueError = "The download service is not ready.") }
            return
        }
        val destDir = applicationContext?.getExternalFilesDir(
            android.os.Environment.DIRECTORY_DOWNLOADS
        ) ?: android.os.Environment.getExternalStoragePublicDirectory(
            android.os.Environment.DIRECTORY_DOWNLOADS,
        )
        val pageUrl = _uiState.value.activeTab?.url
            ?.takeIf { it.startsWith("http") }
        val route = resolveMediaDownloadRoute(
            videoUrl = video.url,
            pageUrl = pageUrl,
            formatId = formatId,
            hasDiscoveredFormats = _uiState.value.mediaFormats.isNotEmpty(),
        )
        val isDirectMedia = route.isDirectMedia
        val downloadUrl = route.url
        val discoveredMetadata = _uiState.value.mediaFormats
            .firstOrNull { it.formatId == formatId }
            ?: _uiState.value.mediaFormats.firstOrNull()
        val rawTitle = _uiState.value.activeTab?.title
            ?.takeIf { it.isNotBlank() && !it.startsWith("http://") && !it.startsWith("https://") }
        val title = videoTitle
            ?.takeIf { it.isNotBlank() }
            ?: discoveredMetadata?.title
                ?.takeIf { it.isNotBlank() }
            ?: rawTitle
            ?: video.suggestedFilename
        val platform = platformName
            ?.takeIf { it.isNotBlank() }
            ?: discoveredMetadata?.platform
                ?.takeIf { it.isNotBlank() }
            ?: runCatching {
                java.net.URI(downloadUrl).host?.removePrefix("www.")
            }.getOrNull()
        val desiredName = buildMediaFileName(
            title = title,
            quality = formatLabel,
            platform = platform,
            extension = extension,
        )
        val fileName = generateUniqueFileName(destDir, desiredName)
        val destPath = java.io.File(destDir, fileName).absolutePath
        _uiState.update { it.copy(isQueueingMedia = true, mediaQueueError = null) }
        viewModelScope.launch(Dispatchers.IO) {
            try {
                // Direct media URLs use native engines; page URLs use yt-dlp extraction.
                repo.addMediaDownload(
                    url = downloadUrl,
                    fileName = fileName,
                    destinationPath = destPath,
                    priority = dev.fetchx.core.download.DownloadPriority.NORMAL,
                    formatId = formatId,
                )
                val engineLabel = if (isDirectMedia) "native engine" else "yt-dlp"
                Log.i(TAG, "Queuing media download via $engineLabel: url=$downloadUrl fileName=$fileName")
                withContext(Dispatchers.Main) { closeMediaFormatPicker() }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (e: Exception) {
                Log.e(TAG, "Failed to queue video download: ${e.javaClass.simpleName}: ${e.message}", e)
                _uiState.update {
                    it.copy(
                        isQueueingMedia = false,
                        mediaQueueError = "Error: ${e.message ?: "Could not queue video download."}",
                    )
                }
            }
        }
        dismissVideoBar()
    }

    fun openTrackerDetails() {
        _uiState.update { it.copy(showTrackerDetails = true) }
    }

    internal fun enqueueSitePermission(
        origin: String,
        permission: String,
        detail: String?,
        resolve: (Boolean) -> Unit,
    ) {
        val prompt = SitePermissionPrompt(
            id = UUID.randomUUID().toString(),
            origin = origin,
            permission = permission,
            detail = detail,
        )
        synchronized(permissionLock) {
            sitePermissionQueue.addLast(PendingSitePermission(prompt, resolve))
            if (sitePermissionQueue.size == 1) {
                _uiState.update { it.copy(sitePermissionPrompt = prompt) }
            }
        }
    }

    fun resolveSitePermissionPrompt(promptId: String, allowed: Boolean) {
        val pending: PendingSitePermission
        val nextPrompt: SitePermissionPrompt?
        synchronized(permissionLock) {
            val active = sitePermissionQueue.firstOrNull() ?: return
            if (active.prompt.id != promptId) return
            pending = sitePermissionQueue.removeFirst()
            nextPrompt = sitePermissionQueue.firstOrNull()?.prompt
            _uiState.update { it.copy(sitePermissionPrompt = nextPrompt) }
        }
        pending.resolve(allowed)
    }

    private fun requestAndroidRuntimePermissions(
        permissions: Array<out String>,
        resolve: (Boolean) -> Unit,
    ) {
        val requested = permissions.distinct()
        val context = applicationContext
        if (context == null || requested.isEmpty() || requested.any { !isSupportedAndroidRuntimePermission(it) }) {
            resolve(false)
            return
        }
        val missing = requested.filter {
            context.checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED
        }
        if (missing.isEmpty()) {
            resolve(true)
            return
        }

        synchronized(permissionLock) {
            val request = AndroidRuntimePermissionRequest(
                id = UUID.randomUUID().toString(),
                permissions = missing,
            )
            androidPermissionQueue.addLast(PendingAndroidRuntimePermission(request, requested, resolve))
            _uiState.update {
                it.copy(androidRuntimePermissionRequests = androidPermissionQueue.map { pending -> pending.request })
            }
        }
    }

    fun resolveNextAndroidRuntimePermissionRequest(results: Map<String, Boolean>) {
        val pending: PendingAndroidRuntimePermission
        val nextRequests: List<AndroidRuntimePermissionRequest>
        val context = applicationContext
        synchronized(permissionLock) {
            pending = androidPermissionQueue.pollFirst() ?: return
            nextRequests = androidPermissionQueue.map { it.request }
            _uiState.update { it.copy(androidRuntimePermissionRequests = nextRequests) }
        }
        val granted = context != null &&
            areAndroidRuntimePermissionsGranted(pending.requestedPermissions) { permission ->
                results[permission] == true ||
                    context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
            }
        pending.resolve(granted)
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

    fun setAdBlockingEnabled(enabled: Boolean) {
        privacyController.setAdBlockingEnabled(enabled)
        applyPrivacySettings()
        syncUBlockWithAdBlockingSetting()
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

    fun clickFetchxRemoteAction(): Boolean = GeckoRuntimeHolder.clickFetchxRemoteAction()

    fun closeFetchxRemotePopup() {
        GeckoRuntimeHolder.closeFetchxRemotePopup()
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

    fun searchFirefoxAddons(query: String) {
        val generation = ++firefoxAddonsSearchGeneration
        _uiState.update {
            it.copy(
                firefoxAddonsCatalogState = it.firefoxAddonsCatalogState.copy(
                    query = query,
                    results = emptyList(),
                    isLoading = true,
                    error = null,
                ),
            )
        }
        viewModelScope.launch {
            try {
                val results = withContext(Dispatchers.IO) {
                    firefoxAddonsCatalog.search(query)
                }
                if (generation == firefoxAddonsSearchGeneration) {
                    _uiState.update {
                        it.copy(
                            firefoxAddonsCatalogState = it.firefoxAddonsCatalogState.copy(
                                results = results,
                                isLoading = false,
                                error = null,
                            ),
                        )
                    }
                }
            } catch (error: IOException) {
                if (generation == firefoxAddonsSearchGeneration) {
                    _uiState.update {
                        it.copy(
                            firefoxAddonsCatalogState = it.firefoxAddonsCatalogState.copy(
                                results = emptyList(),
                                isLoading = false,
                                error = error.message ?: "Firefox Add-ons search failed.",
                            ),
                        )
                    }
                }
            } catch (error: IllegalArgumentException) {
                if (generation == firefoxAddonsSearchGeneration) {
                    _uiState.update {
                        it.copy(
                            firefoxAddonsCatalogState = it.firefoxAddonsCatalogState.copy(
                                results = emptyList(),
                                isLoading = false,
                                error = error.message ?: "Enter an add-on name to search.",
                            ),
                        )
                    }
                }
            }
        }
    }

    fun installFirefoxAddon(addon: FirefoxAddon) {
        val manager = extensionManager
        if (manager == null) {
            _uiState.update {
                it.copy(extensionManagerState = it.extensionManagerState.copy(
                    error = "The browser extension service is not available.",
                ))
            }
            return
        }
        manager.installFromFirefoxAddons(addon)
    }

    fun resolveExtensionPermissionRequest(allow: Boolean) {
        extensionManager?.resolveInstallPermissionRequest(allow)
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
        saveBookmarks(bState.bookmarks)
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
        savePrivacySettings(settings)
    }

    private fun syncExtensionManagerState() {
        val manager = extensionManager ?: return
        val state = manager.state.value
        _uiState.update { it.copy(extensionManagerState = state) }
        syncUBlockWithAdBlockingSetting()
    }

    private fun syncUBlockWithAdBlockingSetting() {
        val manager = extensionManager ?: return
        val managerState = manager.state.value
        if (managerState.isLoading || managerState.isBusy || managerState.isShutdown) return

        val uBlock = managerState.extensions.firstOrNull {
            it.id.value == UBLOCK_ORIGIN_EXTENSION_ID
        } ?: return
        if (uBlockNeedsStateChange(
                extensionState = uBlock.state,
                adBlockingEnabled = privacyController.settings.value.adBlockingEnabled,
            )
        ) {
            if (privacyController.settings.value.adBlockingEnabled) {
                manager.enable(uBlock.id)
            } else {
                manager.disable(uBlock.id)
            }
        }
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
        searchPreferences?.edit()?.putString(SEARCH_ENGINE_KEY, engine.name)?.apply()
        onUrlInputChanged(_uiState.value.urlInput)
    }

    fun setBrowserSuggestionsEnabled(enabled: Boolean) {
        _uiState.update {
            it.copy(
                browserSuggestionsEnabled = enabled,
                searchSuggestions = emptyList(),
                isLoadingSearchSuggestions = false,
                searchSuggestionError = null,
            )
        }
        searchPreferences?.edit()?.putBoolean(LOCAL_SUGGESTIONS_KEY, enabled)?.apply()
        searchSuggestionJob?.cancel()
        searchSuggestionJob = null
    }

    // ──URL input sync ─────────────────────────────────────────────────────

    fun onUrlInputChanged(url: String) {
        searchSuggestionJob?.cancel()
        _uiState.update {
            it.copy(
                urlInput = url,
                searchSuggestions = emptyList(),
                isLoadingSearchSuggestions = it.browserSuggestionsEnabled && url.isNotBlank(),
                searchSuggestionError = null,
                isLocalSearch = false,
                showLocalSearchEmpty = false,
            )
        }
        if (!_uiState.value.browserSuggestionsEnabled || url.isBlank()) return

        val bookmarks = bookmarkController.state.value.bookmarks
        val history = tabManager?.allHistory().orEmpty()
        val tabs = tabManager?.allTabStates().orEmpty()
        searchSuggestionJob = viewModelScope.launch {
            delay(SEARCH_SUGGESTION_DEBOUNCE_MS)
            try {
                val suggestions = withContext(Dispatchers.Default) {
                    buildSuggestions(url, bookmarks, history, tabs, _uiState.value.selectedSearchEngine)
                }
                _uiState.update { state ->
                    if (state.urlInput == url) {
                        state.copy(
                            searchSuggestions = suggestions,
                            isLoadingSearchSuggestions = false,
                        )
                    } else {
                        state
                    }
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                Log.e(TAG, "Could not prepare browser search suggestions", error)
                _uiState.update { state ->
                    if (state.urlInput == url) {
                        state.copy(
                            isLoadingSearchSuggestions = false,
                            searchSuggestionError = "Browser suggestions could not be loaded.",
                        )
                    } else {
                        state
                    }
                }
            }
        }
    }

    fun syncUrlInput(url: String) {
        searchSuggestionJob?.cancel()
        _uiState.update {
            it.copy(
                urlInput = url,
                searchSuggestions = emptyList(),
                isLoadingSearchSuggestions = false,
                searchSuggestionError = null,
                isLocalSearch = false,
                showLocalSearchEmpty = false,
            )
        }
    }

    fun submitSearch(rawInput: String): SearchIntent {
        val query = rawInput.trim()
        val results = localResults(query)
        val engine = _uiState.value.selectedSearchEngine
        val intent = SearchIntentClassifier.classify(
            query,
            results,
            UrlTemplateSearchProvider(engine.searchUrl),
        )
        searchSuggestionJob?.cancel()
        when (intent) {
            SearchIntent.Empty -> {
                _uiState.update { it.copy(isLocalSearch = false, showLocalSearchEmpty = false) }
            }
            is SearchIntent.NavigateUrl -> {
                tabManager?.navigate(intent.url)
                _uiState.update { it.copy(searchSuggestions = emptyList(), isLocalSearch = false) }
            }
            is SearchIntent.WebSearch -> {
                tabManager?.navigate(intent.url)
                _uiState.update { it.copy(searchSuggestions = emptyList(), isLocalSearch = false) }
            }
            is SearchIntent.LocalSearch -> {
                val suggestions = results.map(::suggestionFor).toMutableList()
                suggestions += SearchSuggestion(
                    title = "Search the web for \"$query\"",
                    url = UrlTemplateSearchProvider(engine.searchUrl).searchUrl(query),
                    category = "Web search · ${engine.displayName}",
                )
                _uiState.update {
                    it.copy(
                        urlInput = query,
                        searchSuggestions = suggestions,
                        isLoadingSearchSuggestions = false,
                        searchSuggestionError = null,
                        isLocalSearch = true,
                        showLocalSearchEmpty = results.isEmpty(),
                    )
                }
            }
        }
        return intent
    }

    fun applySuggestion(suggestion: SearchSuggestion) {
        val normalized = suggestion.url.trim()
        if (normalized.isEmpty()) return
        searchSuggestionJob?.cancel()
        _uiState.update {
            it.copy(urlInput = normalized, searchSuggestions = emptyList(), isLocalSearch = false)
        }
        if (suggestion.tabId != null) {
            selectTab(suggestion.tabId)
        } else {
            tabManager?.navigate(normalized)
        }
    }

    // ──Private ────────────────────────────────────────────────────────────

    private fun buildSuggestions(
        rawQuery: String,
        bookmarks: List<Bookmark>,
        history: List<HistoryEntry>,
        tabs: List<TabState>,
        engine: SearchEngine,
    ): List<SearchSuggestion> {
        val query = rawQuery.trim()
        val results = BrowserLocalSearch.search(query, bookmarks, history, tabs)
        val intent = SearchIntentClassifier.classify(
            query,
            results,
            UrlTemplateSearchProvider(engine.searchUrl),
        )
        val suggestions = results.map(::suggestionFor).toMutableList()
        when (intent) {
            SearchIntent.Empty -> suggestions += SearchSuggestion(
                "Search with ${engine.displayName}",
                engine.homepage,
                "Web search provider",
            )
            is SearchIntent.NavigateUrl -> suggestions.add(
                0,
                SearchSuggestion("Open website", intent.url, "Address"),
            )
            is SearchIntent.WebSearch -> suggestions += SearchSuggestion(
                "Search the web for \"${intent.query}\"",
                intent.url,
                "Web search · ${engine.displayName}",
            )
            is SearchIntent.LocalSearch -> suggestions += SearchSuggestion(
                "Search the web for \"${intent.query}\"",
                UrlTemplateSearchProvider(engine.searchUrl).searchUrl(intent.query),
                "Web search · ${engine.displayName}",
            )
        }
        return suggestions.distinctBy { it.tabId ?: it.url }.take(8)
    }

    private fun localResults(query: String): List<BrowserSearchResult> =
        BrowserLocalSearch.search(
            query = query,
            bookmarks = bookmarkController.state.value.bookmarks,
            history = tabManager?.allHistory().orEmpty(),
            tabs = tabManager?.allTabStates().orEmpty(),
        )

    private fun suggestionFor(result: BrowserSearchResult) = SearchSuggestion(
        title = result.title,
        url = result.url,
        category = result.type.label,
        tabId = result.tabId,
    )

    private fun loadSearchSettings(context: Context) {
        val preferences = searchPreferences ?: context.getSharedPreferences(
            SEARCH_PREFERENCES_FILE,
            Context.MODE_PRIVATE,
        ).also { searchPreferences = it }
        val engine = preferences.getString(SEARCH_ENGINE_KEY, null)
            ?.let { stored -> SearchEngine.entries.firstOrNull { it.name == stored } }
            ?: SearchEngine.DUCKDUCKGO
        _uiState.update {
            it.copy(
                selectedSearchEngine = engine,
                browserSuggestionsEnabled = preferences.getBoolean(LOCAL_SUGGESTIONS_KEY, true),
            )
        }
    }

    private fun reattachActiveSession() {
        val view = geckoViewRef?.get() ?: return
        tabManager?.activeSession()?.let { view.setSession(it) }
    }

    private fun openExternalUri(uriString: String) {
        val context = applicationContext ?: return
        try {
            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(uriString)).apply {
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            context.startActivity(intent)
        } catch (e: ActivityNotFoundException) {
            android.widget.Toast.makeText(context, "No app can open this link", android.widget.Toast.LENGTH_LONG).show()
        } catch (e: Exception) {
            Log.e(TAG, "Could not open external app", e)
        }
    }

    private fun loadSavedTabSnapshot(
        store: DurableBrowserTabStore?,
        context: Context,
    ): Pair<List<TabState>, String?> {
        val durableSnapshot = try {
            store?.load()
        } catch (error: IOException) {
            Log.e(TAG, "Could not load durable browser tabs; trying the legacy snapshot", error)
            null
        }
        if (durableSnapshot != null) {
            Log.i(TAG, "Restored browser tab snapshot generation ${durableSnapshot.generation}")
            return durableSnapshot.tabs to durableSnapshot.activeTabId
        }

        val prefs = context.getSharedPreferences(
            BROWSER_DATA_PREFERENCES_FILE,
            Context.MODE_PRIVATE,
        )
        val legacyTabs = BrowserDataCodec.decodeTabs(prefs.getString(SAVED_TABS_KEY, null).orEmpty())
        val legacyActiveId = prefs.getString(SAVED_ACTIVE_TAB_ID_KEY, null)?.takeIf { it.isNotBlank() }
        if (legacyTabs.isNotEmpty()) {
            Log.i(TAG, "Restoring legacy SharedPreferences browser tabs for durable migration")
        }
        return legacyTabs to legacyActiveId
    }

    private fun saveCurrentTabs(tabs: List<TabState>, activeId: String?) {
        tabSnapshotWriteGate.submit(tabs, activeId, ::persistCurrentTabs)
    }

    private fun persistCurrentTabs(tabs: List<TabState>, activeId: String?) {
        val store = getDurableTabStore() ?: return
        store.submit(tabs, activeId)
    }

    private fun getDurableTabStore(): DurableBrowserTabStore? {
        durableTabStore?.let { return it }
        val context = applicationContext ?: return null
        return DurableBrowserTabStore(
            slots = AtomicFileSnapshotSlotStorage(File(context.filesDir, BROWSER_TAB_SNAPSHOT_DIRECTORY)),
            onReadError = { error -> Log.e(TAG, "Browser tab snapshot read failed", error) },
            onWriteResult = { result ->
                when (result) {
                    is WriteResult.Persisted ->
                        Log.d(TAG, "Browser tab snapshot generation ${result.generation} is durable")
                    is WriteResult.Superseded ->
                        Log.d(
                            TAG,
                            "Browser tab snapshot generation ${result.generation} superseded by ${result.byGeneration}",
                        )
                    is WriteResult.Failed ->
                        Log.e(TAG, "Browser tab snapshot generation ${result.generation} was not persisted", result.error)
                }
            },
        ).also { durableTabStore = it }
    }

    private fun loadSavedBookmarks(): List<Bookmark> {
        val prefs = browserDataPreferences ?: applicationContext?.getSharedPreferences(
            BROWSER_DATA_PREFERENCES_FILE,
            Context.MODE_PRIVATE,
        )?.also { browserDataPreferences = it } ?: return emptyList()

        val raw = prefs.getString(SAVED_BOOKMARKS_KEY, null) ?: return emptyList()
        return BrowserDataCodec.decodeBookmarks(raw)
    }

    private fun saveBookmarks(bookmarks: List<Bookmark>) {
        val prefs = browserDataPreferences ?: applicationContext?.getSharedPreferences(
            BROWSER_DATA_PREFERENCES_FILE,
            Context.MODE_PRIVATE,
        )?.also { browserDataPreferences = it } ?: return

        val encoded = BrowserDataCodec.encodeBookmarks(bookmarks)
        prefs.edit().putString(SAVED_BOOKMARKS_KEY, encoded).apply()
    }

    private fun loadSavedHistory(context: Context): List<HistoryEntry> {
        val prefs = context.getSharedPreferences(
            BROWSER_DATA_PREFERENCES_FILE,
            Context.MODE_PRIVATE,
        )
        return BrowserDataCodec.decodeHistory(prefs.getString(SAVED_HISTORY_KEY, null).orEmpty())
    }

    private fun saveHistory(entries: List<HistoryEntry>) {
        val prefs = browserDataPreferences ?: applicationContext?.getSharedPreferences(
            BROWSER_DATA_PREFERENCES_FILE,
            Context.MODE_PRIVATE,
        )?.also { browserDataPreferences = it } ?: return
        prefs.edit().putString(SAVED_HISTORY_KEY, BrowserDataCodec.encodeHistory(entries)).apply()
    }

    private fun loadSavedPrivacySettings() {
        val prefs = browserDataPreferences ?: applicationContext?.getSharedPreferences(
            BROWSER_DATA_PREFERENCES_FILE,
            Context.MODE_PRIVATE,
        )?.also { browserDataPreferences = it } ?: return

        if (!prefs.contains(PRIVACY_AD_BLOCKING_KEY)) return

        val settings = PrivacySettings(
            adBlockingEnabled = prefs.getBoolean(PRIVACY_AD_BLOCKING_KEY, true),
            fingerprintingProtectionEnabled = prefs.getBoolean(PRIVACY_FINGERPRINT_KEY, true),
            cryptominingProtectionEnabled = prefs.getBoolean(PRIVACY_CRYPTOMINING_KEY, true),
            thirdPartyCookieIsolationEnabled = prefs.getBoolean(PRIVACY_COOKIE_ISOLATION_KEY, true),
            httpsOnlyEnabled = prefs.getBoolean(PRIVACY_HTTPS_ONLY_KEY, false),
        )
        privacyController.setAdBlockingEnabled(settings.adBlockingEnabled)
        privacyController.setFingerprintingProtectionEnabled(settings.fingerprintingProtectionEnabled)
        privacyController.setCryptominingProtectionEnabled(settings.cryptominingProtectionEnabled)
        privacyController.setThirdPartyCookieIsolationEnabled(settings.thirdPartyCookieIsolationEnabled)
        privacyController.setHttpsOnlyEnabled(settings.httpsOnlyEnabled)
    }

    private fun savePrivacySettings(settings: PrivacySettings) {
        val prefs = browserDataPreferences ?: applicationContext?.getSharedPreferences(
            BROWSER_DATA_PREFERENCES_FILE,
            Context.MODE_PRIVATE,
        )?.also { browserDataPreferences = it } ?: return

        prefs.edit()
            .putBoolean(PRIVACY_AD_BLOCKING_KEY, settings.adBlockingEnabled)
            .putBoolean(PRIVACY_FINGERPRINT_KEY, settings.fingerprintingProtectionEnabled)
            .putBoolean(PRIVACY_CRYPTOMINING_KEY, settings.cryptominingProtectionEnabled)
            .putBoolean(PRIVACY_COOKIE_ISOLATION_KEY, settings.thirdPartyCookieIsolationEnabled)
            .putBoolean(PRIVACY_HTTPS_ONLY_KEY, settings.httpsOnlyEnabled)
            .apply()
    }

    override fun onCleared() {
        super.onCleared()
        val outstandingSitePermissions: List<PendingSitePermission>
        val outstandingAndroidPermissions: List<PendingAndroidRuntimePermission>
        synchronized(permissionLock) {
            outstandingSitePermissions = sitePermissionQueue.toList()
            outstandingAndroidPermissions = androidPermissionQueue.toList()
            sitePermissionQueue.clear()
            androidPermissionQueue.clear()
            _uiState.update {
                it.copy(
                    sitePermissionPrompt = null,
                    androidRuntimePermissionRequests = emptyList(),
                )
            }
        }
        outstandingSitePermissions.forEach { it.resolve(false) }
        outstandingAndroidPermissions.forEach { it.resolve(false) }
        searchSuggestionJob?.cancel()
        searchSuggestionJob = null
        tabRestoreGuard.dispose()
        tabSnapshotWriteGate.dispose()
        tabRestoreNavigationGate.dispose()
        tabRestoreJob?.cancel()
        tabRestoreJob = null
        mediaFormatJob?.cancel()
        mediaFormatJob = null
        tabManager?.dispose()
        tabManager = null
        extensionManager?.let { manager ->
            if (manager.onStateChanged === extensionStateListener) {
                manager.onStateChanged = null
            }
        }
        extensionStateListener = null
        extensionManager = null
        GeckoRuntimeHolder.closeFetchxRemotePopup()
        GeckoRuntimeHolder.setFetchxRemotePopupSessionListener(null)
        applicationContext = null
        trackerBlockingController.onStateChanged = null
        privacyController.onSettingsChanged = null
        GeckoRuntimeHolder.onVideoDetectedFromExtension = null
        runtime = null
        Log.i(TAG, "BrowserViewModel cleared")
    }

    private fun generateUniqueFileName(dir: java.io.File, desiredName: String): String {
        val sanitized = desiredName.trim()
            .replace(Regex("[/\\\\:*?\"<>|]"), "_")
            .trimStart('.')
            .ifBlank { "download" }
        if (!java.io.File(dir, sanitized).exists()) return sanitized
        val dot = sanitized.lastIndexOf('.')
        val base = if (dot > 0) sanitized.substring(0, dot) else sanitized
        val ext = if (dot > 0) sanitized.substring(dot) else ""
        var n = 1
        while (java.io.File(dir, "$base ($n)$ext").exists()) {
            n++
        }

        return "$base ($n)$ext"
    }

    companion object {
        /** Returns true for URLs that are direct media files, downloadable without yt-dlp. */
        fun isDirectMediaUrl(url: String): Boolean {
            val path = try {
                java.net.URI(url).rawPath.orEmpty()
            } catch (_: Exception) {
                return false
            }
            val extension = path.substringAfterLast('/').substringAfterLast('.', "")
            return extension.equals("mp4", ignoreCase = true) ||
                extension.equals("m4v", ignoreCase = true) ||
                extension.equals("webm", ignoreCase = true) ||
                extension.equals("mkv", ignoreCase = true) ||
                extension.equals("avi", ignoreCase = true) ||
                extension.equals("mov", ignoreCase = true) ||
                extension.equals("ts", ignoreCase = true) ||
                extension.equals("m3u8", ignoreCase = true) ||
                extension.equals("m4a", ignoreCase = true) ||
                extension.equals("mp3", ignoreCase = true) ||
                extension.equals("ogg", ignoreCase = true) ||
                extension.equals("flac", ignoreCase = true) ||
                extension.equals("aac", ignoreCase = true) ||
                extension.equals("wav", ignoreCase = true)
        }

        internal fun resolveMediaDownloadRoute(
            videoUrl: String,
            pageUrl: String?,
            formatId: String?,
            hasDiscoveredFormats: Boolean,
        ): MediaDownloadRoute {
            val isDirectMedia = formatId == null && isDirectMediaUrl(videoUrl)
            val url = when {
                isDirectMedia -> videoUrl
                pageUrl != null && (formatId != null || hasDiscoveredFormats) -> pageUrl
                else -> videoUrl.takeIf { it.isNotBlank() } ?: pageUrl.orEmpty()
            }
            return MediaDownloadRoute(url, isDirectMedia)
        }

        private const val SEARCH_PREFERENCES_FILE = "fetchx_browser_search"
        private const val SEARCH_ENGINE_KEY = "search_engine"
        private const val LOCAL_SUGGESTIONS_KEY = "browser_suggestions_enabled"
        const val SEARCH_SUGGESTION_DEBOUNCE_MS = 300L

        private const val BROWSER_DATA_PREFERENCES_FILE = "fetchx_browser_data"
        private const val SAVED_TABS_KEY = "saved_browser_tabs"
        private const val SAVED_ACTIVE_TAB_ID_KEY = "saved_active_tab_id"
        private const val BROWSER_TAB_SNAPSHOT_DIRECTORY = "browser-tab-snapshots"
        private const val SAVED_BOOKMARKS_KEY = "saved_bookmarks"
        private const val SAVED_HISTORY_KEY = "saved_browser_history"
        private const val PRIVACY_AD_BLOCKING_KEY = "privacy_ad_blocking"
        private const val PRIVACY_FINGERPRINT_KEY = "privacy_fingerprint"
        private const val PRIVACY_CRYPTOMINING_KEY = "privacy_cryptomining"
        private const val PRIVACY_COOKIE_ISOLATION_KEY = "privacy_cookie_isolation"
        private const val PRIVACY_HTTPS_ONLY_KEY = "privacy_https_only"

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
    SearchIntentClassifier.normalizeBrowserUrl(trimmed)?.let { return it }
    return UrlTemplateSearchProvider(searchEngine.searchUrl).searchUrl(trimmed)
}

internal fun looksLikeWebAddress(input: String): Boolean {
    return SearchIntentClassifier.normalizeBrowserUrl(input) != null
}
