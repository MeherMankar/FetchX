package dev.fetchx.browser

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Star
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import org.mozilla.geckoview.GeckoView
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Full-screen browser powered by GeckoView, with multi-tab support and history.
 *
 * [onNavigateUp] is called when the user taps the navigation-back arrow
 * in the top app bar — it navigates back to the Downloads screen.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BrowserScreen(
    onNavigateUp: () -> Unit = {},
    viewModel: BrowserViewModel = viewModel(factory = BrowserViewModel.Factory),
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val activeTab = uiState.activeTab

    // Sync URL bar whenever the active tab's URL changes (e.g. clicking a link or switching tabs).
    LaunchedEffect(activeTab?.url) {
        viewModel.onUrlInputChanged(activeTab?.url ?: "")
    }

    val onNavigate = { viewModel.navigate(uiState.urlInput) }

    Scaffold(
        topBar = {
            Column {
                TopAppBar(
                    navigationIcon = {
                        IconButton(onClick = onNavigateUp) {
                            Icon(
                                imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                                contentDescription = "Back to downloads",
                            )
                        }
                    },
                    title = {
                        OutlinedTextField(
                            value = uiState.urlInput,
                            onValueChange = { viewModel.onUrlInputChanged(it) },
                            singleLine = true,
                            placeholder = { Text("https://") },
                            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Go),
                            keyboardActions = KeyboardActions(onGo = { onNavigate() }),
                            modifier = Modifier.fillMaxWidth(),
                        )
                    },
                    actions = {
                        // Bookmark toggle: filled/primary when bookmarked, muted when not
                        val canBookmark = activeTab?.url?.let {
                            it.isNotBlank() && it != "about:blank" &&
                            (it.startsWith("http://") || it.startsWith("https://"))
                        } == true && activeTab?.isLoading != true
                        IconButton(
                            onClick = { viewModel.toggleBookmark() },
                            enabled = canBookmark,
                        ) {
                            Icon(
                                imageVector = Icons.Default.Star,
                                contentDescription = if (uiState.isCurrentPageBookmarked)
                                    "Remove bookmark" else "Bookmark this page",
                                tint = if (uiState.isCurrentPageBookmarked)
                                    MaterialTheme.colorScheme.primary
                                else
                                    MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                            )
                        }
                        // Bookmarks list opener — uses Star with secondary tint to distinguish
                        // from the history button (Icons.AutoMirrored.Filled.List).
                        // Icons.Default.BookmarkBorder / Bookmarks are in material-icons-extended,
                        // which is not a project dependency; secondary-tinted Star is the closest
                        // visually distinct alternative available in material-icons-core.
                        IconButton(onClick = { viewModel.openBookmarks() }) {
                            Icon(
                                imageVector = Icons.Default.Star,
                                contentDescription = "Bookmarks",
                                tint = MaterialTheme.colorScheme.secondary,
                            )
                        }
                        IconButton(onClick = { viewModel.openHistory() }) {
                            Icon(
                                imageVector = Icons.AutoMirrored.Filled.List,
                                contentDescription = "History",
                            )
                        }
                        IconButton(
                            onClick = { viewModel.goBack() },
                            enabled = activeTab?.canGoBack == true,
                        ) {
                            Icon(
                                imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                                contentDescription = "Navigate back",
                            )
                        }
                        IconButton(
                            onClick = { viewModel.goForward() },
                            enabled = activeTab?.canGoForward == true,
                        ) {
                            Icon(
                                imageVector = Icons.AutoMirrored.Filled.ArrowForward,
                                contentDescription = "Navigate forward",
                            )
                        }
                        IconButton(onClick = { viewModel.reload() }) {
                            Icon(
                                imageVector = Icons.Default.Refresh,
                                contentDescription = "Reload",
                            )
                        }
                    },
                )

                FilterChip(
                    selected = uiState.trackerBlockingEnabled,
                    onClick = { viewModel.openTrackerDetails() },
                    label = {
                        val status = if (uiState.trackerBlockingEnabled) {
                            "Tracker blocking on"
                        } else {
                            "Tracker blocking off"
                        }
                        Text("$status · ${uiState.activeTabBlockedRequests}")
                    },
                    modifier = Modifier.padding(start = 12.dp, bottom = 4.dp),
                )
                FilterChip(
                    selected = uiState.privacySettings.fingerprintingProtectionEnabled ||
                        uiState.privacySettings.cryptominingProtectionEnabled ||
                        uiState.privacySettings.thirdPartyCookieIsolationEnabled ||
                        uiState.privacySettings.httpsOnlyEnabled,
                    onClick = { viewModel.openPrivacySettings() },
                    label = { Text("Privacy controls") },
                    modifier = Modifier.padding(start = 8.dp, bottom = 4.dp),
                )

                // Tab bar — horizontal scrollable row of  FilterChips + new-tab button
                Row(
                    modifier = Modifier
                        .horizontalScroll(rememberScrollState())
                        .fillMaxWidth()
                        .padding(horizontal = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    uiState.tabs.forEach { tab ->
                        val isActive = tab.id == uiState.activeTabId
                        FilterChip(
                            selected = isActive,
                            onClick = { viewModel.selectTab(tab.id) },
                            label = {
                                Text(
                                    text = tab.title ?: tab.url.take(20),
                                    maxLines = 1,
                                )
                            },
                            trailingIcon = {
                                IconButton(
                                    onClick = { viewModel.closeTab(tab.id) },
                                    modifier = Modifier.size(18.dp),
                                ) {
                                    Icon(
                                        imageVector = Icons.Default.Close,
                                        contentDescription = "Close tab",
                                        modifier = Modifier.size(14.dp),
                                    )
                                }
                            },
                            modifier = Modifier.padding(end = 4.dp),
                        )
                    }
                    IconButton(onClick = { viewModel.newTab() }) {
                        Icon(
                            imageVector = Icons.Default.Add,
                            contentDescription = "New tab",
                        )
                    }
                }

                if (activeTab?.isLoading == true) {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                }
            }
        },
    ) { innerPadding ->
        Column(
            modifier = Modifier
                .padding(innerPadding)
                .fillMaxSize(),
        ) {
            activeTab?.errorText?.let { error ->
                Surface(
                    color = MaterialTheme.colorScheme.errorContainer,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(
                        text = error,
                        color = MaterialTheme.colorScheme.onErrorContainer,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
            }

            val context = LocalContext.current
            AndroidView(
                factory = { ctx ->
                    GeckoView(ctx).also { geckoView ->
                        viewModel.attachView(geckoView, GeckoRuntimeHolder.getOrCreate(ctx))
                    }
                },
                modifier = Modifier.fillMaxSize(),
            )
        }
    }

    // History dialog — overlays the Scaffold
    if (uiState.showHistory) {
        AlertDialog(
            onDismissRequest = { viewModel.closeHistory() },
            title = { Text("History") },
            text = {
                Column {
                    if (uiState.historyEntries.isEmpty()) {
                        Text(
                            text = "No history yet.",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    } else {
                        LazyColumn(
                            modifier = Modifier.fillMaxWidth(),
                            verticalArrangement = Arrangement.spacedBy(0.dp),
                        ) {
                            items(uiState.historyEntries, key = { it.id }) { entry ->
                                Column(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clickable {
                                            viewModel.navigateToHistoryEntry(entry.url)
                                        }
                                        .padding(vertical = 8.dp, horizontal = 4.dp),
                                ) {
                                    Text(
                                        text = entry.title ?: entry.url,
                                        style = MaterialTheme.typography.bodyMedium,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                    Text(
                                        text = entry.url,
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                    Text(
                                        text = formatHistoryTimestamp(entry.timestampMs),
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                HorizontalDivider()
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { viewModel.clearHistory() }) {
                    Text("Clear")
                }
            },
            dismissButton = {
                TextButton(onClick = { viewModel.closeHistory() }) {
                    Text("Close")
                }
            },
        )
    }

    // Bookmarks dialog — overlays the Scaffold
    if (uiState.showBookmarks) {
        BookmarksDialog(
            bookmarks = uiState.bookmarks,
            onDismiss = { viewModel.closeBookmarks() },
            onOpen = { url -> viewModel.navigateToBookmark(url) },
            onDelete = { id -> viewModel.removeBookmark(id) },
        )
    }

    if (uiState.showTrackerDetails) {
        TrackerBlockingDialog(
            enabled = uiState.trackerBlockingEnabled,
            blockedRequests = uiState.activeTabBlockedRequests,
            blockedHosts = uiState.activeTabBlockedHosts,
            onEnabledChange = viewModel::setTrackerBlockingEnabled,
            onDismiss = viewModel::closeTrackerDetails,
        )
    }

    if (uiState.showPrivacySettings) {
        PrivacySettingsDialog(
            settings = uiState.privacySettings,
            onFingerprintingProtectionChange = viewModel::setFingerprintingProtectionEnabled,
            onCryptominingProtectionChange = viewModel::setCryptominingProtectionEnabled,
            onThirdPartyCookieIsolationChange = viewModel::setThirdPartyCookieIsolationEnabled,
            onHttpsOnlyChange = viewModel::setHttpsOnlyEnabled,
            onDismiss = viewModel::closePrivacySettings,
        )
    }
}

@Composable
private fun PrivacySettingsDialog(
    settings: PrivacySettings,
    onFingerprintingProtectionChange: (Boolean) -> Unit,
    onCryptominingProtectionChange: (Boolean) -> Unit,
    onThirdPartyCookieIsolationChange: (Boolean) -> Unit,
    onHttpsOnlyChange: (Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Privacy controls") },
        text = {
            Column(
                modifier = Modifier
                    .heightIn(max = 420.dp)
                    .verticalScroll(rememberScrollState()),
            ) {
                Text(
                    "These browser-wide settings apply to all tabs. Reload open pages after changing them.",
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(bottom = 8.dp),
                )
                PrivacySwitchRow(
                    title = "Fingerprinting protection",
                    description = "Enable GeckoView fingerprinting protections and block known fingerprinting trackers.",
                    checked = settings.fingerprintingProtectionEnabled,
                    onCheckedChange = onFingerprintingProtectionChange,
                )
                PrivacySwitchRow(
                    title = "Cryptomining protection",
                    description = "Block known cryptomining resources identified by GeckoView.",
                    checked = settings.cryptominingProtectionEnabled,
                    onCheckedChange = onCryptominingProtectionChange,
                )
                PrivacySwitchRow(
                    title = "Third-party cookie isolation",
                    description = "Partition third-party cookies by top-level site. Turning this off accepts all cookies and site data.",
                    checked = settings.thirdPartyCookieIsolationEnabled,
                    onCheckedChange = onThirdPartyCookieIsolationChange,
                )
                PrivacySwitchRow(
                    title = "HTTPS-only mode",
                    description = "Use GeckoView HTTPS-Only mode. HTTP-only sites may not load.",
                    checked = settings.httpsOnlyEnabled,
                    onCheckedChange = onHttpsOnlyChange,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Done") }
        },
    )
}

@Composable
private fun PrivacySwitchRow(
    title: String,
    description: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 6.dp),
    ) {
        Column(modifier = Modifier.weight(1f).padding(end = 8.dp)) {
            Text(title, style = MaterialTheme.typography.bodyMedium)
            Text(
                description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Switch(checked = checked, onCheckedChange = onCheckedChange)
    }
}

@Composable
private fun TrackerBlockingDialog(
    enabled: Boolean,
    blockedRequests: Long,
    blockedHosts: List<String>,
    onEnabledChange: (Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Tracker blocking") },
        text = {
            Column {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = if (enabled) "Enabled" else "Disabled",
                        modifier = Modifier.weight(1f),
                    )
                    Switch(checked = enabled, onCheckedChange = onEnabledChange)
                }
                Text("Blocked requests in this tab: $blockedRequests")
                if (blockedHosts.isNotEmpty()) {
                    Text(
                        text = "Blocked hosts",
                        style = MaterialTheme.typography.titleSmall,
                        modifier = Modifier.padding(top = 12.dp, bottom = 4.dp),
                    )
                    LazyColumn {
                        items(blockedHosts) { host ->
                            Text(
                                text = host,
                                style = MaterialTheme.typography.bodySmall,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.padding(vertical = 2.dp),
                            )
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Done") }
        },
    )
}

@Composable
private fun BookmarksDialog(
    bookmarks: List<Bookmark>,
    onDismiss: () -> Unit,
    onOpen: (String) -> Unit,
    onDelete: (String) -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Bookmarks") },
        text = {
            Column {
                if (bookmarks.isEmpty()) {
                    Text(
                        text = "No bookmarks yet.",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                } else {
                    LazyColumn(modifier = Modifier.fillMaxWidth()) {
                        items(bookmarks, key = { it.id }) { bookmark ->
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable { onOpen(bookmark.url) }
                                    .padding(vertical = 8.dp, horizontal = 4.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Column(modifier = Modifier.weight(1f)) {
                                    Text(
                                        text = bookmark.title,
                                        style = MaterialTheme.typography.bodyMedium,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                    Text(
                                        text = bookmark.url,
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                }
                                IconButton(onClick = { onDelete(bookmark.id) }) {
                                    Icon(
                                        imageVector = Icons.Default.Close,
                                        contentDescription = "Delete bookmark",
                                    )
                                }
                            }
                            HorizontalDivider()
                        }
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Close") }
        },
    )
}

private fun formatHistoryTimestamp(epochMs: Long): String {
    val sdf = SimpleDateFormat("HH:mm", Locale.getDefault())
    return sdf.format(Date(epochMs))
}
