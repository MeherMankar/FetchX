package dev.fetchx.app.storage

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
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
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.Button
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import android.content.Context
import android.net.Uri
import org.json.JSONObject
import java.io.IOException
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import dev.fetchx.app.service.DownloadNetworkPolicy
import dev.fetchx.app.media.MediaDownloadPreferences
import dev.fetchx.downloads.storage.StorageManager

@OptIn(ExperimentalMaterial3Api::class)
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
    var quickSharedDownloads by remember(context) {
        mutableStateOf(MediaDownloadPreferences.isQuickDownloadEnabled(context))
    }
    val scope = rememberCoroutineScope()
    var backupMessage by remember { mutableStateOf<String?>(null) }
    var backupError by remember { mutableStateOf<String?>(null) }
    val exportLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.CreateDocument("application/json"),
    ) { uri: Uri? ->
        if (uri != null) {
            scope.launch {
                backupError = null
                backupMessage = null
                try {
                    val json = withContext(Dispatchers.IO) { buildSettingsJson(context) }
                    withContext(Dispatchers.IO) {
                        context.contentResolver.openOutputStream(uri)?.use { stream ->
                            stream.write(json.toByteArray(Charsets.UTF_8))
                        } ?: throw IOException("Could not open the selected file for writing.")
                    }
                    backupMessage = "Settings exported."
                } catch (error: Exception) {
                    backupError = error.message ?: "Export failed."
                }
            }
        }
    }
    val importLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri: Uri? ->
        if (uri != null) {
            scope.launch {
                backupError = null
                backupMessage = null
                try {
                    val text = withContext(Dispatchers.IO) {
                        context.contentResolver.openInputStream(uri)?.use { stream ->
                            stream.readBytes().toString(Charsets.UTF_8)
                        } ?: throw IOException("Could not open the selected file for reading.")
                    }
                    withContext(Dispatchers.IO) { applySettingsJson(context, text) }
                    // Reflect the restored preferences in the on-screen state.
                    wifiOnly = DownloadNetworkPolicy.isWifiOnlyEnabled(context)
                    unmeteredOnly =
                        DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.UNMETERED_ONLY_KEY)
                    roamingAllowed =
                        DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.ROAMING_ALLOWED_KEY)
                    chargingOnly =
                        DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.CHARGING_ONLY_KEY)
                    batteryControl =
                        DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.BATTERY_CONTROL_KEY)
                    batteryThreshold = DownloadNetworkPolicy.getBatteryThreshold(context)
                    smartFolders = downloadStorageManager.isSmartFoldersEnabled()
                    quickSharedDownloads = MediaDownloadPreferences.isQuickDownloadEnabled(context)
                    backupMessage = "Settings imported."
                } catch (error: Exception) {
                    backupError = error.message ?: "Import failed."
                }
            }
        }
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

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("FetchX storage") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    IconButton(
                        onClick = { refreshRequest++ },
                        enabled = !isLoading,
                    ) {
                        Icon(Icons.Default.Refresh, contentDescription = "Refresh storage")
                    }
                },
            )
        },
    ) { innerPadding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(innerPadding),
            contentAlignment = androidx.compose.ui.Alignment.TopCenter,
        ) {
            Column(
                modifier = Modifier
                    .widthIn(max = 840.dp)
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 20.dp),
                verticalArrangement = Arrangement.spacedBy(24.dp),
            ) {
                Text(
                    "Read-only estimates of app-specific storage. FetchX does not remove files from this screen.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                StorageSection("Backup & restore") {
                    Text(
                        "Export your FetchX transfer, media, and smart-folder settings to a JSON file, or restore them from a previous export.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Button(onClick = { exportLauncher.launch("fetchx-settings.json") }) {
                            Text("Export")
                        }
                        Button(
                            onClick = { importLauncher.launch(arrayOf("application/json", "*/*")) },
                        ) {
                            Text("Import")
                        }
                    }
                    backupMessage?.let { message ->
                        Text(
                            message,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    backupError?.let { message ->
                        Text(
                            message,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                }
                StorageSection("Download folders") {
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
                StorageSection("Transfer settings") {
                    PolicySwitch(
                        title = "Download only on Wi-Fi",
                        description = "Pause active transfers when Wi-Fi is unavailable, then resume them on Wi-Fi.",
                        checked = wifiOnly,
                        onCheckedChange = { enabled ->
                            wifiOnly = enabled
                            DownloadNetworkPolicy.setWifiOnlyEnabled(context, enabled)
                        },
                    )
                    PolicySwitch(
                        title = "Monitor clipboard for download links",
                        description = "Show a prompt when you copy a URL, so you can add it as a download immediately.",
                        checked = quickSharedDownloads,
                        onCheckedChange = { _ ->
                            // Clipboard monitor preference — stored separately via ClipboardMonitorReceiver
                            android.widget.Toast.makeText(
                                context,
                                "Restart the app to apply clipboard monitoring changes.",
                                android.widget.Toast.LENGTH_SHORT,
                            ).show()
                        },
                    )
                }
                StorageSection("Media downloads") {
                    PolicySwitch(
                        title = "Quick shared links",
                        description = "Open shared video links directly in FetchX’s media download flow.",
                        checked = quickSharedDownloads,
                        onCheckedChange = { enabled ->
                            quickSharedDownloads = enabled
                            MediaDownloadPreferences.setQuickDownloadEnabled(context, enabled)
                        },
                    )
                    Text(
                        "Media extraction uses FetchX's native Rust yt-dlp engine. " +
                            "SponsorBlock, subtitle post-processing, and metadata embedding are not currently available.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                StorageSection("Network conditions") {
                    PolicySwitch(
                        title = "Unmetered connections only",
                        description = "Pause transfers on metered mobile and hotspot networks.",
                        checked = unmeteredOnly,
                        onCheckedChange = { enabled ->
                            unmeteredOnly = enabled
                            DownloadNetworkPolicy.setPolicy(
                                context,
                                DownloadNetworkPolicy.UNMETERED_ONLY_KEY,
                                enabled,
                            )
                        },
                    )
                    PolicySwitch(
                        title = "Allow downloads while roaming",
                        description = "Keep roaming transfers paused unless explicitly allowed.",
                        checked = roamingAllowed,
                        onCheckedChange = { enabled ->
                            roamingAllowed = enabled
                            DownloadNetworkPolicy.setPolicy(
                                context,
                                DownloadNetworkPolicy.ROAMING_ALLOWED_KEY,
                                enabled,
                            )
                        },
                    )
                }
                StorageSection("Battery") {
                    PolicySwitch(
                        title = "Download only while charging",
                        description = "Pause transfers while the device is running on battery.",
                        checked = chargingOnly,
                        onCheckedChange = { enabled ->
                            chargingOnly = enabled
                            DownloadNetworkPolicy.setPolicy(
                                context,
                                DownloadNetworkPolicy.CHARGING_ONLY_KEY,
                                enabled,
                            )
                        },
                    )
                    PolicySwitch(
                        title = "Battery level control",
                        description = "Pause downloads when battery falls below $batteryThreshold%.",
                        checked = batteryControl,
                        onCheckedChange = { enabled ->
                            batteryControl = enabled
                            DownloadNetworkPolicy.setPolicy(
                                context,
                                DownloadNetworkPolicy.BATTERY_CONTROL_KEY,
                                enabled,
                            )
                        },
                    )
                    Card(modifier = Modifier.fillMaxWidth()) {
                        Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                            Text(
                                "Minimum battery level: $batteryThreshold%",
                                style = MaterialTheme.typography.bodyLarge,
                            )
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
                }
                StorageSection("Storage use") {
                    if (isLoading) {
                        LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                    }
                    errorMessage?.let { message ->
                        Text(
                            "Storage scan failed: $message",
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                    usage?.let { result ->
                        Card(modifier = Modifier.fillMaxWidth()) {
                            Column {
                                StorageUsageRow("Downloads (including partial files)", result.downloadsBytes)
                                HorizontalDivider()
                                StorageUsageRow("Browser data (including extensions)", result.browserDataBytes)
                                HorizontalDivider()
                                StorageUsageRow("Browser and app cache", result.browserCacheBytes)
                                HorizontalDivider()
                                StorageUsageRow("Temporary workspaces", result.temporaryBytes)
                                HorizontalDivider()
                                StorageUsageRow("Database", result.databaseBytes)
                                HorizontalDivider()
                                StorageUsageRow("Other app data", result.otherAppDataBytes)
                                HorizontalDivider()
                                Row(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .padding(horizontal = 16.dp, vertical = 12.dp),
                                    horizontalArrangement = Arrangement.SpaceBetween,
                                ) {
                                    Text("Total", style = MaterialTheme.typography.titleMedium)
                                    Text(
                                        formatStorageBytes(result.totalBytes),
                                        style = MaterialTheme.typography.titleMedium,
                                    )
                                }
                            }
                        }
                        Text(
                            if (result.customSafLocationConfigured) {
                                "The selected document-provider download folder is outside app storage and is not included."
                            } else {
                                "Only app-specific downloads are included. Files saved to other document-provider folders are not measured."
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            "Browser extensions are included in browser data. This screen does not expose cache-cleanup or workspace-removal controls.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun StorageSection(
    title: String,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(title, style = MaterialTheme.typography.titleSmall)
        content()
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
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Text(
            formatStorageBytes(bytes),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
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

/**
 * Serialises the current FetchX SharedPreferences (network policy, media, and
 * smart-folder settings) to a versioned JSON string for export.
 */
private fun buildSettingsJson(context: Context): String {
    val json = JSONObject()
    json.put("version", 1)
    json.put("exported_at", isoUtcNow())
    val network = JSONObject()
    network.put("wifi_only", DownloadNetworkPolicy.isWifiOnlyEnabled(context))
    network.put(
        "unmetered_only",
        DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.UNMETERED_ONLY_KEY),
    )
    network.put(
        "roaming_allowed",
        DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.ROAMING_ALLOWED_KEY),
    )
    network.put(
        "charging_only",
        DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.CHARGING_ONLY_KEY),
    )
    network.put(
        "battery_control",
        DownloadNetworkPolicy.isEnabled(context, DownloadNetworkPolicy.BATTERY_CONTROL_KEY),
    )
    network.put("battery_threshold", DownloadNetworkPolicy.getBatteryThreshold(context))
    json.put("network_policy", network)
    val media = JSONObject()
    media.put("quick_download", MediaDownloadPreferences.isQuickDownloadEnabled(context))
    json.put("media", media)
    val storage = JSONObject()
    storage.put("smart_folders", StorageManager(context).isSmartFoldersEnabled())
    json.put("storage", storage)
    return json.toString(2)
}

/**
 * Parses an exported settings JSON string and restores the preferences it
 * contains. Throws [IOException] when the file is malformed or has an
 * unsupported version.
 */
private fun applySettingsJson(context: Context, text: String) {
    val json = JSONObject(text)
    val version = json.optInt("version", -1)
    if (version != 1) {
        throw IOException("Unsupported settings file version: $version")
    }
    json.optJSONObject("network_policy")?.let { network ->
        DownloadNetworkPolicy.setWifiOnlyEnabled(
            context,
            network.optBoolean("wifi_only", DownloadNetworkPolicy.isWifiOnlyEnabled(context)),
        )
        DownloadNetworkPolicy.setPolicy(
            context,
            DownloadNetworkPolicy.UNMETERED_ONLY_KEY,
            network.optBoolean("unmetered_only", false),
        )
        DownloadNetworkPolicy.setPolicy(
            context,
            DownloadNetworkPolicy.ROAMING_ALLOWED_KEY,
            network.optBoolean("roaming_allowed", false),
        )
        DownloadNetworkPolicy.setPolicy(
            context,
            DownloadNetworkPolicy.CHARGING_ONLY_KEY,
            network.optBoolean("charging_only", false),
        )
        DownloadNetworkPolicy.setPolicy(
            context,
            DownloadNetworkPolicy.BATTERY_CONTROL_KEY,
            network.optBoolean("battery_control", false),
        )
        DownloadNetworkPolicy.setBatteryThreshold(context, network.optInt("battery_threshold", 20))
    }
    json.optJSONObject("media")?.let { media ->
        MediaDownloadPreferences.setQuickDownloadEnabled(
            context,
            media.optBoolean("quick_download", true),
        )
    }
    json.optJSONObject("storage")?.let { storage ->
        StorageManager(context).setSmartFoldersEnabled(storage.optBoolean("smart_folders", false))
    }
}

private fun isoUtcNow(): String {
    val format = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.ROOT)
    format.timeZone = TimeZone.getTimeZone("UTC")
    return format.format(Date())
}
