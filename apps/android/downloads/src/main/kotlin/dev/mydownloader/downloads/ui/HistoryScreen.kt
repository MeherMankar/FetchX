package dev.fetchx.downloads.ui

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.fetchx.core.download.DownloadState

/** History screen — shows completed downloads from the ViewModel. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HistoryScreen(
    viewModel: DownloadsViewModel,
    onBack: () -> Unit,
) {
    val completed by viewModel.completedDownloads.collectAsStateWithLifecycle()
    val totalBytes = completed.sumOf { it.totalBytes ?: 0L }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Download history") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { innerPadding ->
        Column(modifier = Modifier.padding(innerPadding)) {
            // Summary bar
            if (completed.isNotEmpty()) {
                Surface(
                    modifier = Modifier.fillMaxWidth(),
                    tonalElevation = 1.dp,
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
                        horizontalArrangement = Arrangement.spacedBy(24.dp),
                    ) {
                        Column {
                            Text("Completed", style = MaterialTheme.typography.labelSmall)
                            Text("", style = MaterialTheme.typography.titleMedium)
                        }
                        Column {
                            Text("Total downloaded", style = MaterialTheme.typography.labelSmall)
                            Text(formatHistoryBytes(totalBytes), style = MaterialTheme.typography.titleMedium)
                        }
                    }
                }
            }

            if (completed.isEmpty()) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(
                        "No completed downloads yet.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            } else {
                LazyColumn(
                    contentPadding = PaddingValues(16.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(completed, key = { it.id }) { item ->
                        Card(modifier = Modifier.fillMaxWidth()) {
                            Column(modifier = Modifier.padding(12.dp)) {
                                Text(
                                    item.fileName,
                                    style = MaterialTheme.typography.bodyLarge,
                                    maxLines = 1,
                                )
                                Text(
                                    formatHistoryBytes(item.totalBytes ?: 0L),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

private fun formatHistoryBytes(bytes: Long): String {
    if (bytes < 1024L) return " B"
    val units = listOf("KB", "MB", "GB", "TB")
    var value = bytes.toDouble()
    var unitIndex = -1
    do { value /= 1024.0; unitIndex++ } while (value >= 1024.0 && unitIndex < units.lastIndex)
    return "%.1f %s".format(java.util.Locale.ROOT, value, units[unitIndex])
}
