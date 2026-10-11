package dev.mydownloader.downloads.ui

import androidx.lifecycle.ViewModel
import dev.mydownloader.core.download.DownloadItem
import dev.mydownloader.core.download.DownloadState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * ViewModel for the Downloads screen.
 *
 * In Phase 20B/20C this will be wired to a real [dev.mydownloader.core.download.DownloadRepository]
 * implementation backed by the Rust UniFFI bridge. For now it exposes an empty stub.
 */
class DownloadsViewModel(
    // Phase 20B: inject a real DownloadRepository here
    // private val repository: DownloadRepository
) : ViewModel() {

    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    val downloads: StateFlow<List<DownloadItem>> = _downloads.asStateFlow()

    /** Downloads currently active (downloading). */
    val activeDownloads: List<DownloadItem>
        get() = _downloads.value.filter { it.state is DownloadState.Active }

    /** Downloads waiting in the queue. */
    val queuedDownloads: List<DownloadItem>
        get() = _downloads.value.filter { it.state is DownloadState.Queued }

    /** Downloads that finished successfully. */
    val completedDownloads: List<DownloadItem>
        get() = _downloads.value.filter { it.state is DownloadState.Completed }

    /** Downloads that failed. */
    val failedDownloads: List<DownloadItem>
        get() = _downloads.value.filter { it.state is DownloadState.Failed }

    // Phase 20B: collect from repository.observeDownloads() in a viewModelScope coroutine
    // Phase 20B: forward pause/resume/cancel/remove/retry to repository
}
