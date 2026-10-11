package dev.mydownloader.downloads.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.weight
import androidx.compose.material3.Card
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.mydownloader.core.download.DownloadItem
import dev.mydownloader.core.download.DownloadState

/**
 * A single row in the downloads list.
 * Displays filename, progress, speed, ETA, engine, and contextual action buttons.
 */
@Composable
fun DownloadItemRow(
    item: DownloadItem,
    onPause: () -> Unit,
    onResume: () -> Unit,
    onCancel: () -> Unit,
    onRemove: () -> Unit,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier
) {
    Card(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp)
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            // File name
            Text(
                text = item.fileName,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )

            Spacer(modifier = Modifier.height(4.dp))

            // Engine badge
            Text(
                text = item.engine,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )

            when (val state = item.state) {
                is DownloadState.Active -> {
                    Spacer(modifier = Modifier.height(6.dp))

                    // Progress bar
                    val progress = if (state.totalBytes != null && state.totalBytes > 0) {
                        state.downloadedBytes.toFloat() / state.totalBytes.toFloat()
                    } else {
                        0f
                    }
                    if (state.totalBytes != null) {
                        LinearProgressIndicator(
                            progress = { progress },
                            modifier = Modifier.fillMaxWidth()
                        )
                    } else {
                        LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                    }

                    Spacer(modifier = Modifier.height(4.dp))

                    // Speed and ETA
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            text = formatSpeed(state.speedBytesPerSec),
                            style = MaterialTheme.typography.labelMedium
                        )
                        if (state.etaSeconds != null) {
                            Text(
                                text = " — ${formatEta(state.etaSeconds)}",
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
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
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(modifier = Modifier.weight(1f))
                        TextButton(onClick = onCancel) { Text("Cancel") }
                    }
                }

                is DownloadState.Paused -> {
                    Spacer(modifier = Modifier.height(4.dp))
                    Row {
                        Text(
                            text = "Paused",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(modifier = Modifier.weight(1f))
                        TextButton(onClick = onResume) { Text("Resume") }
                        TextButton(onClick = onCancel) { Text("Cancel") }
                    }
                }

                is DownloadState.Completed -> {
                    Spacer(modifier = Modifier.height(4.dp))
                    Row {
                        Text(
                            text = "Completed · ${formatBytes(state.totalBytes)}",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.primary
                        )
                        Spacer(modifier = Modifier.weight(1f))
                        TextButton(onClick = onRemove) { Text("Remove") }
                    }
                }

                is DownloadState.Failed -> {
                    Spacer(modifier = Modifier.height(4.dp))
                    Row {
                        Text(
                            text = "Failed: ${state.reason}",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.error,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f)
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
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(modifier = Modifier.weight(1f))
                        TextButton(onClick = onRemove) { Text("Remove") }
                    }
                }
            }
        }
    }
}

private fun formatSpeed(bytesPerSec: Long): String = when {
    bytesPerSec >= 1_048_576 -> String.format("%.1f MB/s", bytesPerSec / 1_048_576.0)
    bytesPerSec >= 1_024 -> String.format("%.0f KB/s", bytesPerSec / 1_024.0)
    else -> "${bytesPerSec} B/s"
}

private fun formatEta(seconds: Long): String = when {
    seconds < 60 -> "${seconds}s"
    seconds < 3_600 -> "${seconds / 60}m ${seconds % 60}s"
    else -> "${seconds / 3_600}h ${(seconds % 3_600) / 60}m"
}

private fun formatBytes(bytes: Long): String = when {
    bytes >= 1_073_741_824 -> String.format("%.2f GB", bytes / 1_073_741_824.0)
    bytes >= 1_048_576 -> String.format("%.1f MB", bytes / 1_048_576.0)
    bytes >= 1_024 -> String.format("%.0f KB", bytes / 1_024.0)
    else -> "${bytes} B"
}
