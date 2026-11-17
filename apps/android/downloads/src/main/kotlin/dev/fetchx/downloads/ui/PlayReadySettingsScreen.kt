package dev.fetchx.downloads.ui

import android.content.Context
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.edit

private const val PREFS_NAME = "playready_prefs"
private const val KEY_PRD_PATH = "playready_prd_path"
private const val KEY_REMOTE_CDM_URL = "playready_remote_cdm_url"
private const val KEY_REMOTE_CDM_DEVICE = "playready_remote_cdm_device"

private fun prefs(context: Context) =
    context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

/**
 * Settings screen for PlayReady DRM configuration.
 *
 * Exposes three fields:
 * - Path to a `.prd` device file (for use with the remote CDM server when
 *   PlayReady MediaDrm is unavailable on the device).
 * - Base URL of a pyplayready remote CDM server.
 * - Device name registered on that server.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PlayReadySettingsScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val p = prefs(context)

    var prdPath by remember { mutableStateOf(p.getString(KEY_PRD_PATH, "") ?: "") }
    var remoteCdmUrl by remember { mutableStateOf(p.getString(KEY_REMOTE_CDM_URL, "") ?: "") }
    var remoteCdmDevice by remember {
        mutableStateOf(p.getString(KEY_REMOTE_CDM_DEVICE, "") ?: "")
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("PlayReady Settings") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back",
                        )
                    }
                },
            )
        },
    ) { innerPadding ->
        Column(
            modifier = Modifier
                .padding(innerPadding)
                .padding(horizontal = 16.dp, vertical = 8.dp),
        ) {
            Text(
                text = ".prd Device File",
                style = MaterialTheme.typography.titleSmall,
            )
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = "Path to your pyplayready .prd device file. Leave blank to use the " +
                    "platform CDM (unavailable on most Android devices).",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(modifier = Modifier.height(8.dp))
            OutlinedTextField(
                value = prdPath,
                onValueChange = { prdPath = it },
                label = { Text(".prd file path") },
                placeholder = { Text("/storage/emulated/0/Download/device.prd") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.height(16.dp))

            Text(
                text = "Remote CDM Server",
                style = MaterialTheme.typography.titleSmall,
            )
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = "Base URL and device name of a pyplayready remote CDM server. " +
                    "Used when PlayReady MediaDrm is unavailable on this device.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(modifier = Modifier.height(8.dp))
            OutlinedTextField(
                value = remoteCdmUrl,
                onValueChange = { remoteCdmUrl = it },
                label = { Text("Remote CDM URL") },
                placeholder = { Text("http://localhost:4567") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(modifier = Modifier.height(8.dp))
            OutlinedTextField(
                value = remoteCdmDevice,
                onValueChange = { remoteCdmDevice = it },
                label = { Text("Remote CDM device name") },
                placeholder = { Text("device") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.height(8.dp))
            TextButton(
                onClick = {
                    p.edit {
                        putString(KEY_PRD_PATH, prdPath)
                        putString(KEY_REMOTE_CDM_URL, remoteCdmUrl)
                        putString(KEY_REMOTE_CDM_DEVICE, remoteCdmDevice)
                    }
                },
            ) {
                Text("Save")
            }
        }
    }
}
