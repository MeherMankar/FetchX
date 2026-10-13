package dev.fetchx.browser

import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoView
import java.lang.ref.WeakReference

private const val TAG = "BrowserViewModel"

data class BrowserUiState(
    val tabs: List<TabState> = emptyList(),
    val activeTabId: String? = null,
    val urlInput: String = "https://example.com",
    val showHistory: Boolean = false,
    val historyEntries: List<HistoryEntry> = emptyList(),
) {
    val activeTab: TabState? get() = tabs.find { it.id == activeTabId }
}

class BrowserViewModel : ViewModel() {

    private val _uiState = MutableStateFlow(BrowserUiState())
    val uiState: StateFlow<BrowserUiState> = _uiState.asStateFlow()

    private var tabManager: TabManager? = null
    private var geckoViewRef: WeakReference<GeckoView>? = null

    /**
     * Called from the AndroidView factory (and on recomposition).
     * First call: creates the TabManager and the initial tab.
     * Subsequent calls: re-attaches the active session to the (potentially new) GeckoView.
     */
    fun attachView(view: GeckoView, runtime: GeckoRuntime) {
        geckoViewRef = WeakReference(view)
        val tm = tabManager ?: TabManager(runtime).also { newTm ->
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
        _uiState.update {
            it.copy(
                tabs = tabs,
                activeTabId = activeId,
                urlInput = activeUrl,
                historyEntries = historyEntries,
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
