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
import androidx.compose.material3.Slider
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
import dev.fetchx.downloads.storage.StorageManager

@Composable
internal fun StorageScreen(onBack: () -> Unit, onOpenAria2: () -> Unit = {}) {
    val context = LocalContext.current
    val storageManager = remember(context) { FetchXStorageManager(context) }
    var refreshRequest by remember { mutableIntStateOf(0) }
    var usage by remember { mutableStateOf<FetchXStorageUsage?>(null) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isLoading by remember { mutableStateOf(false) }
    var wifiOnly by remember(context) {
        mutableStateOf(DownloadNetworkPolicy.isWifiOnlyEnabled(context))
    }
    var unmeteredOnly by remember(context) {
        mutableStateOf(DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.UNMETERED_ONLY_KEY))
    }
    var roamingAllowed by remember(context) {
        mutableStateOf(DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.ROAMING_ALLOWED_KEY))
    }
    var chargingOnly by remember(context) {
        mutableStateOf(DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.CHARGING_ONLY_KEY))
    }
    var batteryControl by remember(context) {
        mutableStateOf(DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.BATTERY_CONTROL_KEY))
    }
    var batteryThreshold by remember(context) {
        mutableIntStateOf(DownloadNetworkPolicy.getBatteryThreshold(context))
    }
    val downloadStorageManager = remember(context) { StorageManager(context) }
    var smartFolders by remember(context) {
        mutableStateOf(downloadStorageManager.isSmartFoldersEnabled())
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
        Card(
            modifier = Modifier.fillMaxWidth(),
            onClick = onOpenAria2,
        ) {
            Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                Text("Remote aria2 server", style = MaterialTheme.typography.bodyLarge)
                Text(
                    "Configure an aria2 JSON-RPC connection and test server status.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            PolicySwitch(
                title = "Smart download folders",
                description = "Route files into Videos, Audio, Archives, Documents, or Others automatically.",
                checked = smartFolders,
                onCheckedChange = { enabled ->
                    smartFolders = enabled
                    downloadStorageManager.setSmartFoldersEnabled(enabled)
                },
            )
        }
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
        PolicySwitch(
            title = "Unmetered connections only",
            description = "Pause transfers on metered mobile and hotspot networks.",
            checked = unmeteredOnly,
            onCheckedChange = { enabled ->
                unmeteredOnly = enabled
                DownloadNetworkPolicy.setPolicy(context, DownloadNetworkPolicy.UNMETERED_ONLY_KEY, enabled)
            },
        )
        PolicySwitch(
            title = "Allow downloads while roaming",
            description = "Keep roaming transfers paused unless explicitly allowed.",
            checked = roamingAllowed,
            onCheckedChange = { enabled ->
                roamingAllowed = enabled
                DownloadNetworkPolicy.setPolicy(context, DownloadNetworkPolicy.ROAMING_ALLOWED_KEY, enabled)
            },
        )
        PolicySwitch(
            title = "Download only while charging",
            description = "Pause transfers while the device is running on battery.",
            checked = chargingOnly,
            onCheckedChange = { enabled ->
                chargingOnly = enabled
                DownloadNetworkPolicy.setPolicy(context, DownloadNetworkPolicy.CHARGING_ONLY_KEY, enabled)
            },
        )
        PolicySwitch(
            title = "Battery level control",
            description = "Pause downloads when battery falls below $batteryThreshold%.",
            checked = batteryControl,
            onCheckedChange = { enabled ->
                batteryControl = enabled
                DownloadNetworkPolicy.setPolicy(context, DownloadNetworkPolicy.BATTERY_CONTROL_KEY, enabled)
            },
        )
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                Text("Minimum battery level: $batteryThreshold%", style = MaterialTheme.typography.bodyLarge)
                Slider(
                    value = batteryThreshold.toFloat(),
                    onValueChange = {
                        batteryThreshold = it.toInt()
                        DownloadNetworkPolicy.setBatteryThreshold(context, batteryThreshold)
                    },
                    valueRange = 0f..100f,
                    steps = 19,
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
private fun PolicySwitch(
    title: String,
    description: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.bodyLarge)
                Text(description, style = MaterialTheme.typography.bodySmall)
            }
            Switch(checked = checked, onCheckedChange = onCheckedChange)
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
