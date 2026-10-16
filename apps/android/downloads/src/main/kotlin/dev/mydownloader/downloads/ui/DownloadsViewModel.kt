package dev.fetchx.downloads.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.DownloadRepository
import dev.fetchx.core.download.DownloadState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * UI state for the Downloads screen.
 *
 * @param isLoading true while an async operation (e.g. addDownload) is in progress.
 * @param error     User-readable error string; null when there is no pending error.
 *                  Never contains stack traces or internal Rust/JNA messages.
 * @param bridgeAvailable false if the bridge failed to initialise; UI shows a degraded-mode banner.
 */
data class DownloadsUiState(
    val isLoading: Boolean = false,
    val error: String? = null,
    val bridgeAvailable: Boolean = true,
)

/**
 * ViewModel for the Downloads screen.
 *
 * Wired to a real [DownloadRepository] implementation backed by the Rust
 * UniFFI bridge. All UI interactions are forwarded to the repository, which
 * delegates to the Rust core (the single source of truth for download state).
 *
 * ViewModels never depend on the JNI/UniFFI layer directly — only on the
 * [DownloadRepository] interface defined in :core.
 */
class DownloadsViewModel(
    private val repository: DownloadRepository,
    bridgeAvailable: Boolean = true,
) : ViewModel() {

    // ── Private UI state ──────────────────────────────────────────────────

    private val _uiState = MutableStateFlow(DownloadsUiState(bridgeAvailable = bridgeAvailable))

    /** Observed by the screen for loading/error/degraded-mode signals. */
    val uiState: StateFlow<DownloadsUiState> = _uiState.asStateFlow()

    // ── Reactive download lists ────────────────────────────────────────────

    /** Reactive list of all downloads. Updated on every Rust event. */
    val downloads: StateFlow<List<DownloadItem>> = repository
        .observeDownloads()
        .stateIn(
            scope = viewModelScope,
            started = SharingStarted.WhileSubscribed(5_000),
            initialValue = emptyList(),
        )

    /** All downloads, sorted newest-first. */
    val allDownloads: StateFlow<List<DownloadItem>> = downloads
        .map { list -> list.sortedByDescending { it.createdAt } }
        .stateIn(
            scope = viewModelScope,
            started = SharingStarted.WhileSubscribed(5_000),
            initialValue = emptyList(),
        )

    /** Downloads currently active (downloading or preparing). */
    val activeDownloads: StateFlow<List<DownloadItem>> = downloads
        .map { list -> list.filter { it.state is DownloadState.Active } }
        .stateIn(
            scope = viewModelScope,
            started = SharingStarted.WhileSubscribed(5_000),
            initialValue = emptyList(),
        )

    /** Downloads waiting in the queue. */
    val queuedDownloads: StateFlow<List<DownloadItem>> = downloads
        .map { list -> list.filter { it.state is DownloadState.Queued } }
        .stateIn(
            scope = viewModelScope,
            started = SharingStarted.WhileSubscribed(5_000),
            initialValue = emptyList(),
        )

    /** Downloads that finished successfully. */
    val completedDownloads: StateFlow<List<DownloadItem>> = downloads
        .map { list -> list.filter { it.state is DownloadState.Completed } }
        .stateIn(
            scope = viewModelScope,
            started = SharingStarted.WhileSubscribed(5_000),
            initialValue = emptyList(),
        )

    /** Downloads that failed. */
    val failedDownloads: StateFlow<List<DownloadItem>> = downloads
        .map { list -> list.filter { it.state is DownloadState.Failed } }
        .stateIn(
            scope = viewModelScope,
            started = SharingStarted.WhileSubscribed(5_000),
            initialValue = emptyList(),
        )

    /** Downloads cancelled by the user. */
    val cancelledDownloads: StateFlow<List<DownloadItem>> = downloads
        .map { list -> list.filter { it.state is DownloadState.Cancelled } }
        .stateIn(
            scope = viewModelScope,
            started = SharingStarted.WhileSubscribed(5_000),
            initialValue = emptyList(),
        )

    // ── Actions ────────────────────────────────────────────────────────────

    /**
     * Add a new download. Errors are mapped to user-readable messages and
     * surfaced via [uiState].error — no stack traces, no internal messages.
     */
    fun addDownload(
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority,
    ) {
        viewModelScope.launch {
            _uiState.update { it.copy(isLoading = true, error = null) }
            try {
                repository.addDownload(
                    url = url,
                    fileName = fileName,
                    destinationPath = destinationPath,
                    priority = priority,
                )
                _uiState.update { it.copy(isLoading = false) }
            } catch (e: Exception) {
                val userMessage = mapAddDownloadError(e)
                _uiState.update { it.copy(isLoading = false, error = userMessage) }
            }
        }
    }

    /** Clear a pending error message shown in the Snackbar. */
    fun clearError() {
        _uiState.update { it.copy(error = null) }
    }

    fun pause(downloadId: String) {
        performAction("pause") { repository.pauseDownload(downloadId) }
    }

    fun resume(downloadId: String) {
        performAction("resume") { repository.resumeDownload(downloadId) }
    }

    fun cancel(downloadId: String) {
        performAction("cancel") { repository.cancelDownload(downloadId) }
    }

    fun remove(downloadId: String) {
        performAction("remove") { repository.removeDownload(downloadId) }
    }

    fun retry(downloadId: String) {
        performAction("retry") { repository.retryDownload(downloadId) }
    }

    fun setPriority(downloadId: String, priority: DownloadPriority) {
        performAction("update priority") { repository.setPriority(downloadId, priority) }
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    private fun performAction(action: String, operation: suspend () -> Unit) {
        viewModelScope.launch {
            try {
                operation()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val message = if (
                    action == "pause" &&
                    e.message?.contains("does not support active pause", ignoreCase = true) == true
                ) {
                    "Pause isn't supported for this download type."
                } else {
                    "Could not $action this download. Please try again."
                }
                _uiState.update { it.copy(error = message) }
            }
        }
    }

    /** Map exceptions from addDownload to user-readable, non-technical strings. */
    private fun mapAddDownloadError(e: Exception): String = when {
        e is IllegalArgumentException ->
            // Do not interpolate e.message — the bridge/Rust layer can produce
            // IllegalArgumentException with internal tokens (panic strings, JNA fragments,
            // state-machine identifiers). Fall through to the URL-check branch or use
            // the generic fallback instead.
            if (e.message?.contains("url", ignoreCase = true) == true)
                "Download could not be added. Please check the URL and try again."
            else
                "Download could not be added. Please try again."
        e.message?.contains("url", ignoreCase = true) == true ||
                e.message?.contains("invalid", ignoreCase = true) == true ->
            "Download could not be added. Please check the URL and try again."
        e.message?.contains("storage", ignoreCase = true) == true ||
                e.message?.contains("disk", ignoreCase = true) == true ->
            "Download could not be added. Storage may be unavailable."
        else ->
            "Download could not be added. Please try again."
    }

    // ── Factory ────────────────────────────────────────────────────────────

    /**
     * Factory for creating [DownloadsViewModel] with the required [DownloadRepository].
     *
     * @param bridgeAvailable pass false when [BridgeInitializer.repositoryOrNull] returned null
     *   so the UI can display the degraded-mode banner.
     */
    class Factory(
        private val repository: DownloadRepository,
        private val bridgeAvailable: Boolean = true,
    ) : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T {
            require(modelClass.isAssignableFrom(DownloadsViewModel::class.java)) {
                "Unknown ViewModel class: ${modelClass.name}"
            }
            return DownloadsViewModel(repository, bridgeAvailable) as T
        }
    }
}
