package dev.mydownloader.browser

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import org.mozilla.geckoview.GeckoView

/**
 * Full-screen browser powered by GeckoView, with multi-tab support.
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

                // Tab bar — horizontal scrollable row of FilterChips + new-tab button
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
}
