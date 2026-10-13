package dev.mydownloader.browser

import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import org.mozilla.geckoview.GeckoResult
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoSession
import org.mozilla.geckoview.GeckoView
import org.mozilla.geckoview.WebRequestError

private const val TAG = "BrowserViewModel"

data class BrowserUiState(
    val currentUrl: String = "https://example.com",
    val isLoading: Boolean = false,
    val canGoBack: Boolean = false,
    val canGoForward: Boolean = false,
    val errorText: String? = null,
    val title: String? = null,
)

/**
 * ViewModel managing the browser state and [GeckoSession] lifecycle.
 *
 * Lifecycle ownership:
 * - [GeckoSession] is owned by this ViewModel and survives configuration changes.
 * - [GeckoView] is an Android View and does NOT survive configuration changes.
 * - [attachView] must be called each time the Compose AndroidView factory runs.
 * - When the ViewModel is cleared, the session is closed.
 *
 * [GeckoRuntime] is owned by [GeckoRuntimeHolder] (process-level singleton).
 */
class BrowserViewModel : ViewModel() {

    private val _uiState = MutableStateFlow(BrowserUiState())
    val uiState: StateFlow<BrowserUiState> = _uiState.asStateFlow()

    private var session: GeckoSession? = null
    private var sessionOpened = false
    private var pendingUrl: String? = null

    /**
     * Attaches the [GeckoView] to the current [GeckoSession], creating a new session
     * on first call. Safe to call on every AndroidView factory/update.
     */
    fun attachView(view: GeckoView, runtime: GeckoRuntime) {
        val s = session ?: createSession().also { session = it }
        view.setSession(s)

        if (!sessionOpened) {
            s.open(runtime)
            sessionOpened = true
            Log.i(TAG, "GeckoSession opened")
            val url = pendingUrl ?: _uiState.value.currentUrl
            s.loadUri(url)
            pendingUrl = null
        }
    }

    private fun createSession(): GeckoSession {
        val s = GeckoSession()

        s.contentDelegate = object : GeckoSession.ContentDelegate {
            override fun onTitleChange(session: GeckoSession, title: String?) {
                _uiState.update { it.copy(title = title) }
            }
        }

        s.navigationDelegate = object : GeckoSession.NavigationDelegate {
            override fun onLocationChange(
                session: GeckoSession,
                url: String?,
                perms: MutableList<GeckoSession.PermissionDelegate.ContentPermission>,
                hasUserGesture: Boolean,
            ) {
                _uiState.update { it.copy(currentUrl = url ?: "", errorText = null) }
            }

            override fun onCanGoBack(session: GeckoSession, canGoBack: Boolean) {
                _uiState.update { it.copy(canGoBack = canGoBack) }
            }

            override fun onCanGoForward(session: GeckoSession, canGoForward: Boolean) {
                _uiState.update { it.copy(canGoForward = canGoForward) }
            }

            override fun onLoadError(
                session: GeckoSession,
                uri: String?,
                error: WebRequestError,
            ): GeckoResult<String?>? {
                _uiState.update {
                    it.copy(
                        errorText = "Could not load page. Check your connection and try again.",
                        isLoading = false,
                    )
                }
                return null
            }
        }

        s.progressDelegate = object : GeckoSession.ProgressDelegate {
            override fun onPageStart(session: GeckoSession, url: String) {
                _uiState.update { it.copy(isLoading = true, errorText = null) }
            }

            override fun onPageStop(session: GeckoSession, success: Boolean) {
                _uiState.update { it.copy(isLoading = false) }
            }
        }

        return s
    }

    /** Navigates to [rawUrl], normalizing bare hostnames to https://. */
    fun navigate(rawUrl: String) {
        val url = normalizeUrl(rawUrl)
        val s = session
        if (s != null && sessionOpened) {
            s.loadUri(url)
        } else {
            pendingUrl = url
        }
        _uiState.update { it.copy(currentUrl = url, errorText = null) }
    }

    fun goBack() {
        session?.goBack()
    }

    fun goForward() {
        session?.goForward()
    }

    fun reload() {
        session?.reload()
    }

    override fun onCleared() {
        super.onCleared()
        session?.close()
        session = null
        sessionOpened = false
        Log.i(TAG, "BrowserViewModel cleared — GeckoSession closed")
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
