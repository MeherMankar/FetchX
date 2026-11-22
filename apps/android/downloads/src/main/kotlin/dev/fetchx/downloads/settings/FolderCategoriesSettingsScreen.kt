package dev.fetchx.downloads.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

private data class CategoryRowState(
    val label: String,
    val key: String,
    var folder: String,
    var extensions: String,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FolderCategoriesSettingsScreen(
    config: FolderCategoriesConfig,
    onSave: (FolderCategoriesConfig) -> Unit,
    onNavigateUp: () -> Unit,
) {
    var enabled by remember { mutableStateOf(config.enabled) }
    var custom by remember { mutableStateOf(config.custom) }

    // One mutable state entry per built-in category row.
    val rows = remember {
        mutableListOf(
            CategoryRowState("Compressed", "compressed", config.compressed.folder, config.compressed.extensions),
            CategoryRowState("Documents", "documents", config.documents.folder, config.documents.extensions),
            CategoryRowState("Music", "music", config.music.folder, config.music.extensions),
            CategoryRowState("Programs", "programs", config.programs.folder, config.programs.extensions),
            CategoryRowState("Video", "video", config.video.folder, config.video.extensions),
            CategoryRowState("Images", "images", config.images.folder, config.images.extensions),
        )
    }
    // Track row state changes so Compose re-composes.
    val rowStates = remember { rows.map { mutableStateOf(it) } }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Folder categories") },
                navigationIcon = {
                    IconButton(onClick = onNavigateUp) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { padding ->
        LazyColumn(
            modifier = Modifier
                .padding(padding)
                .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Auto-categorise downloads", modifier = Modifier.weight(1f))
                    Switch(checked = enabled, onCheckedChange = { enabled = it })
                }
            }

            if (enabled) {
                items(rowStates) { rowState ->
                    var row by rowState
                    Column(modifier = Modifier.fillMaxWidth()) {
                        Text(row.label)
                        Spacer(modifier = Modifier.height(4.dp))
                        OutlinedTextField(
                            value = row.folder,
                            onValueChange = { row = row.copy(folder = it) },
                            label = { Text("Folder") },
                            modifier = Modifier.fillMaxWidth(),
                            singleLine = true,
                        )
                        Spacer(modifier = Modifier.height(4.dp))
                        OutlinedTextField(
                            value = row.extensions,
                            onValueChange = { row = row.copy(extensions = it) },
                            label = { Text("Extensions (comma-separated)") },
                            modifier = Modifier.fillMaxWidth(),
                            singleLine = true,
                        )
                    }
                }

                item {
                    OutlinedTextField(
                        value = custom,
                        onValueChange = { custom = it },
                        label = { Text("Custom rules") },
                        placeholder = { Text("Books=epub,mobi;3D=stl,obj") },
                        modifier = Modifier.fillMaxWidth(),
                        minLines = 3,
                        supportingText = { Text("Format: Folder=ext,ext;Folder2=ext. Custom rules override the built-in categories.") },
                    )
                }
            }

            item {
                Spacer(modifier = Modifier.height(8.dp))
                Button(
                    onClick = {
                        val states = rowStates.map { it.value }
                        onSave(
                            config.copy(
                                enabled = enabled,
                                compressed = CategoryRule(states[0].folder, states[0].extensions),
                                documents = CategoryRule(states[1].folder, states[1].extensions),
                                music = CategoryRule(states[2].folder, states[2].extensions),
                                programs = CategoryRule(states[3].folder, states[3].extensions),
                                video = CategoryRule(states[4].folder, states[4].extensions),
                                images = CategoryRule(states[5].folder, states[5].extensions),
                                custom = custom,
                            ),
                        )
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Save")
                }
            }
        }
    }
}
