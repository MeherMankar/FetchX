package dev.fetchx.downloads.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.DownloadState

/**
 * A single row in the downloads list.
 * Displays filename, progress, speed, ETA, engine, priority, and contextual action buttons.
 */
@Composable
fun DownloadItemRow(
    item: DownloadItem,
    onPause: () -> Unit,
    onResume: () -> Unit,
    onCancel: () -> Unit,
    onRemove: () -> Unit,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Card(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp),
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            // File name
            Text(
                text = item.fileName,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )

            Spacer(modifier = Modifier.height(4.dp))

            // Engine badge + priority badge on the same row
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = item.engine,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(end = 6.dp),
                )
                PriorityBadge(priority = item.priority)
            }

            when (val state = item.state) {
                is DownloadState.Active -> {
                    Spacer(modifier = Modifier.height(6.dp))

                    // Capture nullable properties as locals so the compiler can smart-cast them.
                    val totalBytes = state.totalBytes
                    val etaSeconds = state.etaSeconds

                    // Progress bar
                    val progress = if (totalBytes != null && totalBytes > 0) {
                        state.downloadedBytes.toFloat() / totalBytes.toFloat()
                    } else {
                        0f
                    }
                    if (totalBytes != null) {
                        LinearProgressIndicator(
                            progress = { progress },
                            modifier = Modifier.fillMaxWidth(),
                        )
                    } else {
                        LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                    }

                    Spacer(modifier = Modifier.height(4.dp))

                    // Percentage + size line
                    val sizeText = if (totalBytes != null && totalBytes > 0) {
                        "${(progress * 100).toInt()}% \u00b7 ${formatBytes(state.downloadedBytes)} / ${formatBytes(totalBytes)}"
                    } else {
                        "${formatBytes(state.downloadedBytes)} downloaded"
                    }
                    Text(
                        text = sizeText,
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )

                    Spacer(modifier = Modifier.height(2.dp))

                    // Speed and ETA
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            text = formatSpeed(state.speedBytesPerSec),
                            style = MaterialTheme.typography.labelMedium,
                        )
                        if (etaSeconds != null) {
                            Text(
                                text = " \u2014 ${formatEta(etaSeconds)}",
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Spacer(modifier = Modifier.weight(1f))
                        TextButton(onClick = onPause) { Text("Pause") }
                        TextButton(onClick = onCancel) { Text("Cancel") }
                    }
                }

                is DownloadState.Queued -> {
                    Spacer(modifier = Modifier.height(4.dp))
                    Row {
                        Text(
                            text = "Queued",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Spacer(modifier = Modifier.weight(1f))
                        TextButton(onClick = onCancel) { Text("Cancel") }
                    }
                }

                is DownloadState.Paused -> {
                    Spacer(modifier = Modifier.height(4.dp))
                    // DownloadState.Paused carries no byte-progress fields — the domain model
                    // does not propagate progress into the paused state. Extending this requires
                    // changes to the Rust models crate and the Android bridge (Phase 20E+).
                    Row {
                        Text(
                            text = "Paused",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Spacer(modifier = Modifier.weight(1f))
                        TextButton(onClick = onResume) { Text("Resume") }
                        TextButton(onClick = onCancel) { Text("Cancel") }
                    }
                }

                is DownloadState.Completed -> {
                    Spacer(modifier = Modifier.height(4.dp))

                    // Show destination file name as a hint.
                    val destFileName = state.destinationPath
                        .substringAfterLast('/')
                        .substringAfterLast('\\')
                        .ifBlank { null }
                    if (destFileName != null) {
                        Text(
                            text = destFileName,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Spacer(modifier = Modifier.height(2.dp))
                    }

                    Row {
                        Text(
                            text = "Completed \u00b7 ${formatBytes(state.totalBytes)}",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.primary,
                        )
                        Spacer(modifier = Modifier.weight(1f))
                        TextButton(onClick = onRemove) { Text("Remove") }
                    }
                }

                is DownloadState.Failed -> {
                    Spacer(modifier = Modifier.height(4.dp))
                    Row {
                        Text(
                            text = "Failed: ${sanitizeFailureReason(state.reason)}",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.error,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                        if (state.retryable) {
                            TextButton(onClick = onRetry) { Text("Retry") }
                        }
                        TextButton(onClick = onRemove) { Text("Remove") }
                    }
                }

                is DownloadState.Cancelled -> {
                    Spacer(modifier = Modifier.height(4.dp))
                    Row {
                        Text(
                            text = "Cancelled",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Spacer(modifier = Modifier.weight(1f))
                        TextButton(onClick = onRemove) { Text("Remove") }
                    }
                }
            }
        }
    }
}

// ── Priority badge ──────────────────────────────────────────────────────────────

@Composable
private fun PriorityBadge(priority: DownloadPriority) {
    val (label, containerColor) = when (priority) {
        DownloadPriority.LOW ->
            "LOW" to MaterialTheme.colorScheme.surfaceVariant
        DownloadPriority.NORMAL ->
            "NORMAL" to MaterialTheme.colorScheme.primaryContainer
        DownloadPriority.HIGH ->
            "HIGH" to MaterialTheme.colorScheme.tertiaryContainer
        DownloadPriority.CRITICAL ->
            // CRITICAL is treated as HIGH at the UI layer — never shown as a separate value.
            "HIGH" to MaterialTheme.colorScheme.tertiaryContainer
    }
    Surface(
        shape = MaterialTheme.shapes.extraSmall,
        color = containerColor,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
        )
    }
}

// ── Formatting helpers ──────────────────────────────────────────────────────────

internal fun formatSpeed(bytesPerSec: Long): String = when {
    bytesPerSec >= 1_048_576 -> String.format("%.1f MB/s", bytesPerSec / 1_048_576.0)
    bytesPerSec >= 1_024 -> String.format("%.0f KB/s", bytesPerSec / 1_024.0)
    else -> "$bytesPerSec B/s"
}

internal fun formatEta(seconds: Long): String = when {
    seconds < 60 -> "${seconds}s"
    seconds < 3_600 -> "${seconds / 60}m ${seconds % 60}s"
    else -> "${seconds / 3_600}h ${(seconds % 3_600) / 60}m"
}

internal fun formatBytes(bytes: Long): String = when {
    bytes >= 1_073_741_824 -> String.format("%.2f GB", bytes / 1_073_741_824.0)
    bytes >= 1_048_576 -> String.format("%.1f MB", bytes / 1_048_576.0)
    bytes >= 1_024 -> String.format("%.0f KB", bytes / 1_024.0)
    else -> "$bytes B"
}

/**
 * Maps a raw failure reason string to a user-readable message.
 *
 * Raw reasons from Rust or the bridge may contain internal state-machine names,
 * JNA exception fragments, or other technical detail that should never be shown directly.
 * This function strips such content and returns a clean, user-facing description.
 */
internal fun sanitizeFailureReason(reason: String): String {
    if (reason.isBlank()) return "Unknown error"

    // Patterns that indicate an internal/technical message — replace entirely.
    val internalPatterns = listOf(
        "java.", "android.", "kotlin.", "uniffi.", "jni", "jna",
        "exception", "nullpointer", "illegalstate", "illegalargument",
        "stacktrace", "at dev.", "caused by", "panicked at",
    )
    val lower = reason.lowercase()
    if (internalPatterns.any { lower.contains(it) }) {
        return "Download failed. Please try again."
    }

    // Translate known Rust-level error tokens to friendly text.
    return when {
        lower.contains("network") || lower.contains("connect") || lower.contains("timeout") ->
            "Network error. Check your connection and retry."
        lower.contains("storage") || lower.contains("disk") || lower.contains("space") ||
                lower.contains("io error") || lower.contains("file") ->
            "Storage error. Check available space and retry."
        lower.contains("not found") || lower.contains("404") ->
            "File not found on the server."
        lower.contains("forbidden") || lower.contains("403") || lower.contains("unauthori") ->
            "Access denied by the server."
        lower.contains("invalid url") || lower.contains("url") ->
            "Invalid download URL."
        lower.contains("cancelled") || lower.contains("canceled") ->
            "Download was cancelled."
        // Reason is short enough to show, but only if it carries no structural signs
        // of an internal origin: colons (e.g. "state_machine: invalid_transition"),
        // snake_case tokens (words joined by underscores, e.g. "invalid_state"),
        // or identifiers typical of Rust/bridge internals.
        // A string that contains neither colons nor underscore-joined lower-case tokens
        // is considered safe to surface verbatim.
        reason.length <= 80 &&
                !reason.contains(':') &&
                !Regex("[a-z]+_[a-z]").containsMatchIn(reason) -> reason
        else -> "Download failed. Please try again."
    }
}

