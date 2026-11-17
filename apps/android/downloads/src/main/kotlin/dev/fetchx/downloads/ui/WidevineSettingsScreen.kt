package dev.fetchx.downloads.ui

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
import dev.fetchx.downloads.drm.WidevinePreferences

/**
 * Settings screen for Widevine DRM configuration.
 *
 * Currently exposes a single text field for the user to enter the path to a
 * `.wvd` device file. The path is persisted via [WidevinePreferences] and read
 * back by [dev.fetchx.downloads.drm.WidevineManager] when the .wvd fallback
 * path is active.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WidevineSettingsScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    var wvdPath by remember {
        mutableStateOf(WidevinePreferences.getWvdFilePath(context) ?: "")
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Widevine Settings") },
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
                text = "Widevine Device File (.wvd)",
                style = MaterialTheme.typography.titleSmall,
            )
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = "Enter the full path to a .wvd device file exported from pywidevine. " +
                    "Leave blank to use the platform CDM.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(modifier = Modifier.height(8.dp))
            OutlinedTextField(
                value = wvdPath,
                onValueChange = { wvdPath = it },
                label = { Text(".wvd file path") },
                placeholder = { Text("/storage/emulated/0/Download/device.wvd") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(modifier = Modifier.height(8.dp))
            TextButton(
                onClick = {
                    WidevinePreferences.setWvdFilePath(context, wvdPath)
                },
            ) {
                Text("Save")
            }
        }
    }
}
