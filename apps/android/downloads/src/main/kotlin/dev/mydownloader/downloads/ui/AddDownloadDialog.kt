package dev.fetchx.downloads.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.Switch
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import dev.fetchx.bridge.BridgeInitializer
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.core.download.DownloadBatchItem
import dev.fetchx.core.download.TorrentMetadata
import dev.fetchx.core.download.HlsVariant
import dev.fetchx.core.download.HttpAuthType
import dev.fetchx.core.download.HttpCredentials
import dev.fetchx.core.download.AdvancedDownloadOptions
import dev.fetchx.downloads.storage.StorageManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.util.UUID

private const val MAX_TORRENT_FILE_BYTES = 8 * 1024 * 1024

/**
 * Dialog for adding a new download.
 *
 * Basic format validation is performed client-side (blank URL, missing scheme).
 * Full validation is delegated to the Rust core via the repository — no second
 * validation system is introduced here.
 *
 * Security note: URL values are never logged.
 *
 * @param downloadsDir    Absolute path to the downloads directory — displayed read-only.
 * @param onFindDuplicate Checks persisted downloads for a matching HTTP(S) URL.
 * @param onAdd           Called with (url, fileName, destinationPath, priority) when
 *                        the user confirms and validation passes.
 * @param onDismiss       Called when the user cancels or dismisses the dialog.
 */
@Composable
fun AddDownloadDialog(
    downloadsDir: String,
    reservedFileNames: Set<String> = emptySet(),
    onInspectTorrent: suspend (String) -> TorrentMetadata,
    onFindDuplicate: suspend (String) -> String?,
    onAdd: (
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority,
        selectedTorrentFileIndexes: List<Int>?,
        expectedSha256: String?,
    ) -> Unit,
    onAddBatch: (
        items: List<DownloadBatchItem>,
        priority: DownloadPriority,
        expectedSha256: String?,
    ) -> Unit,
    onAddAdvanced: (
        url: String,
        fileName: String,
        destinationPath: String,
        priority: DownloadPriority,
        credentials: HttpCredentials?,
        options: AdvancedDownloadOptions,
        downloadSubtitles: Boolean,
        subtitleLangs: String,
    ) -> Unit = { url, fileName, dest, prio, _, _, _, _ ->
        onAdd(url, fileName, dest, prio, null, null)
    },
    onDiscoverHlsVariants: suspend (String) -> List<HlsVariant> = { emptyList() },
    globalSubtitleEnabled: Boolean = false,
    globalSubtitleLangs: String = "en",
    onDismiss: () -> Unit,
) {
    var url by rememberSaveable { mutableStateOf("") }
    val bulkUrls = remember(url) { BulkLinkParser.extract(url) }
    var fileName by rememberSaveable { mutableStateOf("") }
    var userEditedFileName by rememberSaveable { mutableStateOf(false) }
    var urlError by rememberSaveable { mutableStateOf<String?>(null) }
    var selectedPriority by rememberSaveable { mutableStateOf(DownloadPriority.NORMAL) }
    var packageName by rememberSaveable { mutableStateOf("") }
    var expectedSha256 by rememberSaveable { mutableStateOf("") }
    var duplicateWarning by rememberSaveable { mutableStateOf<String?>(null) }
    var duplicateCheckConfirmed by rememberSaveable { mutableStateOf(false) }
    var isCheckingDuplicate by remember { mutableStateOf(false) }
    var torrentMetadata by remember { mutableStateOf<TorrentMetadata?>(null) }
    var selectedTorrentFiles by remember { mutableStateOf<Set<Int>>(emptySet()) }

    // ── Subtitle state ────────────────────────────────────────────────────
    var subtitleEnabled by rememberSaveable { mutableStateOf(globalSubtitleEnabled) }
    var subtitleLangs by rememberSaveable { mutableStateOf(globalSubtitleLangs) }

    // ── HLS variants ──────────────────────────────────────────────────────
    var hlsVariants by remember { mutableStateOf<List<HlsVariant>>(emptyList()) }
    var selectedHlsVariant by remember { mutableStateOf<HlsVariant?>(null) }
    var isFetchingHls by remember { mutableStateOf(false) }
    var hlsDropdownExpanded by remember { mutableStateOf(false) }

    // ── Advanced section ──────────────────────────────────────────────────
    var showAdvanced by rememberSaveable { mutableStateOf(false) }
    var authType by rememberSaveable { mutableStateOf<HttpAuthType?>(null) }
    var authUsername by rememberSaveable { mutableStateOf("") }
    var authPassword by rememberSaveable { mutableStateOf("") }
    var authDomain by rememberSaveable { mutableStateOf("") }
    var proxyUrl by rememberSaveable { mutableStateOf("") }
    var userAgent by rememberSaveable { mutableStateOf("") }
    val context = androidx.compose.ui.platform.LocalContext.current
    val scope = rememberCoroutineScope()
    val torrentPicker = androidx.activity.compose.rememberLauncherForActivityResult(
        contract = androidx.activity.result.contract.ActivityResultContracts.OpenDocument(),
    ) { uri ->
        if (uri != null) {
            scope.launch {
                try {
                    val sourceFile = withContext(Dispatchers.IO) {
                        val directory = File(context.filesDir, "torrent-sources")
                        if (!directory.exists() && !directory.mkdirs()) {
                            throw IOException("Could not create private torrent storage")
                        }
                        val destination = File(directory, "${UUID.randomUUID()}.torrent")
                        try {
                            val input = context.contentResolver.openInputStream(uri)
                                ?: throw IOException("Could not open the selected torrent document")
                            input.use { source ->
                                destination.outputStream().use { output ->
                                    val buffer = ByteArray(8192)
                                    var copiedBytes = 0
                                    while (true) {
                                        val count = source.read(buffer)
                                        if (count < 0) break
                                        copiedBytes += count
                                        if (copiedBytes > MAX_TORRENT_FILE_BYTES) {
                                            throw IOException("Torrent file exceeds the 8 MiB limit")
                                        }
                                        output.write(buffer, 0, count)
                                    }
                                }
                            }
                            destination
                        } catch (error: IOException) {
                            destination.delete()
                            throw error
                        } catch (error: SecurityException) {
                            destination.delete()
                            throw error
                        }

                    }
                    url = sourceFile.absolutePath
                    urlError = null
                    duplicateWarning = null
                    duplicateCheckConfirmed = false
                    torrentMetadata = null
                    selectedTorrentFiles = emptySet()
                } catch (error: IOException) {
                    urlError = error.message ?: "Could not read the selected torrent file"
                } catch (error: SecurityException) {
                    urlError = "Permission to read the selected torrent file was denied"
                }
            }
        }
    }

    // Auto-derive fileName from URL's last path segment when the user has not edited it manually.
    LaunchedEffect(url) {
        if (!userEditedFileName) {
            val derived = url.trimEnd('/').substringAfterLast('/')
            if (derived.isNotBlank() && derived != url) {
                fileName = derived
            }
        }
    }
    LaunchedEffect(bulkUrls.firstOrNull(), packageName) {
        if (packageName.isBlank()) {
            val firstUrl = bulkUrls.firstOrNull()
            if (!firstUrl.isNullOrBlank()) {
                packageName = BulkLinkParser.suggestedPackageName(
                    firstUrl,
                    BulkLinkParser.suggestedFileName(firstUrl),
                )
            }
        }
    }

    val rawFileName = fileName.ifBlank {
        url.trimEnd('/').substringAfterLast('/').ifBlank { "download" }
    }
    val resolvedFileName = remember(downloadsDir, rawFileName, reservedFileNames) {
        BridgeInitializer.publicDownloadStorageOrNull()?.uniquePublicFileName(
            rawFileName,
            reservedFileNames = reservedFileNames,
        )
            ?: dev.fetchx.downloads.storage.StorageManager(context)
                .uniqueFileName(java.io.File(downloadsDir), rawFileName, reservedFileNames)
    }
    val storageManager = remember(context) { StorageManager(context) }
    val destinationDirectory = remember(downloadsDir, fileName, url) {
        storageManager.resolveSmartDownloadDirectory(downloadsDir, resolvedFileName, null)
    }
    val destinationPath = "$destinationDirectory/$resolvedFileName"

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add download") },
        text = {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
            ) {
                // URL field
                OutlinedTextField(
                    value = url,
                    onValueChange = { newValue ->
                        url = newValue
                        urlError = null   // clear error while user types
                        duplicateWarning = null
                        duplicateCheckConfirmed = false
                        torrentMetadata = null
                        selectedTorrentFiles = emptySet()
                    },
                    label = { Text("URL, magnet link, or local .torrent path") },
                    placeholder = { Text("https://example.com/file.zip, magnet URI, or /path/to/file.torrent") },
                    isError = urlError != null,
                    supportingText = urlError?.let { { Text(it, color = MaterialTheme.colorScheme.error) } },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )

                val trimmed = url.trim()
                val isInfoHash = Regex("^[0-9a-fA-F]{40}$").matches(trimmed) ||
                    Regex("^(btih:|urn:btih:)[0-9a-fA-F]{40}$", RegexOption.IGNORE_CASE).matches(trimmed)

                if (isInfoHash) {
                    Text(
                        text = "BitTorrent info-hash — will search for peers automatically",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(top = 4.dp),
                    )
                }

                if (bulkUrls.size > 1) {
                    Text(
                        text = "${bulkUrls.size} unique links detected",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(top = 4.dp),
                    )
                }

                duplicateWarning?.let { warning ->
                    Text(
                        text = warning,
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodySmall,
                    )
                }

                Button(
                    onClick = {
                        torrentPicker.launch(
                            arrayOf("application/x-bittorrent", "application/octet-stream")
                        )
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Choose .torrent file")
                }

                if (!url.contains("://") && url.trim().endsWith(".torrent", ignoreCase = true)) {
                    TextButton(
                        onClick = {
                            scope.launch {
                                try {
                                    val metadata = onInspectTorrent(url.trim())
                                    torrentMetadata = metadata
                                    selectedTorrentFiles = metadata.files.map { it.index }.toSet()
                                    urlError = null
                                } catch (error: CancellationException) {
                                    throw error
                                } catch (error: Exception) {
                                    urlError = "Could not inspect the torrent file"
                                }
                            }
                        },
                    ) {
                        Text("Inspect torrent files")
                    }
                }

                torrentMetadata?.let { metadata ->
                    Text(
                        text = "${metadata.name} · ${formatBytes(metadata.totalSizeBytes)}",
                        style = MaterialTheme.typography.labelMedium,
                    )
                    val directories = metadata.files
                        .flatMap { file ->
                            val segments = file.path.split('/')
                            (1 until segments.size).map { length -> segments.take(length).joinToString("/") }
                        }
                        .distinct()
                    directories.forEach { directory ->
                        val childIndexes = metadata.files
                            .filter { it.path.startsWith("$directory/") }
                            .map { it.index }
                            .toSet()
                        if (childIndexes.isNotEmpty()) {
                            val checked = childIndexes.all(selectedTorrentFiles::contains)
                            TextButton(
                                onClick = {
                                    selectedTorrentFiles = if (checked) {
                                        selectedTorrentFiles - childIndexes
                                    } else {
                                        selectedTorrentFiles + childIndexes
                                    }
                                },
                            ) {
                                Text("${if (checked) "Deselect" else "Select"} $directory")
                            }
                        }
                    }
                    TextButton(onClick = { selectedTorrentFiles = metadata.files.map { it.index }.toSet() }) {
                        Text("Select all")
                    }
                    TextButton(onClick = { selectedTorrentFiles = emptySet() }) {
                        Text("Deselect all")
                    }
                    metadata.files.forEach { file ->
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
                        ) {
                            Switch(
                                checked = file.index in selectedTorrentFiles,
                                onCheckedChange = { checked ->
                                    selectedTorrentFiles = if (checked) {
                                        selectedTorrentFiles + file.index
                                    } else {
                                        selectedTorrentFiles - file.index
                                    }
                                },
                            )
                            Text(
                                text = file.path,
                                modifier = Modifier.weight(1f),
                                style = MaterialTheme.typography.bodySmall,
                            )
                            Text(formatBytes(file.sizeBytes), style = MaterialTheme.typography.labelSmall)
                        }
                    }
                    val selectedSize = metadata.files
                        .filter { it.index in selectedTorrentFiles }
                        .sumOf { it.sizeBytes }
                    Text(
                        text = "Selected: ${selectedTorrentFiles.size} file(s) · ${formatBytes(selectedSize)}",
                        style = MaterialTheme.typography.labelMedium,
                    )
                }

                Spacer(modifier = Modifier.height(8.dp))

                // File name field
                OutlinedTextField(
                    value = fileName,
                    onValueChange = { newValue ->
                        fileName = newValue
                        userEditedFileName = newValue.isNotBlank()
                    },
                    label = { Text("File name") },
                    placeholder = { Text("file.zip") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )

                Spacer(modifier = Modifier.height(4.dp))

                // Destination hint (read-only)
                Text(
                    text = "Saves to: $downloadsDir",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 4.dp),
                )

                Spacer(modifier = Modifier.height(16.dp))

                OutlinedTextField(
                    value = packageName,
                    onValueChange = { packageName = it },
                    label = { Text("Package name") },
                    placeholder = { Text("FetchX Downloads") },
                    supportingText = { Text("Related links will be stored in this package folder.") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )

                Spacer(modifier = Modifier.height(16.dp))

                OutlinedTextField(
                    value = expectedSha256,
                    onValueChange = { expectedSha256 = it.trim() },
                    label = { Text("Expected SHA-256 (optional)") },
                    placeholder = { Text("64 hexadecimal characters") },
                    supportingText = {
                        if (expectedSha256.isNotEmpty() &&
                            !Regex("^[0-9a-fA-F]{64}$").matches(expectedSha256)
                        ) {
                            Text("Enter exactly 64 hexadecimal characters")
                        }
                    },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )

                Spacer(modifier = Modifier.height(16.dp))

                Text(
                    text = "Priority",
                    style = MaterialTheme.typography.labelMedium,
                    modifier = Modifier.padding(bottom = 4.dp),
                )

                // Priority chips — CRITICAL must not appear in the UI.
                Row {
                    listOf(DownloadPriority.LOW, DownloadPriority.NORMAL, DownloadPriority.HIGH)
                        .forEach { priority ->
                            FilterChip(
                                selected = selectedPriority == priority,
                                onClick = { selectedPriority = priority },
                                label = {
                                    Text(
                                        text = priority.name.lowercase()
                                            .replaceFirstChar { it.uppercase() }
                                    )
                                },
                                modifier = Modifier.padding(end = 8.dp),
                            )
                        }
                }

                // ── HLS quality picker ────────────────────────────────────
                val trimmedUrl = url.trim()
                if (trimmedUrl.endsWith(".m3u8", ignoreCase = true)) {
                    Spacer(modifier = Modifier.height(8.dp))
                    Text("HLS stream", style = MaterialTheme.typography.labelMedium)
                    if (hlsVariants.isEmpty()) {
                        Button(
                            onClick = {
                                scope.launch {
                                    isFetchingHls = true
                                    try {
                                        val variants = onDiscoverHlsVariants(trimmedUrl)
                                        hlsVariants = variants
                                        selectedHlsVariant = variants.firstOrNull()
                                    } catch (e: CancellationException) {
                                        throw e
                                    } catch (_: Exception) {
                                        urlError = "Could not fetch HLS qualities"
                                    } finally {
                                        isFetchingHls = false
                                    }
                                }
                            },
                            enabled = !isFetchingHls,
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Text(if (isFetchingHls) "Fetching…" else "Fetch qualities")
                        }
                    } else {
                        TextButton(
                            onClick = { hlsDropdownExpanded = true },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Text(selectedHlsVariant?.label ?: "Select quality")
                        }
                        DropdownMenu(
                            expanded = hlsDropdownExpanded,
                            onDismissRequest = { hlsDropdownExpanded = false },
                        ) {
                            hlsVariants.forEach { variant ->
                                DropdownMenuItem(
                                    text = { Text(variant.label) },
                                    onClick = {
                                        selectedHlsVariant = variant
                                        hlsDropdownExpanded = false
                                    },
                                )
                            }
                        }
                    }
                }

                // ── Subtitle section (shown for media/yt-dlp URLs) ────────
                val isMediaLike = isMediaLikeUrl(trimmedUrl)
                if (isMediaLike) {
                    Spacer(modifier = Modifier.height(8.dp))
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = androidx.compose.foundation.layout.Arrangement.SpaceBetween,
                    ) {
                        Text("Download subtitles", style = MaterialTheme.typography.labelMedium)
                        Switch(
                            checked = subtitleEnabled,
                            onCheckedChange = { subtitleEnabled = it },
                        )
                    }
                    if (subtitleEnabled) {
                        OutlinedTextField(
                            value = subtitleLangs,
                            onValueChange = { subtitleLangs = it },
                            label = { Text("Languages") },
                            placeholder = { Text("en") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                }

                // ── Advanced section ──────────────────────────────────────
                Spacer(modifier = Modifier.height(8.dp))
                TextButton(
                    onClick = { showAdvanced = !showAdvanced },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(if (showAdvanced) "▲ Advanced" else "▼ Advanced")
                }
                AnimatedVisibility(visible = showAdvanced) {
                    Column {
                        // Auth type selector
                        Text("Authentication", style = MaterialTheme.typography.labelMedium)
                        Row {
                            listOf(null, HttpAuthType.BASIC, HttpAuthType.DIGEST, HttpAuthType.NTLM)
                                .forEach { type ->
                                    FilterChip(
                                        selected = authType == type,
                                        onClick = { authType = type },
                                        label = { Text(type?.name?.lowercase()?.replaceFirstChar { it.uppercase() } ?: "None") },
                                        modifier = Modifier.padding(end = 4.dp),
                                    )
                                }
                        }
                        if (authType != null) {
                            OutlinedTextField(
                                value = authUsername,
                                onValueChange = { authUsername = it },
                                label = { Text("Username") },
                                singleLine = true,
                                modifier = Modifier.fillMaxWidth(),
                            )
                            OutlinedTextField(
                                value = authPassword,
                                onValueChange = { authPassword = it },
                                label = { Text("Password") },
                                singleLine = true,
                                modifier = Modifier.fillMaxWidth(),
                            )
                        }
                        if (authType == HttpAuthType.NTLM) {
                            OutlinedTextField(
                                value = authDomain,
                                onValueChange = { authDomain = it },
                                label = { Text("Domain") },
                                singleLine = true,
                                modifier = Modifier.fillMaxWidth(),
                            )
                        }
                        Spacer(modifier = Modifier.height(8.dp))
                        OutlinedTextField(
                            value = proxyUrl,
                            onValueChange = { proxyUrl = it },
                            label = { Text("Proxy URL (optional)") },
                            placeholder = { Text("http://proxy:8080") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth(),
                        )
                        OutlinedTextField(
                            value = userAgent,
                            onValueChange = { userAgent = it },
                            label = { Text("User-Agent (optional)") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    val error = validateUrl(url)
                    if (error != null) {
                        urlError = error
                    } else if (torrentMetadata != null && selectedTorrentFiles.isEmpty()) {
                        urlError = "Select at least one torrent file."
                    } else {
                        val selected = torrentMetadata?.let { selectedTorrentFiles.sorted() }
                        val checksum = expectedSha256.takeIf { it.isNotEmpty() }
                        if (checksum != null && !Regex("^[0-9a-fA-F]{64}$").matches(checksum)) {
                            return@TextButton
                        }
                        scope.launch {
                            if (!duplicateCheckConfirmed) {
                                isCheckingDuplicate = true
                                urlError = null
                                try {
                                    val candidates = if (bulkUrls.size > 1 && selectedTorrentFiles.isEmpty()) {
                                        bulkUrls
                                    } else {
                                        listOf(url.trim())
                                    }
                                    val duplicateNames = candidates
                                        .filter {
                                            it.startsWith("http://", ignoreCase = true) ||
                                                it.startsWith("https://", ignoreCase = true)
                                        }
                                        .mapNotNull { onFindDuplicate(it) }
                                    duplicateCheckConfirmed = true
                                    if (duplicateNames.isNotEmpty()) {
                                        val count = duplicateNames.size
                                        duplicateWarning = buildString {
                                            append("$count matching download")
                                            if (count != 1) append('s')
                                            append(" already exist: ")
                                            append(duplicateNames.joinToString())
                                            append(". Add anyway?")
                                        }
                                        return@launch
                                    }
                                } catch (error: CancellationException) {
                                    throw error
                                } catch (_: Exception) {
                                    urlError = "Could not check for an existing download. Please try again."
                                    duplicateCheckConfirmed = false
                                    return@launch
                                } finally {
                                    isCheckingDuplicate = false
                                }
                            }
                            duplicateWarning = null
                            if (bulkUrls.size > 1 && selectedTorrentFiles.isEmpty()) {
                                val batchItems = bulkUrls.map { link ->
                                    val rawName = BulkLinkParser.suggestedFileName(link)
                                    val bulkFileName = BridgeInitializer.publicDownloadStorageOrNull()
                                        ?.uniquePublicFileName(rawName, reservedFileNames = reservedFileNames)
                                        ?: StorageManager(context).uniqueFileName(
                                            File(downloadsDir),
                                            rawName,
                                            reservedFileNames,
                                        )
                                    val bulkDestination = StorageManager(context).resolveSmartDownloadDirectory(
                                        downloadsDir,
                                        bulkFileName,
                                        null,
                                    )
                                    val packageFolder = packageFolderName(packageName)
                                    val packagedDestination = if (packageFolder == null) {
                                        "$bulkDestination/$bulkFileName"
                                    } else {
                                        "$downloadsDir/$packageFolder/$bulkFileName"
                                    }
                                    DownloadBatchItem(
                                        url = link,
                                        fileName = bulkFileName,
                                        destinationPath = packagedDestination,
                                    )
                                }
                                onAddBatch(batchItems, selectedPriority, checksum)
                            } else {
                                val packageFolder = packageFolderName(packageName)
                                val finalDest = if (packageFolder == null) {
                                    destinationPath
                                } else {
                                    "$downloadsDir/$packageFolder/$resolvedFileName"
                                }
                                // Use the selected HLS variant URL if available
                                val finalUrl = selectedHlsVariant?.url ?: url.trim()
                                val hasAdvanced = authType != null || proxyUrl.isNotBlank() || userAgent.isNotBlank()
                                val hasSubtitles = isMediaLikeUrl(finalUrl) && subtitleEnabled
                                if (hasAdvanced || hasSubtitles) {
                                    val credentials = if (authType != null) {
                                        HttpCredentials(
                                            authType = authType!!,
                                            username = authUsername,
                                            password = authPassword,
                                            domain = authDomain.takeIf { it.isNotBlank() },
                                        )
                                    } else null
                                    onAddAdvanced(
                                        finalUrl,
                                        resolvedFileName,
                                        finalDest,
                                        selectedPriority,
                                        credentials,
                                        AdvancedDownloadOptions(
                                            proxyUrl = proxyUrl.takeIf { it.isNotBlank() },
                                            userAgent = userAgent.takeIf { it.isNotBlank() },
                                        ),
                                        hasSubtitles,
                                        subtitleLangs,
                                    )
                                } else {
                                    val selected = torrentMetadata?.let { selectedTorrentFiles.sorted() }
                                    onAdd(finalUrl, resolvedFileName, finalDest, selectedPriority, selected, checksum)
                                }
                            }
                            onDismiss()
                        }
                    }
                },
                enabled = url.isNotBlank() && !isCheckingDuplicate,
            ) {
                Text(
                    if (isCheckingDuplicate) "Checking..."
                    else if (duplicateWarning != null) "Add anyway"
                    else "Add",
                )
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

/**
 * Client-side URL format check.
 * Full validation is performed by the Rust core — this only catches obvious errors early.
 * Exposed as `internal` so it can be tested from `:downloads` unit tests.
 *
 * @return a user-readable error string, or null if the URL passes basic checks.
 */
internal fun validateUrl(url: String): String? {
    val trimmed = url.trim()
    if (trimmed.isBlank()) return "URL cannot be empty"
    val isInfoHash = Regex("^[0-9a-fA-F]{40}$").matches(trimmed) ||
        Regex("^(btih:|urn:btih:)[0-9a-fA-F]{40}$", RegexOption.IGNORE_CASE).matches(trimmed)
    if (isInfoHash) return null
    if (!trimmed.startsWith("http://", ignoreCase = true) &&
        !trimmed.startsWith("https://", ignoreCase = true) &&
        !trimmed.startsWith("ftp://", ignoreCase = true) &&
        !trimmed.startsWith("ftps://", ignoreCase = true) &&
        !trimmed.startsWith("sftp://", ignoreCase = true) &&
        !trimmed.startsWith("magnet:", ignoreCase = true) &&
        (trimmed.contains("://") || !trimmed.endsWith(".torrent", ignoreCase = true))
    ) {
        return "Enter a supported download URL, magnet URI, info-hash, or local .torrent path"
    }

    return null
}

internal fun packageFolderName(name: String): String? {
    if (name.trim().let { it == "." || it == ".." }) return null
    val sanitized = name.trim()
        .replace(Regex("[\\\\/:*?\"<>|\\u0000-\\u001F]"), "_")
        .trim('.', ' ')
        .ifBlank { "FetchX Downloads" }
    return sanitized.takeIf { it != "." && it != ".." }
}

internal fun isMediaLikeUrl(url: String): Boolean {
    val lower = url.lowercase()
    return lower.contains("youtube.com") ||
        lower.contains("youtu.be") ||
        lower.contains("vimeo.com") ||
        lower.contains("twitch.tv") ||
        lower.endsWith(".mp4") ||
        lower.endsWith(".mkv") ||
        lower.endsWith(".webm") ||
        lower.endsWith(".m4v") ||
        lower.endsWith(".avi") ||
        lower.endsWith(".mov") ||
        lower.endsWith(".m3u8")
}
