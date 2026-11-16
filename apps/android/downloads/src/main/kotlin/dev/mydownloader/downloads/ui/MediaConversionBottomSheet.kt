package dev.fetchx.downloads.ui

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/** Bottom sheet for converting a completed download to another format. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MediaConversionBottomSheet(
    downloadId: String,
    filename: String,
    onDismiss: () -> Unit,
    onConvert: (downloadId: String, format: String) -> Unit,
) {
    val formats = listOf(
        "mp4" to "MP4 (remux)",
        "mkv" to "Matroska (remux)",
        "webm" to "WebM (remux)",
        "mp3" to "MP3 (audio extract)",
        "flac" to "FLAC (audio extract)",
        "wav" to "WAV (audio extract)",
    )

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text("Convert / Remux", style = MaterialTheme.typography.titleMedium)
            Text(
                filename,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
            HorizontalDivider(modifier = Modifier.padding(vertical = 4.dp))
            formats.forEach { (key, label) ->
                TextButton(
                    onClick = { onConvert(downloadId, key); onDismiss() },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(label)
                }
            }
            OutlinedButton(
                onClick = onDismiss,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Cancel")
            }
        }
    }
}
