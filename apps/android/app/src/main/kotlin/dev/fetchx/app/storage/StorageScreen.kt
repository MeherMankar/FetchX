package dev.fetchx.app.storage

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Card
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.IOException
import dev.fetchx.app.service.DownloadNetworkPolicy

@Composable
internal fun StorageScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val storageManager = remember(context) { FetchXStorageManager(context) }
    var refreshRequest by remember { mutableIntStateOf(0) }
    var usage by remember { mutableStateOf<FetchXStorageUsage?>(null) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isLoading by remember { mutableStateOf(false) }
    var wifiOnly by remember(context) {
        mutableStateOf(DownloadNetworkPolicy.isWifiOnlyEnabled(context))
    }

    LaunchedEffect(refreshRequest) {
        isLoading = true
        errorMessage = null
        try {
            usage = withContext(Dispatchers.IO) { storageManager.getStorageUsage() }
        } catch (error: IOException) {
            errorMessage = error.message ?: "Storage could not be read."
        } catch (error: SecurityException) {
            errorMessage = error.message ?: "Storage access was denied."
        } finally {
            isLoading = false
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            OutlinedButton(onClick = onBack) { Text("Back") }
            OutlinedButton(
                onClick = { refreshRequest++ },
                enabled = !isLoading,
            ) {
                Text("Refresh")
            }
        }
        Text("FetchX storage", style = MaterialTheme.typography.headlineSmall)
        Text(
            "Read-only estimates of app-specific storage. FetchX does not remove files from this screen.",
            style = MaterialTheme.typography.bodyMedium,
        )
        Card(modifier = Modifier.fillMaxWidth()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text("Download only on Wi-Fi", style = MaterialTheme.typography.bodyLarge)
                    Text(
                        "Pause active transfers when Wi-Fi is unavailable, then resume them on Wi-Fi.",
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                Switch(
                    checked = wifiOnly,
                    onCheckedChange = { enabled ->
                        wifiOnly = enabled
                        DownloadNetworkPolicy.setWifiOnlyEnabled(context, enabled)
                    },
                )
            }
        }

        if (isLoading) LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
        errorMessage?.let { message ->
            Text(
                "Storage scan failed: $message",
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        usage?.let { result ->
            StorageUsageRow("Downloads (including partial files)", result.downloadsBytes)
            StorageUsageRow("Browser data (including extensions)", result.browserDataBytes)
            StorageUsageRow("Browser and app cache", result.browserCacheBytes)
            StorageUsageRow("Temporary workspaces", result.temporaryBytes)
            StorageUsageRow("Database", result.databaseBytes)
            StorageUsageRow("Other app data", result.otherAppDataBytes)
            Text(
                "Total: ${formatStorageBytes(result.totalBytes)}",
                style = MaterialTheme.typography.titleLarge,
            )
            Text(
                if (result.customSafLocationConfigured) {
                    "The selected document-provider download folder is outside app storage and is not included."
                } else {
                    "Only app-specific downloads are included. Files saved to other document-provider folders are not measured."
                },
                style = MaterialTheme.typography.bodySmall,
            )
            Text(
                "Browser extensions are included in browser data. This screen does not expose cache-cleanup or workspace-removal controls.",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

@Composable
private fun StorageUsageRow(label: String, bytes: Long) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(label, style = MaterialTheme.typography.bodyLarge)
            Text(formatStorageBytes(bytes), style = MaterialTheme.typography.bodyLarge)
        }
    }
}

internal fun formatStorageBytes(bytes: Long): String {
    if (bytes < 1024L) return "$bytes B"
    val units = listOf("KiB", "MiB", "GiB", "TiB")
    var value = bytes.toDouble()
    var unitIndex = -1
    do {
        value /= 1024.0
        unitIndex++
    } while (value >= 1024.0 && unitIndex < units.lastIndex)
    return "%.1f %s".format(java.util.Locale.ROOT, value, units[unitIndex])
}
