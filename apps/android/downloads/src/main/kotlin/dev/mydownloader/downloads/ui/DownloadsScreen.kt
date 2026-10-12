package dev.mydownloader.downloads.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.lifecycle.compose.collectAsStateWithLifecycle

private val TABS = listOf("Active", "Queued", "Completed", "Failed")

/**
 * Top-level Downloads screen with four tabs: Active, Queued, Completed, Failed.
 * The [viewModel] must be created with [DownloadsViewModel.Factory] and passed in
 * from the NavGraph so the factory has access to the bridge repository.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DownloadsScreen(
    viewModel: DownloadsViewModel
) {
    @Suppress("UNUSED_VARIABLE")
    val downloads by viewModel.downloads.collectAsStateWithLifecycle()
    var selectedTab by remember { mutableIntStateOf(0) }

    Scaffold(
        topBar = {
            TopAppBar(title = { Text("Downloads") })
        }
    ) { innerPadding ->
        Column(modifier = Modifier.padding(innerPadding)) {
            TabRow(selectedTabIndex = selectedTab) {
                TABS.forEachIndexed { index, title ->
                    Tab(
                        selected = selectedTab == index,
                        onClick = { selectedTab = index },
                        text = { Text(title) }
                    )
                }
            }

            val filtered = when (selectedTab) {
                0 -> viewModel.activeDownloads
                1 -> viewModel.queuedDownloads
                2 -> viewModel.completedDownloads
                3 -> viewModel.failedDownloads
                else -> emptyList()
            }

            if (filtered.isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    Text(
                        text = "No ${TABS[selectedTab].lowercase()} downloads",
                        style = MaterialTheme.typography.bodyMedium,
                        textAlign = TextAlign.Center,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            } else {
                LazyColumn(modifier = Modifier.fillMaxSize()) {
                    items(filtered, key = { it.id }) { item ->
                        DownloadItemRow(
                            item = item,
                            onPause = { viewModel.pause(item.id) },
                            onResume = { viewModel.resume(item.id) },
                            onCancel = { viewModel.cancel(item.id) },
                            onRemove = { viewModel.remove(item.id) },
                            onRetry = { viewModel.retry(item.id) }
                        )
                    }
                }
            }
        }
    }
}
