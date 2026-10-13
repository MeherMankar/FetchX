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

data class BrowserUiState(
    val tabs: List<TabState> = emptyList(),
    val activeTabId: String? = null,
    val urlInput: String = "https://example.com",
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
) {
    val activeTab: TabState? get() = tabs.find { it.id == activeTabId }
}

class BrowserViewModel internal constructor(
    internal val bookmarkController: BookmarkController = BookmarkController(),
    internal val trackerBlockingController: TrackerBlockingController = TrackerBlockingController(),
    internal val privacyController: PrivacyController = PrivacyController(),
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
        val activeUrl = tabs.find { it.id == activeId }?.url ?: "https://example.com"
        val historyEntries = tm.historyForActiveTab()
        bookmarkController.refreshForUrl(activeUrl)
        val bState = bookmarkController.state.value
        val trackerState = trackerBlockingController.state.value
        val trackerStats = trackerState.tabStats[activeId]
        _uiState.update {
            it.copy(
                tabs = tabs,
                activeTabId = activeId,
                urlInput = activeUrl,
                historyEntries = historyEntries,
                isCurrentPageBookmarked = bState.isCurrentPageBookmarked,
                bookmarks = bState.bookmarks,
                trackerBlockingEnabled = trackerState.enabled,
                activeTabBlockedRequests = trackerStats?.blockedRequests ?: 0,
                activeTabBlockedHosts = trackerStats?.blockedHosts.orEmpty(),
            )
        }
    }

    // ──Tab actions ────────────────────────────────────────────────────────

    fun newTab(url: String = "https://example.com") {
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
        tabManager?.navigate(rawUrl)
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

    fun openTrackerDetails() {
        _uiState.update { it.copy(showTrackerDetails = true) }
    }

    fun closeTrackerDetails() {
        _uiState.update { it.copy(showTrackerDetails = false) }
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

    // ──URL input sync ─────────────────────────────────────────────────────

    fun onUrlInputChanged(url: String) {
        _uiState.update { it.copy(urlInput = url) }
    }

    // ──Private ────────────────────────────────────────────────────────────

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
 * Normalizes a user-entered URL string to an absolute HTTPS URL.
 *
 * Examples:
 * - `example.com` → `https://example.com`
 * - `http://example.com` → unchanged
 * - `https://example.com` → unchanged
 * - blank → `https://example.com`
 *
 * Exposed as `internal` for unit testing.
 */
internal fun normalizeUrl(input: String): String {
    val trimmed = input.trim()
    if (trimmed.isBlank()) return "https://example.com"
    if (trimmed.startsWith("http://") || trimmed.startsWith("https://")) return trimmed
    return "https://$trimmed"
}
