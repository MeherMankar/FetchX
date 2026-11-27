package dev.fetchx.app.settings

import android.app.admin.DevicePolicyManager
import android.content.ComponentName
import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Divider
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import dev.fetchx.app.service.FetchXDeviceAdminReceiver

private val ACCENT_SWATCHES = listOf(
    null to "Default",
    0xFF87CEEB.toLong() to "Sky",
    0xFF4A90D9.toLong() to "Blue",
    0xFF9B59B6.toLong() to "Purple",
    0xFF27AE60.toLong() to "Green",
    0xFFF39C12.toLong() to "Amber",
    0xFFE74C3C.toLong() to "Red",
    0xFFFD79A8.toLong() to "Pink",
    0xFF00B894.toLong() to "Teal",
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppSettingsScreen(onBack: () -> Unit) {
    val context = LocalContext.current

    // ── Subtitle state ────────────────────────────────────────────────────
    var subtitleEnabled by remember { mutableStateOf(AppPreferences.isSubtitleEnabled(context)) }
    var subtitleLangs by remember { mutableStateOf(AppPreferences.getSubtitleLangs(context)) }

    // ── Webhook state ─────────────────────────────────────────────────────
    var webhookEnabled by remember { mutableStateOf(AppPreferences.isWebhookEnabled(context)) }
    var webhookUrl by remember { mutableStateOf(AppPreferences.getWebhookUrl(context)) }
    var webhookOnComplete by remember { mutableStateOf(AppPreferences.isWebhookOnComplete(context)) }
    var webhookOnFail by remember { mutableStateOf(AppPreferences.isWebhookOnFail(context)) }
    var webhookSecret by remember { mutableStateOf(AppPreferences.getWebhookSecret(context)) }

    // ── Accent color state ────────────────────────────────────────────────
    var accentColor by remember { mutableStateOf(AppPreferences.getAccentColor(context)) }

    // ── Post-queue action state ───────────────────────────────────────────
    var postQueueAction by remember { mutableStateOf(AppPreferences.getPostQueueAction(context)) }

    // ── UPnP state ────────────────────────────────────────────────────────
    var upnpEnabled by remember { mutableStateOf(AppPreferences.isUpnpEnabled(context)) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.Default.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { innerPadding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(innerPadding)
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {

            // ── Subtitle settings ─────────────────────────────────────────
            Text("Subtitles", style = MaterialTheme.typography.titleMedium)
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text("Download subtitles by default", style = MaterialTheme.typography.bodyMedium)
                Switch(
                    checked = subtitleEnabled,
                    onCheckedChange = {
                        subtitleEnabled = it
                        AppPreferences.setSubtitleEnabled(context, it)
                    },
                )
            }
            if (subtitleEnabled) {
                OutlinedTextField(
                    value = subtitleLangs,
                    onValueChange = {
                        subtitleLangs = it
                        AppPreferences.setSubtitleLangs(context, it)
                    },
                    label = { Text("Subtitle languages") },
                    placeholder = { Text("en,es") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            }

            Divider(modifier = Modifier.padding(vertical = 8.dp))

            // ── Accent color picker ───────────────────────────────────────
            Text("Accent color", style = MaterialTheme.typography.titleMedium)
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ACCENT_SWATCHES.forEach { (argb, name) ->
                    val isSelected = if (argb == null) {
                        accentColor == AppPreferences.ACCENT_DEFAULT
                    } else {
                        accentColor == argb
                    }
                    Box(
                        modifier = Modifier
                            .size(36.dp)
                            .clip(CircleShape)
                            .background(
                                if (argb == null) MaterialTheme.colorScheme.primary
                                else Color(argb)
                            )
                            .then(
                                if (isSelected) Modifier.border(
                                    3.dp, MaterialTheme.colorScheme.onSurface, CircleShape,
                                ) else Modifier
                            )
                            .clickable {
                                val newColor = argb ?: AppPreferences.ACCENT_DEFAULT
                                accentColor = newColor
                                AppPreferences.setAccentColor(context, newColor)
                            },
                    )
                }
            }
            Text(
                text = "Restart the app for the accent color to take effect.",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            Divider(modifier = Modifier.padding(vertical = 8.dp))

            // ── Post-queue action ─────────────────────────────────────────
            Text("After queue finishes", style = MaterialTheme.typography.titleMedium)
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth(),
            ) {
                RadioButton(
                    selected = postQueueAction == AppPreferences.POST_QUEUE_NOTHING,
                    onClick = {
                        postQueueAction = AppPreferences.POST_QUEUE_NOTHING
                        AppPreferences.setPostQueueAction(context, AppPreferences.POST_QUEUE_NOTHING)
                    },
                )
                Text("Nothing", modifier = Modifier.padding(start = 4.dp))
            }
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth(),
            ) {
                RadioButton(
                    selected = postQueueAction == AppPreferences.POST_QUEUE_SCREEN_OFF,
                    onClick = {
                        postQueueAction = AppPreferences.POST_QUEUE_SCREEN_OFF
                        AppPreferences.setPostQueueAction(context, AppPreferences.POST_QUEUE_SCREEN_OFF)
                    },
                )
                Text("Turn off screen (requires device admin)", modifier = Modifier.padding(start = 4.dp))
            }
            if (postQueueAction == AppPreferences.POST_QUEUE_SCREEN_OFF) {
                val adminComponent = ComponentName(context, FetchXDeviceAdminReceiver::class.java)
                val dpm = context.getSystemService(android.content.Context.DEVICE_POLICY_SERVICE) as DevicePolicyManager
                if (!dpm.isAdminActive(adminComponent)) {
                    Text(
                        text = "FetchX needs device admin to lock the screen. Tap below to grant.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    TextButton(
                        onClick = {
                            val intent = Intent(DevicePolicyManager.ACTION_ADD_DEVICE_ADMIN).apply {
                                putExtra(DevicePolicyManager.EXTRA_DEVICE_ADMIN, adminComponent)
                                putExtra(
                                    DevicePolicyManager.EXTRA_ADD_EXPLANATION,
                                    "Required to lock screen when queue finishes",
                                )
                            }
                            context.startActivity(intent)
                        },
                    ) {
                        Text("Grant device admin")
                    }
                } else {
                    Text(
                        text = "Device admin granted. Screen will lock when queue finishes.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.tertiary,
                    )
                }
            }

            Divider(modifier = Modifier.padding(vertical = 8.dp))

            // ── Webhook settings ──────────────────────────────────────────
            Text("Webhooks", style = MaterialTheme.typography.titleMedium)
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text("Enable webhooks", style = MaterialTheme.typography.bodyMedium)
                Switch(
                    checked = webhookEnabled,
                    onCheckedChange = {
                        webhookEnabled = it
                        AppPreferences.setWebhookEnabled(context, it)
                    },
                )
            }
            if (webhookEnabled) {
                OutlinedTextField(
                    value = webhookUrl,
                    onValueChange = {
                        webhookUrl = it
                        AppPreferences.setWebhookUrl(context, it)
                    },
                    label = { Text("Webhook URL") },
                    placeholder = { Text("https://example.com/hook") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(
                        checked = webhookOnComplete,
                        onCheckedChange = {
                            webhookOnComplete = it
                            AppPreferences.setWebhookOnComplete(context, it)
                        },
                    )
                    Text("On download complete")
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(
                        checked = webhookOnFail,
                        onCheckedChange = {
                            webhookOnFail = it
                            AppPreferences.setWebhookOnFail(context, it)
                        },
                    )
                    Text("On download fail")
                }
                OutlinedTextField(
                    value = webhookSecret,
                    onValueChange = {
                        webhookSecret = it
                        AppPreferences.setWebhookSecret(context, it)
                    },
                    label = { Text("Secret (optional)") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            }

            Divider(modifier = Modifier.padding(vertical = 8.dp))

            // ── UPnP / Torrent settings ───────────────────────────────────
            Text("Torrent", style = MaterialTheme.typography.titleMedium)
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text("UPnP port forwarding", style = MaterialTheme.typography.bodyMedium)
                    Text(
                        "Takes effect on next app restart.",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(
                    checked = upnpEnabled,
                    onCheckedChange = {
                        upnpEnabled = it
                        AppPreferences.setUpnpEnabled(context, it)
                    },
                )
            }

            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}
