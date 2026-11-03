package dev.fetchx.browser

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Bookmark
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Shield
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.Switch
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.unit.dp
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Date
import java.util.Locale
import java.text.SimpleDateFormat

@Composable
internal fun BrowserBookmarksScreen(
    bookmarks: List<Bookmark>,
    onDismiss: () -> Unit,
    onOpen: (String) -> Unit,
    onEdit: (String, String) -> Unit,
    onDelete: (String) -> Unit,
) {
    var query by rememberSaveable { mutableStateOf("") }
    var editingBookmark by remember { mutableStateOf<Bookmark?>(null) }
    var deletingBookmark by remember { mutableStateOf<Bookmark?>(null) }
    val filteredBookmarks = bookmarks.filter {
        query.isBlank() || it.title.contains(query, ignoreCase = true) ||
            it.url.contains(query, ignoreCase = true)
    }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier.fillMaxSize(),
            color = MaterialTheme.colorScheme.background,
        ) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .windowInsetsPadding(WindowInsets.safeDrawing),
            ) {
                LibraryScreenHeader(
                    title = "Bookmarks",
                    iconContentDescription = "Bookmarks",
                    onDismiss = onDismiss,
                )
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    singleLine = true,
                    label = { Text("Search bookmarks") },
                    leadingIcon = {
                        Icon(Icons.Default.Search, contentDescription = null)
                    },
                    trailingIcon = {
                        if (query.isNotEmpty()) {
                            IconButton(onClick = { query = "" }) {
                                Icon(Icons.Default.Close, contentDescription = "Clear bookmark search")
                            }
                        }
                    },
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 8.dp),
                )
                when {
                    bookmarks.isEmpty() -> LibraryEmptyState(
                        icon = { Icon(Icons.Default.Bookmark, contentDescription = null) },
                        title = "No bookmarks yet",
                        message = "Save pages you want to revisit here.",
                        modifier = Modifier.weight(1f),
                    )
                    filteredBookmarks.isEmpty() -> LibraryEmptyState(
                        icon = { Icon(Icons.Default.Search, contentDescription = null) },
                        title = "No matching bookmarks",
                        message = "Try another title or address.",
                        modifier = Modifier.weight(1f),
                    )
                    else -> LazyColumn(
                        modifier = Modifier
                            .fillMaxWidth()
                            .weight(1f),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        items(filteredBookmarks, key = { it.id }) { bookmark ->
                            BookmarkRow(
                                bookmark = bookmark,
                                onOpen = { onOpen(bookmark.url) },
                                onEdit = { editingBookmark = bookmark },
                                onDelete = { deletingBookmark = bookmark },
                            )
                        }
                    }
                }
            }
        }
    }

    editingBookmark?.let { bookmark ->
        BookmarkEditorDialog(
            bookmark = bookmark,
            onDismiss = { editingBookmark = null },
            onSave = { title ->
                onEdit(bookmark.id, title)
                editingBookmark = null
            },
        )
    }

    deletingBookmark?.let { bookmark ->
        AlertDialog(
            onDismissRequest = { deletingBookmark = null },
            title = { Text("Delete bookmark?") },
            text = { Text("“${bookmark.title}” will be removed from your bookmarks.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        onDelete(bookmark.id)
                        deletingBookmark = null
                    },
                ) { Text("Delete") }
            },
            dismissButton = {
                TextButton(onClick = { deletingBookmark = null }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun BookmarkRow(
    bookmark: Bookmark,
    onOpen: () -> Unit,
    onEdit: () -> Unit,
    onDelete: () -> Unit,
) {
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.padding(start = 16.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
        ) {
            Column(
                modifier = Modifier
                    .weight(1f)
                    .clickable(onClick = onOpen)
                    .padding(vertical = 8.dp),
            ) {
                Text(
                    text = bookmark.title,
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
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
            IconButton(
                onClick = onEdit,
                modifier = Modifier.size(48.dp),
            ) {
                Icon(Icons.Default.Edit, contentDescription = "Edit bookmark ${bookmark.title}")
            }
            IconButton(
                onClick = onDelete,
                modifier = Modifier.size(48.dp),
            ) {
                Icon(Icons.Default.Delete, contentDescription = "Delete bookmark ${bookmark.title}")
            }
        }
    }
}

@Composable
private fun BookmarkEditorDialog(
    bookmark: Bookmark,
    onDismiss: () -> Unit,
    onSave: (String) -> Unit,
) {
    var title by rememberSaveable(bookmark.id) { mutableStateOf(bookmark.title) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Edit bookmark") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedTextField(
                    value = title,
                    onValueChange = { title = it },
                    singleLine = true,
                    label = { Text("Title") },
                    modifier = Modifier.fillMaxWidth(),
                )
                Column {
                    Text("URL", style = MaterialTheme.typography.labelMedium)
                    Text(
                        text = bookmark.url,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 4.dp),
                    )
                    Text(
                        text = "The saved URL is preserved; URL editing is not supported.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 4.dp),
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onSave(title.trim()) },
                enabled = title.isNotBlank(),
            ) { Text("Save") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

@Composable
internal fun BrowserHistoryScreen(
    entries: List<HistoryEntry>,
    onDismiss: () -> Unit,
    onOpen: (String) -> Unit,
    onDelete: (String) -> Unit,
    onClear: () -> Unit,
) {
    var query by rememberSaveable { mutableStateOf("") }
    var showClearConfirmation by rememberSaveable { mutableStateOf(false) }
    val filteredEntries = entries
        .filter {
            query.isBlank() || it.url.contains(query, ignoreCase = true) ||
                (it.title ?: "").contains(query, ignoreCase = true)
        }
        .sortedByDescending { it.timestampMs }
    val groupedEntries = filteredEntries.groupBy { historyDate(it.timestampMs) }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier.fillMaxSize(),
            color = MaterialTheme.colorScheme.background,
        ) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .windowInsetsPadding(WindowInsets.safeDrawing),
            ) {
                LibraryScreenHeader(
                    title = "History",
                    iconContentDescription = "History",
                    onDismiss = onDismiss,
                    action = {
                        IconButton(
                            onClick = { showClearConfirmation = true },
                            enabled = entries.isNotEmpty(),
                        ) {
                            Icon(Icons.Default.Delete, contentDescription = "Clear browsing history")
                        }
                    },
                )
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    singleLine = true,
                    label = { Text("Search history") },
                    leadingIcon = { Icon(Icons.Default.Search, contentDescription = null) },
                    trailingIcon = {
                        if (query.isNotEmpty()) {
                            IconButton(onClick = { query = "" }) {
                                Icon(Icons.Default.Close, contentDescription = "Clear history search")
                            }
                        }
                    },
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 8.dp),
                )
                if (filteredEntries.isEmpty()) {
                    LibraryEmptyState(
                        icon = { Icon(Icons.Default.History, contentDescription = null) },
                        title = if (entries.isEmpty()) "No history yet" else "No matching history",
                        message = if (entries.isEmpty()) {
                            "Pages you visit will appear here."
                        } else {
                            "Try another title or address."
                        },
                        modifier = Modifier.weight(1f),
                    )
                } else {
                    LazyColumn(
                        modifier = Modifier
                            .fillMaxWidth()
                            .weight(1f),
                    ) {
                        groupedEntries.forEach { (date, dateEntries) ->
                            item(key = "history-date-$date") {
                                Text(
                                    text = historyDateLabel(date),
                                    style = MaterialTheme.typography.titleSmall,
                                    color = MaterialTheme.colorScheme.primary,
                                    fontWeight = FontWeight.SemiBold,
                                    modifier = Modifier.padding(horizontal = 20.dp, vertical = 10.dp),
                                )
                            }
                            items(dateEntries, key = { it.id }) { entry ->
                                HistoryRow(
                                    entry = entry,
                                    onOpen = { onOpen(entry.url) },
                                    onDelete = { onDelete(entry.id) },
                                )
                            }
                        }
                    }
                }
            }
        }
    }

    if (showClearConfirmation) {
        AlertDialog(
            onDismissRequest = { showClearConfirmation = false },
            title = { Text("Clear browsing history?") },
            text = {
                Text("Saved history will be removed. Open tabs, bookmarks, downloads, and cookies are not affected.")
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        onClear()
                        showClearConfirmation = false
                    },
                ) { Text("Clear") }
            },
            dismissButton = {
                TextButton(onClick = { showClearConfirmation = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun HistoryRow(
    entry: HistoryEntry,
    onOpen: () -> Unit,
    onDelete: () -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onOpen)
            .padding(start = 20.dp, end = 8.dp, top = 4.dp, bottom = 4.dp),
    ) {
        Column(
            modifier = Modifier
                .weight(1f)
                .padding(vertical = 8.dp),
        ) {
            Text(
                text = entry.title?.takeIf(String::isNotBlank) ?: entry.url,
                style = MaterialTheme.typography.bodyLarge,
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
                text = formatHistoryTime(entry.timestampMs),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        IconButton(onClick = onDelete, modifier = Modifier.size(48.dp)) {
            Icon(Icons.Default.Delete, contentDescription = "Delete history entry ${entry.title ?: entry.url}")
        }
    }
}

@Composable
private fun LibraryScreenHeader(
    title: String,
    iconContentDescription: String,
    onDismiss: () -> Unit,
    action: @Composable (() -> Unit)? = null,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 4.dp, end = 8.dp, top = 8.dp, bottom = 4.dp),
    ) {
        IconButton(onClick = onDismiss, modifier = Modifier.size(48.dp)) {
            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Close $iconContentDescription")
        }
        Text(
            text = title,
            style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.weight(1f),
        )
        action?.invoke()
    }
}

@Composable
private fun LibraryEmptyState(
    icon: @Composable () -> Unit,
    title: String,
    message: String,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier = modifier.fillMaxWidth(),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.padding(32.dp),
        ) {
            Surface(
                shape = CircleShape,
                color = MaterialTheme.colorScheme.secondaryContainer,
                modifier = Modifier.size(64.dp),
            ) {
                Box(contentAlignment = Alignment.Center) {
                    icon()
                }
            }
            Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Text(
                message,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
internal fun SitePrivacyDialog(
    url: String,
    trackerProtectionEnabled: Boolean,
    blockedRequests: Long,
    blockedHosts: List<String>,
    onTrackerProtectionChange: (Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    val connectionStatus = siteConnectionStatus(url)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Site privacy") },
        text = {
            Column(
                modifier = Modifier
                    .heightIn(max = 440.dp)
                    .padding(vertical = 4.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(
                        imageVector = when (connectionStatus) {
                            SiteConnectionStatus.HTTPS -> Icons.Default.Lock
                            SiteConnectionStatus.HTTP -> Icons.Default.Warning
                            SiteConnectionStatus.UNKNOWN -> Icons.Default.Shield
                        },
                        contentDescription = null,
                        tint = when (connectionStatus) {
                            SiteConnectionStatus.HTTPS -> MaterialTheme.colorScheme.primary
                            SiteConnectionStatus.HTTP -> MaterialTheme.colorScheme.error
                            SiteConnectionStatus.UNKNOWN -> MaterialTheme.colorScheme.onSurfaceVariant
                        },
                        modifier = Modifier.size(22.dp),
                    )
                    Spacer(Modifier.width(10.dp))
                    Column {
                        Text("Connection", style = MaterialTheme.typography.labelMedium)
                        Text(
                            when (connectionStatus) {
                                SiteConnectionStatus.HTTPS -> "HTTPS connection"
                                SiteConnectionStatus.HTTP -> "HTTP connection — not encrypted"
                                SiteConnectionStatus.UNKNOWN -> "Connection status unavailable"
                            },
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
                if (connectionStatus == SiteConnectionStatus.HTTPS) {
                    Text(
                        "This view reports the URL scheme only; certificate details are not exposed here.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(modifier = Modifier.weight(1f).padding(end = 8.dp)) {
                        Text("Tracking protection", style = MaterialTheme.typography.labelMedium)
                        Text(
                            if (trackerProtectionEnabled) "Enabled" else "Disabled",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                    Switch(
                        checked = trackerProtectionEnabled,
                        onCheckedChange = onTrackerProtectionChange,
                    )
                }
                Text(
                    "Blocked requests observed in this tab during this session: $blockedRequests",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (blockedHosts.isNotEmpty()) {
                    Text("Observed blocked hosts", style = MaterialTheme.typography.labelMedium)
                    LazyColumn(modifier = Modifier.heightIn(max = 120.dp)) {
                        items(blockedHosts) { host ->
                            Text(
                                text = host,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
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

private fun historyDate(epochMs: Long): LocalDate =
    Instant.ofEpochMilli(epochMs).atZone(ZoneId.systemDefault()).toLocalDate()

private fun historyDateLabel(date: LocalDate): String {
    val today = LocalDate.now()
    return when (date) {
        today -> "Today"
        today.minusDays(1) -> "Yesterday"
        else -> date.format(DateTimeFormatter.ofPattern("EEEE, MMM d", Locale.getDefault()))
    }
}

private fun formatHistoryTime(epochMs: Long): String =
    SimpleDateFormat("h:mm a", Locale.getDefault()).format(Date(epochMs))
