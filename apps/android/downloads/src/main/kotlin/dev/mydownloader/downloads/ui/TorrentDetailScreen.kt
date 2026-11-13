package dev.fetchx.downloads.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadState
import dev.fetchx.core.download.TorrentDownloadMetadata
import dev.fetchx.core.download.TorrentTelemetry

/**
 * Full-screen detail view for a torrent download.
 *
 * Shows torrent name, info hash, a collapsible file tree with per-file selection
 * checkboxes, tracker list, live telemetry (peers, speed, share ratio), and
 * action buttons (Start Selected Files, Stop Seeding, Cancel).
 *
 * Accessed by tapping a torrent row in [DownloadsScreen].
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TorrentDetailScreen(
    downloadId: String,
    viewModel: DownloadsViewModel,
    onBack: () -> Unit,
) {
    val allDownloads by viewModel.allDownloads.collectAsStateWithLifecycle()
    val torrentTelemetry by viewModel.torrentTelemetry.collectAsStateWithLifecycle()
    val torrentMetadata by viewModel.torrentMetadata.collectAsStateWithLifecycle()

    val item: DownloadItem? = allDownloads.firstOrNull { it.id == downloadId }
    val telemetry: TorrentTelemetry? = torrentTelemetry[downloadId]
    val metadata: TorrentDownloadMetadata? = torrentMetadata[downloadId]

    val torrentName = metadata?.metadata?.name
        ?: item?.fileName
        ?: "Torrent"

    // Per-directory expansion state — must be declared at composable scope, not inside LazyListScope.
    val expandedDirs = remember { mutableStateMapOf<String, Boolean>() }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        text = torrentName,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back",
                        )
                    }
                },
            )
        },
    ) { innerPadding ->
        if (item == null) {
            // Download no longer exists — auto-navigate back or show placeholder.
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(innerPadding)
                    .padding(16.dp),
                verticalArrangement = Arrangement.Center,
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(
                    text = "Download not found",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            return@Scaffold
        }

        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(innerPadding),
            contentAlignment = Alignment.TopCenter,
        ) {
            LazyColumn(
                modifier = Modifier
                    .widthIn(max = 840.dp)
                    .fillMaxSize()
                    .padding(horizontal = 16.dp),
            ) {
            // ── (a) Info card ──────────────────────────────────────────────
            item(key = "info_card") {
                Spacer(modifier = Modifier.height(12.dp))
                InfoCard(item = item, metadata = metadata)
                Spacer(modifier = Modifier.height(12.dp))
            }

            // ── (b) File tree ──────────────────────────────────────────────
            val torrentMeta = metadata?.metadata
            if (torrentMeta != null) {
                item(key = "file_tree_header") {
                    SectionHeader("Files")
                    Spacer(modifier = Modifier.height(4.dp))
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.End,
                    ) {
                        TextButton(onClick = {
                            viewModel.setTorrentSelectionAll(downloadId, true)
                        }) {
                            Text("Select all")
                        }
                        TextButton(onClick = {
                            viewModel.setTorrentSelectionAll(downloadId, false)
                        }) {
                            Text("Select none")
                        }
                    }
                }

                // Compute directory tree once per composition.
                val directories = torrentMeta.files
                    .flatMap { file ->
                        val segments = file.path.split('/')
                        (1 until segments.size).map { segments.take(it).joinToString("/") }
                    }
                    .distinct()
                    .sorted()

                if (directories.isEmpty()) {
                    // Flat file list — no directories.
                    items(torrentMeta.files, key = { "file_${it.index}" }) { file ->
                        FileRow(
                            path = file.path,
                            sizeBytes = file.sizeBytes,
                            checked = file.index in (metadata.selectedFileIndexes),
                            onCheckedChange = { selected ->
                                viewModel.setTorrentFileSelected(downloadId, file.index, selected)
                            },
                        )
                    }
                } else {
                    // Group files by top-level directory.
                    val topDirs = directories
                        .filter { !it.contains('/') }
                        .ifEmpty { directories.map { it.substringBefore('/') }.distinct() }

                    items(topDirs, key = { "dir_$it" }) { dir ->
                        val isExpanded = expandedDirs.getOrDefault(dir, true)
                        DirectoryHeader(
                            directory = dir,
                            isExpanded = isExpanded,
                            onToggleExpand = { expandedDirs[dir] = !isExpanded },
                            onSelectAll = {
                                viewModel.setTorrentDirectorySelected(downloadId, dir, true)
                            },
                            onDeselectAll = {
                                viewModel.setTorrentDirectorySelected(downloadId, dir, false)
                            },
                        )
                        if (isExpanded) {
                            val dirFiles = torrentMeta.files.filter { file ->
                                file.path.startsWith("$dir/") || file.path == dir
                            }
                            dirFiles.forEach { file ->
                                FileRow(
                                    path = file.path.removePrefix("$dir/"),
                                    sizeBytes = file.sizeBytes,
                                    checked = file.index in metadata.selectedFileIndexes,
                                    onCheckedChange = { selected ->
                                        viewModel.setTorrentFileSelected(downloadId, file.index, selected)
                                    },
                                    modifier = Modifier.padding(start = 16.dp),
                                )
                            }
                        }
                    }
                }

                item(key = "file_tree_footer") {
                    Spacer(modifier = Modifier.height(4.dp))
                }

                // ── (c) Trackers ───────────────────────────────────────────
                val trackers = torrentMeta.trackers
                    .ifEmpty { telemetry?.trackerUris.orEmpty() }
                if (trackers.isNotEmpty()) {
                    item(key = "trackers_header") {
                        Spacer(modifier = Modifier.height(8.dp))
                        SectionHeader("Trackers")
                        Spacer(modifier = Modifier.height(4.dp))
                    }
                    items(trackers, key = { "tracker_$it" }) { tracker ->
                        Text(
                            text = tracker,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(vertical = 2.dp),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }

            // ── (d) Live telemetry ─────────────────────────────────────────
            if (telemetry != null) {
                item(key = "telemetry") {
                    Spacer(modifier = Modifier.height(12.dp))
                    SectionHeader("Live telemetry")
                    Spacer(modifier = Modifier.height(4.dp))
                    TelemetryCard(telemetry = telemetry)
                    Spacer(modifier = Modifier.height(12.dp))
                }
            }

            // ── (e) Action buttons ─────────────────────────────────────────
            item(key = "actions") {
                Spacer(modifier = Modifier.height(4.dp))
                ActionButtons(
                    item = item,
                    metadata = metadata,
                    onStartSelectedFiles = {
                        val indexes = metadata?.selectedFileIndexes ?: emptyList()
                        if (indexes.isNotEmpty()) {
                            viewModel.selectTorrentFiles(downloadId, indexes)
                        }
                    },
                    onStopSeeding = { viewModel.stopTorrentSeeding(downloadId) },
                    onCancel = { viewModel.cancel(downloadId) },
                )
                Spacer(modifier = Modifier.height(24.dp))
            }
            }
        }
    }
}

// ── Sub-composables ────────────────────────────────────────────────────────────

@Composable
private fun InfoCard(item: DownloadItem, metadata: TorrentDownloadMetadata?) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            val torrentMeta = metadata?.metadata
            if (torrentMeta != null) {
                LabeledValue("Name", torrentMeta.name)
                LabeledValue("Info hash", torrentMeta.infoHash)
                LabeledValue(
                    "Total size",
                    formatBytes(torrentMeta.totalSizeBytes),
                )
                if (metadata.selectedTotalBytes > 0) {
                    LabeledValue(
                        "Selected size",
                        formatBytes(metadata.selectedTotalBytes),
                    )
                }
            }
            LabeledValue("Engine", item.engine)
            LabeledValue("Status", stateLabel(item.state))
        }
    }
}

@Composable
private fun SectionHeader(title: String) {
    Text(
        text = title,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.onSurface,
    )
    HorizontalDivider(modifier = Modifier.padding(vertical = 4.dp))
}

@Composable
private fun LabeledValue(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Text(
            text = "$label: ",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.weight(0.35f),
        )
        Text(
            text = value,
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.weight(0.65f),
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun DirectoryHeader(
    directory: String,
    isExpanded: Boolean,
    onToggleExpand: () -> Unit,
    onSelectAll: () -> Unit,
    onDeselectAll: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(role = Role.Button, onClick = onToggleExpand)
            .semantics {
                contentDescription = "Folder $directory"
                stateDescription = if (isExpanded) "Expanded" else "Collapsed"
            }
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = if (isExpanded) "▼" else "▶",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(end = 4.dp),
        )
        Text(
            text = directory,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.weight(1f),
        )
        TextButton(onClick = onSelectAll) { Text("Select") }
        TextButton(onClick = onDeselectAll) { Text("Deselect") }
    }
}

@Composable
private fun FileRow(
    path: String,
    sizeBytes: Long,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(
            checked = checked,
            onCheckedChange = onCheckedChange,
            modifier = Modifier.semantics {
                contentDescription = "Select $path"
            },
        )
        Text(
            text = path,
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.weight(1f),
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            text = formatBytes(sizeBytes),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun TelemetryCard(telemetry: TorrentTelemetry) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(12.dp)) {
            val peers = telemetry.peerCount?.toString() ?: "—"
            val completedPeers = telemetry.completedPeerCount?.toString() ?: "—"
            val seeds = telemetry.seedCount?.toString() ?: "—"
            val download = telemetry.downloadSpeedBytesPerSecond?.let(::formatSpeed) ?: "—"
            val upload = telemetry.uploadSpeedBytesPerSecond?.let(::formatSpeed) ?: "—"
            val uploaded = telemetry.uploadedBytes?.let(::formatBytes) ?: "—"
            val ratio = telemetry.shareRatio?.let { "%.3f".format(it) } ?: "—"

            LabeledValue("Peers", "$peers (complete: $completedPeers, seeds: $seeds)")
            LabeledValue("Download speed", download)
            LabeledValue("Upload speed", upload)
            LabeledValue("Uploaded", uploaded)
            LabeledValue("Share ratio", ratio)
        }
    }
}

@Composable
private fun ActionButtons(
    item: DownloadItem,
    metadata: TorrentDownloadMetadata?,
    onStartSelectedFiles: () -> Unit,
    onStopSeeding: () -> Unit,
    onCancel: () -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        val canStartFiles = item.state is DownloadState.MetadataAcquiring &&
            metadata != null &&
            metadata.selectedFileIndexes.isNotEmpty()
        if (canStartFiles) {
            Button(
                onClick = onStartSelectedFiles,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Start selected files")
            }
            Spacer(modifier = Modifier.height(8.dp))
        }

        val canStopSeeding = item.state is DownloadState.Seeding
        if (canStopSeeding) {
            OutlinedButton(
                onClick = onStopSeeding,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Stop seeding")
            }
            Spacer(modifier = Modifier.height(8.dp))
        }

        val canCancel = item.state is DownloadState.Active ||
            item.state is DownloadState.Queued ||
            item.state is DownloadState.MetadataAcquiring ||
            item.state is DownloadState.Recovering ||
            (item.state is DownloadState.Seeding &&
                !item.engine.equals("native_torrent", ignoreCase = true)) ||
            item.state is DownloadState.Paused
        if (canCancel) {
            OutlinedButton(
                onClick = onCancel,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Cancel")
            }
        }
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

private fun stateLabel(state: DownloadState): String = when (state) {
    is DownloadState.Active -> "Downloading"
    is DownloadState.Cancelled -> "Cancelled"
    is DownloadState.Completed -> "Completed"
    is DownloadState.PublicationPending -> "Public copy pending"
    is DownloadState.Failed -> "Failed"
    is DownloadState.MetadataAcquiring -> "Acquiring metadata"
    is DownloadState.Paused -> "Paused"
    is DownloadState.Queued -> "Queued"
    is DownloadState.Recovering -> "Recovering"
    is DownloadState.Seeding -> "Seeding"
    is DownloadState.Stopped -> "Seeding stopped"
}
