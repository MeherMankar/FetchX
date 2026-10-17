package dev.fetchx.downloads.ui

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
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.fetchx.core.download.DownloadPriority
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.util.UUID

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
 * @param onAdd           Called with (url, fileName, destinationPath, priority) when
 *                        the user confirms and validation passes.
 * @param onDismiss       Called when the user cancels or dismisses the dialog.
 */
@Composable
fun AddDownloadDialog(
    downloadsDir: String,
    onAdd: (url: String, fileName: String, destinationPath: String, priority: DownloadPriority) -> Unit,
    onDismiss: () -> Unit,
) {
    var url by rememberSaveable { mutableStateOf("") }
    var fileName by rememberSaveable { mutableStateOf("") }
    var userEditedFileName by rememberSaveable { mutableStateOf(false) }
    var urlError by rememberSaveable { mutableStateOf<String?>(null) }
    var selectedPriority by rememberSaveable { mutableStateOf(DownloadPriority.NORMAL) }
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
                                    source.copyTo(output)
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

    val rawFileName = fileName.ifBlank {
        url.trimEnd('/').substringAfterLast('/').ifBlank { "download" }
    }
    val resolvedFileName = dev.fetchx.downloads.storage.StorageManager(context)
        .uniqueFileName(java.io.File(downloadsDir), rawFileName)
    val destinationPath = "$downloadsDir/$resolvedFileName"

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add Download") },
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
                    },
                    label = { Text("URL, magnet link, or local .torrent path") },
                    placeholder = { Text("https://example.com/file.zip, magnet URI, or /path/to/file.torrent") },
                    isError = urlError != null,
                    supportingText = urlError?.let { { Text(it, color = MaterialTheme.colorScheme.error) } },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )

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
                    text = "Saves to: $destinationPath",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 4.dp),
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
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    val error = validateUrl(url)
                    if (error != null) {
                        urlError = error
                    } else {
                        // onAdd is responsible for closing the dialog (sets showAddDialog = false
                        // in the caller). Do NOT call onDismiss() here — that would cause a
                        // double-dismiss which triggers extra recompositions.
                        onAdd(url.trim(), resolvedFileName, destinationPath, selectedPriority)
                    }
                },
                enabled = url.isNotBlank(),
            ) {
                Text("Add")
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
    if (!trimmed.startsWith("http://", ignoreCase = true) &&
        !trimmed.startsWith("https://", ignoreCase = true) &&
        !trimmed.startsWith("magnet:", ignoreCase = true) &&
        (trimmed.contains("://") || !trimmed.endsWith(".torrent", ignoreCase = true))
    ) {
        return "Enter an HTTP(S) URL, magnet URI, or local .torrent path"
    }
    return null
}
