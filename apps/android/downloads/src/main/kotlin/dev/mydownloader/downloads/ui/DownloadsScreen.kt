package dev.fetchx.downloads.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.fetchx.core.download.TorrentMetadata
import java.io.IOException
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private val TABS = listOf("All", "Active", "Queued", "Completed", "Failed", "Cancelled")

/**
 * Top-level Downloads screen with six tabs: All, Active, Queued, Completed, Failed, Cancelled.
 *
 * The [viewModel] must be created with [DownloadsViewModel.Factory] and passed in
 * from the NavGraph so the factory has access to the bridge repository.
 *
 * [downloadsDir] is the absolute path to the downloads directory used when building the
 * destination path for new downloads. It must be resolved outside the ViewModel (in the
 * NavGraph composable) so the ViewModel stays free of Android Context.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DownloadsScreen(
    viewModel: DownloadsViewModel,
    downloadsDir: String = "",
    onBrowseWeb: () -> Unit = {},
    onPlayFile: (String) -> Unit = {},
    onOpenTorrentDetail: (String) -> Unit = {},
) {
    // Collect all state flows with lifecycle awareness.
    val allDownloads by viewModel.allDownloads.collectAsStateWithLifecycle()
    val activeDownloads by viewModel.activeDownloads.collectAsStateWithLifecycle()
    val queuedDownloads by viewModel.queuedDownloads.collectAsStateWithLifecycle()
    val completedDownloads by viewModel.completedDownloads.collectAsStateWithLifecycle()
    val failedDownloads by viewModel.failedDownloads.collectAsStateWithLifecycle()
    val cancelledDownloads by viewModel.cancelledDownloads.collectAsStateWithLifecycle()
    val torrentTelemetry by viewModel.torrentTelemetry.collectAsStateWithLifecycle()
    val torrentMetadata by viewModel.torrentMetadata.collectAsStateWithLifecycle()
    val aria2NativeRestart by viewModel.aria2NativeRestart.collectAsStateWithLifecycle()
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val queueDispatchPaused by viewModel.queueDispatchPaused.collectAsStateWithLifecycle()
    val downloadSnapshotState by viewModel.downloadSnapshotState.collectAsStateWithLifecycle()
    val tabCounts = listOf(
        allDownloads.size,
        activeDownloads.size,
        queuedDownloads.size,
        completedDownloads.size,
        failedDownloads.size,
        cancelledDownloads.size,
    )
    val reservedFileNames = remember(allDownloads) {
        allDownloads.flatMap { item ->
            listOf(item.fileName, File(item.destinationPath).name)
        }.filter(String::isNotBlank).toSet()
    }

    var selectedTab by rememberSaveable { mutableIntStateOf(0) }
    var showAddDialog by rememberSaveable { mutableStateOf(false) }
    val snackbarHostState = remember { SnackbarHostState() }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var pendingTorrentFileSelection by remember {
        mutableStateOf<((String, TorrentMetadata) -> Unit)?>(null)
    }
    val torrentRestartFilePicker = androidx.activity.compose.rememberLauncherForActivityResult(
        contract = androidx.activity.result.contract.ActivityResultContracts.OpenDocument(),
    ) { uri ->
        val onSelected = pendingTorrentFileSelection
        pendingTorrentFileSelection = null
        if (uri != null && onSelected != null) {
            scope.launch {
                try {
                    val stagedPath = withContext(Dispatchers.IO) {
                        val directory = File(context.filesDir, "torrent-restart-sources")
                        if (!directory.exists() && !directory.mkdirs()) {
                            throw IOException("Could not create private torrent storage.")
                        }
                        val staged = File(directory, "${UUID.randomUUID()}.torrent")
                        try {
                            val input = context.contentResolver.openInputStream(uri)
                                ?: throw IOException("Could not open the selected torrent document.")
                            input.use { source ->
                                staged.outputStream().use { output ->
                                    val buffer = ByteArray(8192)
                                    var copiedBytes = 0
                                    while (true) {
                                        val count = source.read(buffer)
                                        if (count < 0) break
                                        copiedBytes += count
                                        if (copiedBytes > 8 * 1024 * 1024) {
                                            throw IOException("Torrent file exceeds the 8 MiB limit.")
                                        }
                                        output.write(buffer, 0, count)
                                    }
                                }
                            }
                            staged.absolutePath
                        } catch (error: Exception) {
                            staged.delete()
                            throw error
                        }
                    }
                    val metadata = viewModel.inspectAria2TorrentRestartFile(stagedPath)
                    onSelected(stagedPath, metadata)
                } catch (error: Exception) {
                    snackbarHostState.showSnackbar(error.message ?: "Could not inspect the selected torrent file.")
                }
            }
        }
    }

    // Show errors via Snackbar — clear after display.
    LaunchedEffect(uiState.error) {
        uiState.error?.let { message ->
            snackbarHostState.showSnackbar(message)
            viewModel.clearError()
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Downloads") },
                actions = {
                    TextButton(
                        onClick = viewModel::toggleQueueDispatch,
                        enabled = uiState.bridgeAvailable,
                    ) {
                        Text(if (queueDispatchPaused) "Resume queue" else "Hold queue")
                    }
                },
            )
        },
        floatingActionButton = {
            FloatingActionButton(onClick = { showAddDialog = true }) {
                Icon(Icons.Default.Add, contentDescription = "Add download")
            }
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { innerPadding ->
        Column(modifier = Modifier.padding(innerPadding)) {

            // Degraded-mode banner — shown when the Rust bridge failed to initialise.
            if (!uiState.bridgeAvailable) {
                Surface(
                    color = MaterialTheme.colorScheme.errorContainer,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(
                        text = "Download service unavailable. Restart the app or check for errors.",
                        color = MaterialTheme.colorScheme.onErrorContainer,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
            }

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                TABS.forEachIndexed { index, title ->
                    FilterChip(
                        selected = selectedTab == index,
                        onClick = { selectedTab = index },
                        label = { Text("$title · ${tabCounts[index]}", maxLines = 1) },
                    )
                }
            }

            val filtered = when (selectedTab) {
                0 -> allDownloads
                1 -> activeDownloads
                2 -> queuedDownloads
                3 -> completedDownloads
                4 -> failedDownloads
                5 -> cancelledDownloads
                else -> emptyList()
            }

            if (filtered.isEmpty()) {
                if (downloadSnapshotState == DownloadSnapshotState.Loading) {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        CircularProgressIndicator()
                    }
                } else if (downloadSnapshotState == DownloadSnapshotState.Failed) {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = "Downloads could not be loaded. Restart FetchX to try again.",
                            style = MaterialTheme.typography.bodyMedium,
                            textAlign = TextAlign.Center,
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                } else if (selectedTab == 0) {
                    // All tab: rich empty state with an Add Download call-to-action.
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        Column(
                            horizontalAlignment = Alignment.CenterHorizontally,
                            verticalArrangement = Arrangement.spacedBy(12.dp),
                            modifier = Modifier
                                .widthIn(max = 360.dp)
                                .fillMaxWidth()
                                .padding(24.dp),
                        ) {
                            Surface(
                                shape = androidx.compose.foundation.shape.RoundedCornerShape(14.dp),
                                color = MaterialTheme.colorScheme.surfaceContainerHigh,
                                modifier = Modifier.size(48.dp),
                            ) {
                                Box(contentAlignment = Alignment.Center) {
                                    Icon(
                                        imageVector = Icons.Default.Add,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                        modifier = Modifier.size(24.dp),
                                    )
                                }
                            }
                            Text(
                                text = "No downloads yet",
                                style = MaterialTheme.typography.titleMedium,
                                textAlign = TextAlign.Center,
                            )
                            Text(
                                text = "Start a download from the browser or paste a link.",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                textAlign = TextAlign.Center,
                            )
                            Button(onClick = onBrowseWeb) {
                                Text("Browse the web")
                            }
                        }
                    }
                } else {
                    // Other tabs: simple empty message.
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = "No ${TABS[selectedTab].lowercase()} downloads",
                            style = MaterialTheme.typography.bodyMedium,
                            textAlign = TextAlign.Center,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            } else {
                LazyColumn(
                    modifier = Modifier
                        .widthIn(max = 840.dp)
                        .fillMaxSize()
                        .align(Alignment.CenterHorizontally),
                    contentPadding = PaddingValues(vertical = 8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(filtered, key = { it.id }) { item ->
                        DownloadItemRow(
                            item = item,
                            onPause = { viewModel.pause(item.id) },
                            onResume = { viewModel.resume(item.id) },
                            onCancel = { viewModel.cancel(item.id) },
                            onRemove = { viewModel.remove(item.id) },
                            onRetry = { viewModel.retry(item.id) },
                            onMoveUpInQueue = { viewModel.moveUpInQueue(item.id) },
                            onMoveDownInQueue = { viewModel.moveDownInQueue(item.id) },
                            torrentTelemetry = torrentTelemetry[item.id],
                            torrentMetadata = torrentMetadata[item.id],
                            aria2NativeRestart = aria2NativeRestart[item.id],
                            suggestedNativeRestartDestination =
                                viewModel.suggestAria2NativeRestartDestination(item.fileName),
                            suggestedNativeTorrentRestartDestination =
                                viewModel.suggestAria2NativeTorrentRestartDestination(item.fileName),
                            onRestartAria2WithNative = { url, destination, confirmedDirectFile ->
                                viewModel.restartAria2HttpWithNative(
                                    item.id,
                                    url,
                                    destination,
                                    confirmedDirectFile,
                                )
                            },
                            onRestartAria2TorrentWithNative = { magnetUri, destination, confirmed ->
                                viewModel.restartAria2TorrentWithNative(
                                    item.id,
                                    magnetUri,
                                    destination,
                                    confirmed,
                                )
                            },
                            onRestartAria2TorrentFileWithNative = { path, destination, indexes, confirmed, selectionConfirmed ->
                                viewModel.restartAria2TorrentFileWithNative(
                                    item.id,
                                    path,
                                    destination,
                                    indexes,
                                    confirmed,
                                    selectionConfirmed,
                                )
                            },
                            onChooseAria2TorrentFile = { onSelected ->
                                pendingTorrentFileSelection = onSelected
                                torrentRestartFilePicker.launch(arrayOf("*/*"))
                            },
                            onPrepareAria2NativeRestart = {
                                viewModel.clearAria2NativeRestartState(item.id)
                            },
                            onTorrentFileSelected = { index, selected ->
                                viewModel.setTorrentFileSelected(item.id, index, selected)
                            },
                            onTorrentSelectionAll = { selected ->
                                viewModel.setTorrentSelectionAll(item.id, selected)
                            },
                            onTorrentDirectorySelected = { directory, selected ->
                                viewModel.setTorrentDirectorySelected(item.id, directory, selected)
                            },
                            onSelectTorrentFiles = { indexes ->
                                viewModel.selectTorrentFiles(item.id, indexes)
                            },
                            onStopSeeding = { viewModel.stopTorrentSeeding(item.id) },
                            onPlayFile = if ((item.state is dev.fetchx.core.download.DownloadState.Completed ||
                                item.state is dev.fetchx.core.download.DownloadState.Stopped) &&
                                isVideoFile(item.fileName)) {
                                { onPlayFile(item.destinationPath) }
                            } else null,
                            onOpenDetail = if (item.state is dev.fetchx.core.download.DownloadState.MetadataAcquiring ||
                                item.state is dev.fetchx.core.download.DownloadState.Active ||
                                item.state is dev.fetchx.core.download.DownloadState.Seeding) {
                                { onOpenTorrentDetail(item.id) }
                            } else null,
                        )
                    }
                }
            }
        }
    }

    // Add Download dialog.
    if (showAddDialog) {
        AddDownloadDialog(
            downloadsDir = downloadsDir,
            reservedFileNames = reservedFileNames,
            onInspectTorrent = viewModel::inspectTorrentFile,
            onFindDuplicate = viewModel::findDuplicateDownload,
            onAdd = { url, fileName, destinationPath, priority, selectedTorrentFileIndexes, expectedSha256 ->
                viewModel.addDownload(
                    url,
                    fileName,
                    destinationPath,
                    priority,
                    selectedTorrentFileIndexes,
                    expectedSha256,
                )
            },
            onAddBatch = { items, priority, expectedSha256 ->
                viewModel.addDownloadBatch(items, priority, expectedSha256)
            },
            onDismiss = { showAddDialog = false },
        )
    }
}

private val VIDEO_EXTENSIONS = setOf(
    "mp4", "mkv", "webm", "avi", "mov", "flv", "3gp", "m4v", "ts"
)

private fun isVideoFile(fileName: String): Boolean =
    fileName.substringAfterLast('.', "").lowercase() in VIDEO_EXTENSIONS
