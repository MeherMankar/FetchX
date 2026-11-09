package dev.fetchx.core.download

/**
 * A download item as seen by the Android UI layer.
 * In Phase 20B this will be populated from the Rust bridge via UniFFI.
 */
data class DownloadItem(
    /** Stable identifier — matches the Rust DownloadId UUID. */
    val id: String,
    /** Display filename (basename only). */
    val fileName: String,
    /** Source URL. */
    val url: String,
    /** Absolute path to the destination file. */
    val destinationPath: String,
    /** Current lifecycle state. */
    val state: DownloadState,
    /** Scheduling/queue priority. */
    val priority: DownloadPriority,
    /** Engine identifier, including historical values loaded from older records. */
    val engine: String,
    /** MIME type if known. */
    val mimeType: String?,
    /** File size in bytes if known. */
    val totalBytes: Long?,
    /** Epoch millis when the download was added. */
    val createdAt: Long
)
