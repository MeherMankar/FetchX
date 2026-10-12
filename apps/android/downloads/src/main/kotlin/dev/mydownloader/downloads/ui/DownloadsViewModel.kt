package dev.mydownloader.downloads.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import dev.mydownloader.core.download.DownloadItem
import dev.mydownloader.core.download.DownloadPriority
import dev.mydownloader.core.download.DownloadRepository
import dev.mydownloader.core.download.DownloadState
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

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
) : ViewModel() {

    /** Reactive list of all downloads. Updated on every Rust event. */
    val downloads: StateFlow<List<DownloadItem>> = repository
        .observeDownloads()
        .stateIn(
            scope = viewModelScope,
            started = SharingStarted.WhileSubscribed(5_000),
            initialValue = emptyList(),
        )

    /** Downloads currently active (downloading or preparing). */
    val activeDownloads: List<DownloadItem>
        get() = downloads.value.filter { it.state is DownloadState.Active }

    /** Downloads waiting in the queue. */
    val queuedDownloads: List<DownloadItem>
        get() = downloads.value.filter { it.state is DownloadState.Queued }

    /** Downloads that finished successfully. */
    val completedDownloads: List<DownloadItem>
        get() = downloads.value.filter { it.state is DownloadState.Completed }

    /** Downloads that failed. */
    val failedDownloads: List<DownloadItem>
        get() = downloads.value.filter { it.state is DownloadState.Failed }

    // ── Actions ───────────────────────────────────────────────────────────

    fun pause(downloadId: String) {
        viewModelScope.launch { repository.pauseDownload(downloadId) }
    }

    fun resume(downloadId: String) {
        viewModelScope.launch { repository.resumeDownload(downloadId) }
    }

    fun cancel(downloadId: String) {
        viewModelScope.launch { repository.cancelDownload(downloadId) }
    }

    fun remove(downloadId: String) {
        viewModelScope.launch { repository.removeDownload(downloadId) }
    }

    fun retry(downloadId: String) {
        viewModelScope.launch { repository.retryDownload(downloadId) }
    }

    fun setPriority(downloadId: String, priority: DownloadPriority) {
        viewModelScope.launch { repository.setPriority(downloadId, priority) }
    }

    // ── Factory ───────────────────────────────────────────────────────────

    /**
     * Factory for creating [DownloadsViewModel] with the required [DownloadRepository].
     * Use this from the Activity/Fragment via [ViewModelProvider].
     */
    class Factory(private val repository: DownloadRepository) : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T {
            require(modelClass.isAssignableFrom(DownloadsViewModel::class.java)) {
                "Unknown ViewModel class: ${modelClass.name}"
            }
            return DownloadsViewModel(repository) as T
        }
    }
}
