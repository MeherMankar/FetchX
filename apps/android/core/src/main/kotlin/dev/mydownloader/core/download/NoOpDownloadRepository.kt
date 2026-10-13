package dev.fetchx.core.download

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * A no-op [DownloadRepository] used when the Rust bridge fails to initialize.
 *
 * All reads return empty/null. All write operations are silent no-ops.
 * This keeps the UI functional (showing empty state) rather than crashing
 * when the native library cannot be loaded.
 */
class NoOpDownloadRepository : DownloadRepository {
    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    private val _events = MutableStateFlow<DownloadEvent?>(null)

    override fun observeDownloads(): Flow<List<DownloadItem>> = _downloads.asStateFlow()
    override fun observeEvents(): Flow<DownloadEvent> =
        // Never emits — cast to Flow<DownloadEvent> via an empty SharedFlow
        kotlinx.coroutines.flow.MutableSharedFlow<DownloadEvent>().asSharedFlow()

    override suspend fun addDownload(
        url: String, fileName: String, destinationPath: String,
        priority: DownloadPriority, mimeType: String?,
        totalBytes: Long?, referrer: String?
    ): String = ""

    override suspend fun pauseDownload(downloadId: String) = Unit
    override suspend fun resumeDownload(downloadId: String) = Unit
    override suspend fun cancelDownload(downloadId: String, deleteFile: Boolean) = Unit
    override suspend fun removeDownload(downloadId: String, deleteFile: Boolean) = Unit
    override suspend fun retryDownload(downloadId: String): String = ""
    override suspend fun setPriority(downloadId: String, priority: DownloadPriority) = Unit
    override suspend fun scheduleDownload(downloadId: String, startAtMillis: Long) = Unit
    override suspend fun getDownload(downloadId: String): DownloadItem? = null
}

