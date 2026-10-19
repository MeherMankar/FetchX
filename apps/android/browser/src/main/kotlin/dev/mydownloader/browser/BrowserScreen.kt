package dev.fetchx.browser

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.BackHandler
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items as gridItems
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.only
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.AutoStories
import androidx.compose.material.icons.filled.Bookmarks
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Extension
import androidx.compose.material.icons.filled.Fullscreen
import androidx.compose.material.icons.filled.GridView
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Language
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.NorthWest
import androidx.compose.material.icons.filled.OndemandVideo
import androidx.compose.material.icons.filled.Palette
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Shield
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.filled.StarOutline
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material.icons.filled.TravelExplore
import androidx.compose.material.icons.filled.Update
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.Workspaces
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Snackbar
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel as composeViewModel
import kotlinx.coroutines.delay
import org.mozilla.geckoview.GeckoView
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Full-screen browser powered by GeckoView, with multi-tab support and history.
 *
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun BrowserScreen(
    onThemeColorChange: (Long) -> Unit = {},
    homeRequest: Long = 0L,
    searchRequest: Long = 0L,
    tabsRequest: Long = 0L,
    onHomeRequestHandled: () -> Unit = {},
    onSearchRequestHandled: () -> Unit = {},
    onTabsRequestHandled: () -> Unit = {},
    showBrowserMenu: Boolean = false,
    onBrowserMenuDismiss: () -> Unit = {},
    onOpenBrowserMenu: () -> Unit = {},
    onOpenStorage: () -> Unit = {},
    onNavigateToDownloads: () -> Unit = {},
    onDownloadCurrentPage: (url: String, title: String) -> Unit = { _, _ -> },
    onPlayVideo: (String) -> Unit = {},
    browserVersionName: String = "unknown",
    initialNavigateUrl: String? = null,
    viewModel: BrowserViewModel = composeViewModel(factory = BrowserViewModel.Factory),
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val activeTab = uiState.activeTab
    val extensionPicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri -> uri?.let(viewModel::installExtension) }
    val urlFocusRequester = remember { FocusRequester() }
    val keyboardController = LocalSoftwareKeyboardController.current
    var isUrlBarExpanded by rememberSaveable { mutableStateOf(false) }
    var internalShowMenu by rememberSaveable { mutableStateOf(false) }
    val isMenuVisible = showBrowserMenu || internalShowMenu
    val closeMenu = {
        internalShowMenu = false
        onBrowserMenuDismiss()
    }
    val openMenu = {
        internalShowMenu = true
        onOpenBrowserMenu()
    }
    var showBrowserUpdates by rememberSaveable { mutableStateOf(false) }

    BackHandler(enabled = isUrlBarExpanded) {
        isUrlBarExpanded = false
    }
    BackHandler(enabled = !isUrlBarExpanded && uiState.showTabOverview) {
        viewModel.closeTabOverview()
    }
    BackHandler(enabled = !isUrlBarExpanded && !uiState.showTabOverview && activeTab?.canGoBack == true) {
        viewModel.goBack()
    }

    LaunchedEffect(activeTab?.url) {
        viewModel.syncUrlInput(activeTab?.url?.takeUnless { it == "about:blank" }.orEmpty())
    }

    LaunchedEffect(homeRequest) {
        if (homeRequest > 0L) {
            isUrlBarExpanded = false
            viewModel.closeTabOverview()
            viewModel.navigate("about:blank")
            viewModel.syncUrlInput("")
            onHomeRequestHandled()
        }
    }
    LaunchedEffect(searchRequest) {
        if (searchRequest > 0L) {
            val currentUrl = activeTab?.url?.takeUnless { it == "about:blank" }.orEmpty()
            isUrlBarExpanded = true
            viewModel.closeTabOverview()
            viewModel.onUrlInputChanged(currentUrl)
            onSearchRequestHandled()
        }
    }
    LaunchedEffect(tabsRequest) {
        if (tabsRequest > 0L) {
            viewModel.openTabOverview()
            onTabsRequestHandled()
        }
    }
    LaunchedEffect(isUrlBarExpanded) {
        if (isUrlBarExpanded) {
            delay(100)
            urlFocusRequester.requestFocus()
            keyboardController?.show()
        }
    }

    // TextFieldValue lets us control the cursor/selection.
    // When the URL bar expands, select-all so the user can type immediately
    // without the old URL getting in the way (and so ADB input text replaces it).
    var urlFieldValue by remember { mutableStateOf(TextFieldValue("")) }
    LaunchedEffect(uiState.urlInput) {
        // Sync string state → TextFieldValue without disturbing an active selection
        if (urlFieldValue.text != uiState.urlInput) {
            urlFieldValue = TextFieldValue(uiState.urlInput)
        }
    }
    LaunchedEffect(isUrlBarExpanded) {
        if (isUrlBarExpanded) {
            // Select-all when bar opens so typing replaces the current URL
            val text = uiState.urlInput.takeUnless { it == "about:blank" }.orEmpty()
            urlFieldValue = TextFieldValue(text, selection = TextRange(0, text.length))
        }
    }

    // Navigate to a URL passed in from an external intent (e.g. adb shell am start -d "https://...")
    LaunchedEffect(initialNavigateUrl) {
        if (!initialNavigateUrl.isNullOrBlank()) {
            viewModel.navigate(initialNavigateUrl)
        }
    }

    // Direct download requests detected from web pages
    LaunchedEffect(uiState.pendingDownload) {
        uiState.pendingDownload?.let { download ->
            onDownloadCurrentPage(download.url, download.filename ?: "Download")
            viewModel.clearPendingDownload()
        }
    }

    // Magnet download feedback snackbar
    val magnetSnackbarHostState = remember { SnackbarHostState() }
    LaunchedEffect(uiState.magnetQueued, uiState.magnetError) {
        val queued = uiState.magnetQueued
        val error = uiState.magnetError
        when {
            queued != null -> {
                val result = magnetSnackbarHostState.showSnackbar(
                    message = "Magnet download added",
                    actionLabel = "Downloads",
                    duration = SnackbarDuration.Short,
                )
                if (result == SnackbarResult.ActionPerformed) {
                    onNavigateToDownloads()
                }
                viewModel.clearMagnetState()
            }
            error != null -> {
                magnetSnackbarHostState.showSnackbar(
                    message = error,
                    duration = SnackbarDuration.Short,
                )
                viewModel.clearMagnetState()
            }
        }
    }

    val onNavigate = {
        val intent = viewModel.submitSearch(urlFieldValue.text)
        if (intent !is SearchIntent.LocalSearch && intent != SearchIntent.Empty) {
            isUrlBarExpanded = false
        }
    }
    val spaceAccentColor = remember(uiState.selectedProfileId) {
        val hex = uiState.profiles.find { it.id == uiState.selectedProfileId }?.accent ?: "#4F8EF7"
        try { android.graphics.Color.parseColor(hex).let { androidx.compose.ui.graphics.Color(it) } }
        catch (_: Exception) { androidx.compose.ui.graphics.Color(0xFF4F8EF7) }
    }

    Scaffold(
        snackbarHost = { SnackbarHost(magnetSnackbarHostState) },
        topBar = {
            if (!uiState.compactMode) {
                Column {

                Surface(
                    color = MaterialTheme.colorScheme.surface.copy(alpha = 0.98f).let {
                        androidx.compose.ui.graphics.lerp(it, spaceAccentColor, 0.05f)
                    },
                    modifier = Modifier.windowInsetsPadding(WindowInsets.statusBars),
                    tonalElevation = 2.dp,
                ) {
                    if (isUrlBarExpanded) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 8.dp, vertical = 6.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            IconButton(
                                onClick = { isUrlBarExpanded = false },
                                modifier = Modifier.size(40.dp),
                            ) {
                                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Close search")
                            }

                            OutlinedTextField(
                                value = urlFieldValue,
                                onValueChange = { tfv ->
                                    urlFieldValue = tfv
                                    viewModel.onUrlInputChanged(tfv.text)
                                },
                                singleLine = true,
                                placeholder = { Text("Search or type URL") },
                                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Go),
                                keyboardActions = KeyboardActions(onGo = {
                                    isUrlBarExpanded = false
                                    onNavigate()
                                }),
                                leadingIcon = {
                                    Surface(
                                        onClick = { viewModel.openSearchEngineSelector() },
                                        shape = RoundedCornerShape(8.dp),
                                        color = MaterialTheme.colorScheme.surfaceVariant,
                                        modifier = Modifier.padding(start = 6.dp),
                                    ) {
                                        Row(
                                            verticalAlignment = Alignment.CenterVertically,
                                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                                        ) {
                                            Text(
                                                uiState.selectedSearchEngine.displayName.split(" ").first(),
                                                style = MaterialTheme.typography.labelSmall,
                                                color = MaterialTheme.colorScheme.primary,
                                            )
                                            Icon(
                                                Icons.Default.ArrowDropDown,
                                                contentDescription = "Select engine",
                                                modifier = Modifier.size(16.dp),
                                                tint = MaterialTheme.colorScheme.primary,
                                            )
                                        }
                                    }
                                },
                                trailingIcon = {
                                    Row(verticalAlignment = Alignment.CenterVertically) {
                                        if (urlFieldValue.text.isNotEmpty()) {
                                            IconButton(
                                                onClick = {
                                                    urlFieldValue = TextFieldValue("")
                                                    viewModel.onUrlInputChanged("")
                                                },
                                                modifier = Modifier.size(36.dp),
                                            ) {
                                                Icon(Icons.Default.Clear, "Clear text", modifier = Modifier.size(18.dp))
                                            }
                                        }
                                        IconButton(
                                            onClick = {
                                                isUrlBarExpanded = false
                                                onNavigate()
                                            },
                                            modifier = Modifier.size(36.dp),
                                        ) {
                                            Icon(
                                                Icons.AutoMirrored.Filled.ArrowForward,
                                                "Navigate",
                                                tint = MaterialTheme.colorScheme.primary,
                                            )
                                        }
                                    }
                                },
                                shape = RoundedCornerShape(24.dp),
                                colors = OutlinedTextFieldDefaults.colors(
                                    focusedBorderColor = MaterialTheme.colorScheme.primary,
                                    unfocusedBorderColor = MaterialTheme.colorScheme.outlineVariant,
                                ),
                                modifier = Modifier
                                    .weight(1f)
                                    .focusRequester(urlFocusRequester),
                            )
                        }
                    } else {
                        val url = uiState.activeTab?.url.orEmpty()
                        val isBlank = url == "about:blank" || url.isBlank()
                        val isHttps = url.startsWith("https://")
                        val displayAddress = if (isBlank) {
                            "Search or type URL"
                        } else {
                            try {
                                java.net.URI(url).host ?: url.removePrefix("https://").removePrefix("http://").substringBefore("/")
                            } catch (_: Exception) {
                                url.removePrefix("https://").removePrefix("http://").substringBefore("/")
                            }
                        }

                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 10.dp, vertical = 6.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            // Site security / tracker icon
                            IconButton(
                                onClick = {
                                    if (!isBlank) viewModel.openTrackerDetails()
                                    else viewModel.openSearchEngineSelector()
                                },
                                modifier = Modifier.size(36.dp),
                            ) {
                                if (isBlank) {
                                    Icon(
                                        Icons.Default.TravelExplore,
                                        contentDescription = "Search engine",
                                        tint = MaterialTheme.colorScheme.primary,
                                        modifier = Modifier.size(20.dp),
                                    )
                                } else if (isHttps) {
                                    Icon(
                                        Icons.Default.Lock,
                                        contentDescription = "Secure HTTPS",
                                        tint = Color(0xFF34A853),
                                        modifier = Modifier.size(18.dp),
                                    )
                                } else {
                                    Icon(
                                        Icons.Default.Warning,
                                        contentDescription = "Not secure",
                                        tint = MaterialTheme.colorScheme.error,
                                        modifier = Modifier.size(18.dp),
                                    )
                                }
                            }

                            // Address bar capsule
                            Surface(
                                onClick = {
                                    isUrlBarExpanded = true
                                    viewModel.onUrlInputChanged(
                                        url.takeUnless { it == "about:blank" }.orEmpty(),
                                    )
                                },
                                shape = RoundedCornerShape(24.dp),
                                color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.85f),
                                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f)),
                                modifier = Modifier
                                    .weight(1f)
                                    .height(44.dp),
                            ) {
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    modifier = Modifier.padding(horizontal = 12.dp),
                                ) {
                                    if (isBlank) {
                                        Icon(
                                            Icons.Default.Search,
                                            contentDescription = null,
                                            modifier = Modifier.size(16.dp),
                                            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                                        )
                                        Text(
                                            displayAddress,
                                            style = MaterialTheme.typography.bodyMedium,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                                            modifier = Modifier.padding(start = 8.dp),
                                        )
                                    } else {
                                        Text(
                                            displayAddress,
                                            style = MaterialTheme.typography.titleSmall,
                                            color = MaterialTheme.colorScheme.onSurface,
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                            modifier = Modifier.weight(1f, fill = false),
                                        )
                                        if (uiState.activeTabBlockedRequests > 0) {
                                            Spacer(Modifier.width(6.dp))
                                            Surface(
                                                shape = RoundedCornerShape(10.dp),
                                                color = MaterialTheme.colorScheme.primaryContainer,
                                                modifier = Modifier.padding(vertical = 2.dp),
                                            ) {
                                                Text(
                                                    text = "${uiState.activeTabBlockedRequests} blocked",
                                                    style = MaterialTheme.typography.labelSmall,
                                                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                                                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                                                )
                                            }
                                        }
                                    }
                                }
                            }

                            // Reload / Stop button
                            val isLoading = activeTab?.isLoading == true
                            IconButton(
                                onClick = { viewModel.reload() },
                                modifier = Modifier.size(36.dp),
                            ) {
                                if (isLoading) {
                                    Icon(
                                        Icons.Default.Close,
                                        contentDescription = "Stop loading",
                                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                        modifier = Modifier.size(20.dp),
                                    )
                                } else {
                                    Icon(
                                        Icons.Default.Refresh,
                                        contentDescription = "Reload",
                                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                        modifier = Modifier.size(20.dp),
                                    )
                                }
                            }

                            // Quick Bookmark star button
                            val isBookmarked = uiState.isCurrentPageBookmarked
                            val canBookmark = activeTab?.let { tab ->
                                (tab.url.startsWith("http://") || tab.url.startsWith("https://")) && !tab.isLoading
                            } == true
                            IconButton(
                                onClick = { viewModel.toggleBookmark() },
                                enabled = canBookmark,
                                modifier = Modifier.size(36.dp),
                            ) {
                                Icon(
                                    if (isBookmarked) Icons.Default.Star else Icons.Default.StarOutline,
                                    contentDescription = if (isBookmarked) "Bookmarked" else "Bookmark",
                                    tint = if (isBookmarked) spaceAccentColor
                                           else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = if (canBookmark) 1f else 0.4f),
                                    modifier = Modifier.size(20.dp),
                                )
                            }
                        }
                    }
                }

                if (uiState.activeTab?.isLoading == true) {
                    val currentProgress = (uiState.activeTab?.progress ?: 0) / 100f
                    if (currentProgress > 0.05f) {
                        LinearProgressIndicator(
                            progress = { currentProgress },
                            modifier = Modifier.fillMaxWidth().height(2.5.dp),
                            color = spaceAccentColor,
                        )
                    } else {
                        LinearProgressIndicator(
                            modifier = Modifier.fillMaxWidth().height(2.5.dp),
                            color = spaceAccentColor,
                        )
                    }
                }

                if (
                    isUrlBarExpanded &&
                    (
                        uiState.searchSuggestions.isNotEmpty() ||
                            uiState.isLoadingSearchSuggestions ||
                            uiState.searchSuggestionError != null ||
                            uiState.showLocalSearchEmpty
                        )
                ) {
                    Surface(
                        tonalElevation = 6.dp,
                        shape = RoundedCornerShape(16.dp),
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp),
                    ) {
                        Column(modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                            if (uiState.isLoadingSearchSuggestions) {
                                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                            }
                            uiState.searchSuggestionError?.let { error ->
                                Text(
                                    text = error,
                                    color = MaterialTheme.colorScheme.error,
                                    style = MaterialTheme.typography.bodySmall,
                                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                                )
                            }
                            if (uiState.showLocalSearchEmpty) {
                                Text(
                                    text = "No matching bookmarks, history, or open tabs",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                                )
                            }
                            uiState.searchSuggestions.take(8).forEach { suggestion ->
                                val icon = when {
                                    suggestion.category.contains("bookmark", ignoreCase = true) -> Icons.Default.Star
                                    suggestion.category.contains("history", ignoreCase = true) -> Icons.Default.History
                                    suggestion.category.contains("tab", ignoreCase = true) -> Icons.Default.GridView
                                    else -> Icons.Default.Search
                                }
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clickable {
                                            viewModel.applySuggestion(suggestion)
                                            isUrlBarExpanded = false
                                        }
                                        .padding(horizontal = 14.dp, vertical = 10.dp),
                                ) {
                                    Icon(
                                        icon,
                                        null,
                                        tint = MaterialTheme.colorScheme.primary,
                                        modifier = Modifier.size(18.dp),
                                    )
                                    Column(modifier = Modifier.weight(1f).padding(start = 12.dp)) {
                                        Text(
                                            suggestion.title,
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                            style = MaterialTheme.typography.bodyMedium,
                                        )
                                        Text(
                                            suggestion.category,
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                    IconButton(
                                        onClick = {
                                            urlFieldValue = TextFieldValue(
                                                suggestion.title,
                                                selection = TextRange(suggestion.title.length),
                                            )
                                            viewModel.onUrlInputChanged(suggestion.title)
                                        },
                                        modifier = Modifier.size(32.dp),
                                    ) {
                                        Icon(
                                            Icons.Default.NorthWest,
                                            contentDescription = "Complete query",
                                            modifier = Modifier.size(16.dp),
                                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
                }
            }
        },
        bottomBar = {
            if (!isUrlBarExpanded && !uiState.compactMode) {

                BrowserBottomBar(
                    canGoBack = activeTab?.canGoBack == true,
                    canGoForward = activeTab?.canGoForward == true,
                    tabCount = uiState.tabs.size,
                    onBack = { viewModel.goBack() },
                    onForward = { viewModel.goForward() },
                    onHome = {
                        viewModel.navigate("about:blank")
                        viewModel.syncUrlInput("")
                    },
                    onNewTab = { viewModel.newTabInCurrentSpace() },
                    onDownloads = onNavigateToDownloads,
                    onTabs = { viewModel.openTabOverview() },
                    onMenu = openMenu,
                    accentColor = spaceAccentColor,
                )
            }
        },
    ) { innerPadding ->
        Box(modifier = Modifier.padding(innerPadding).fillMaxSize()) {
            // Error banner
            uiState.activeTab?.errorText?.let { error ->
                Surface(
                    color = MaterialTheme.colorScheme.errorContainer,
                    modifier = Modifier.fillMaxWidth().align(Alignment.TopCenter),
                ) {
                    Text(
                        text = error,
                        color = MaterialTheme.colorScheme.onErrorContainer,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
            }

            if (isMenuVisible) {
                BrowserMenuSheet(
                    activeTab = activeTab,
                    uiState = uiState,
                    spaceAccentColor = spaceAccentColor,
                    onDismiss = closeMenu,
                    onNavigateHome = {
                        closeMenu()
                        viewModel.navigate("about:blank")
                        viewModel.syncUrlInput("")
                    },
                    onGoBack = {
                        closeMenu()
                        viewModel.goBack()
                    },
                    onGoForward = {
                        closeMenu()
                        viewModel.goForward()
                    },
                    onReload = {
                        closeMenu()
                        viewModel.reload()
                    },
                    onToggleBookmark = {
                        closeMenu()
                        viewModel.toggleBookmark()
                    },
                    onDownloadMedia = {
                        val tab = activeTab
                        if (tab != null) {
                            closeMenu()
                            onDownloadCurrentPage(tab.url, tab.title.orEmpty())
                        }
                    },
                    onNewTab = {
                        closeMenu()
                        viewModel.newTabInCurrentSpace()
                    },
                    onOpenTabs = {
                        closeMenu()
                        viewModel.openTabOverview()
                    },
                    onToggleReaderMode = {
                        closeMenu()
                        viewModel.toggleReaderMode()
                    },
                    onOpenTrackerDetails = {
                        closeMenu()
                        viewModel.openTrackerDetails()
                    },
                    onOpenPrivacySettings = {
                        closeMenu()
                        viewModel.openPrivacySettings()
                    },
                    onOpenBookmarks = {
                        closeMenu()
                        viewModel.openBookmarks()
                    },
                    onOpenHistory = {
                        closeMenu()
                        viewModel.openHistory()
                    },
                    onOpenDownloads = {
                        closeMenu()
                        onNavigateToDownloads()
                    },
                    onOpenExtensions = {
                        closeMenu()
                        viewModel.openExtensions()
                    },
                    onOpenSpaces = {
                        closeMenu()
                        viewModel.openSpaceSwitcher()
                    },
                    onOpenThemePicker = {
                        closeMenu()
                        viewModel.openThemePicker()
                    },
                    onToggleCompactMode = {
                        closeMenu()
                        viewModel.toggleCompactMode()
                    },
                    onOpenSettings = {
                        closeMenu()
                        viewModel.openBrowserSettings()
                    },
                    onOpenStorage = {
                        closeMenu()
                        onOpenStorage()
                    },
                    onOpenUpdates = {
                        closeMenu()
                        showBrowserUpdates = true
                    },
                )
            }

            // GeckoView
            val context = LocalContext.current
            AndroidView(
                factory = { ctx ->
                    GeckoView(ctx).also { viewModel.attachView(it, GeckoRuntimeHolder.getOrCreate(ctx)) }
                },
                onRelease = { view ->
                    view.releaseSession()
                },
                modifier = Modifier.fillMaxSize(),
            )

            // Floating pill to exit compact mode
            if (uiState.compactMode) {
                Surface(
                    onClick = { viewModel.toggleCompactMode() },
                    shape = CircleShape,
                    color = spaceAccentColor.copy(alpha = 0.92f),
                    shadowElevation = 8.dp,
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(bottom = 28.dp)
                        .size(36.dp),
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            Icons.Default.Fullscreen,
                            contentDescription = "Exit compact mode",
                            tint = Color.White,
                            modifier = Modifier.size(20.dp),
                        )
                    }
                }
            }


            // ── Essentials / new-tab page (Feature 3) ─────────────────────────
            if (uiState.showEssentials) {
                EssentialsPage(
                    essentials = uiState.essentials,
                    spaceAccentColor = spaceAccentColor,
                    profileName = uiState.selectedProfile?.name ?: "Default",
                    blockedCount = uiState.activeTabBlockedRequests,
                    onNavigate = { viewModel.navigateToEssential(it) },
                    onSearchTap = {
                        isUrlBarExpanded = true
                        viewModel.onUrlInputChanged("")
                    },
                    onOpenBookmarks = { viewModel.openBookmarks() },
                    onOpenHistory = { viewModel.openHistory() },
                    onOpenDownloads = onNavigateToDownloads,
                    onOpenTrackerDetails = { viewModel.openTrackerDetails() },
                    onOpenSpaces = { viewModel.openSpaceSwitcher() },
                    modifier = Modifier.fillMaxSize(),
                )
            }

            // ── Video detected pill (tap to open picker) ──────────────────────
            AnimatedVisibility(
                visible = uiState.showVideoDownloadBar && uiState.detectedVideos.isNotEmpty(),
                enter = slideInVertically(initialOffsetY = { it }),
                exit = slideOutVertically(targetOffsetY = { it }),
                modifier = Modifier.align(Alignment.BottomCenter),
            ) {
                var showPicker by remember { mutableStateOf(false) }
                val count = uiState.detectedVideos.size

                Surface(
                    tonalElevation = 6.dp,
                    shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp),
                    color = MaterialTheme.colorScheme.secondaryContainer,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .clickable { showPicker = true }
                            .padding(horizontal = 16.dp, vertical = 10.dp),
                    ) {
                        Icon(
                            Icons.Default.Download,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSecondaryContainer,
                            modifier = Modifier.size(20.dp),
                        )
                        Spacer(Modifier.width(8.dp))
                        Text(
                            if (count == 1) "1 video detected — tap to choose quality"
                            else "$count videos detected — tap to choose",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                            modifier = Modifier.weight(1f),
                        )
                        Spacer(Modifier.width(8.dp))
                        IconButton(
                            onClick = viewModel::dismissVideoBar,
                            modifier = Modifier.size(32.dp),
                        ) {
                            Icon(
                                Icons.Default.Close,
                                "Dismiss",
                                modifier = Modifier.size(16.dp),
                                tint = MaterialTheme.colorScheme.onSecondaryContainer,
                            )
                        }
                    }
                }

                // ── Video picker bottom sheet ──────────────────────────────────
                if (showPicker) {
                    ModalBottomSheet(onDismissRequest = { showPicker = false }) {
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(bottom = 32.dp),
                        ) {
                            Text(
                                "Choose video source",
                                style = MaterialTheme.typography.titleMedium,
                                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                            )
                            HorizontalDivider()
                            uiState.detectedVideos.forEach { video ->
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clickable {
                                            showPicker = false
                                            viewModel.requestMediaFormats(video)
                                        }
                                        .padding(horizontal = 16.dp, vertical = 12.dp),
                                ) {
                                    Icon(
                                        Icons.Default.OndemandVideo,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.primary,
                                        modifier = Modifier.size(24.dp),
                                    )
                                    Spacer(Modifier.width(12.dp))
                                    Column(modifier = Modifier.weight(1f)) {
                                        Text(
                                            video.suggestedFilename,
                                            style = MaterialTheme.typography.bodyMedium,
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                        )
                                        Text(
                                            video.mimeType?.substringBefore(";") ?: video.url.let {
                                                val ext = it.substringBefore("?").substringAfterLast(".", "")
                                                if (ext.isNotEmpty()) ext.uppercase() else "Video"
                                            },
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                        Text(
                                            video.url.substringBefore("?").take(60),
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                        )
                                    }
                                    Spacer(Modifier.width(4.dp))
                                    IconButton(
                                        onClick = {
                                            onPlayVideo(video.url)
                                        },
                                        modifier = Modifier.size(36.dp),
                                    ) {
                                        Icon(
                                            Icons.Default.PlayArrow,
                                            contentDescription = "Play",
                                            tint = MaterialTheme.colorScheme.primary,
                                            modifier = Modifier.size(20.dp),
                                        )
                                    }
                                    IconButton(
                                        onClick = {
                                            showPicker = false
                                            viewModel.requestMediaFormats(video)
                                        },
                                        modifier = Modifier.size(36.dp),
                                    ) {
                                        Icon(
                                            Icons.Default.Download,
                                            contentDescription = "Download",
                                            tint = MaterialTheme.colorScheme.primary,
                                            modifier = Modifier.size(20.dp),
                                        )
                                    }
                                }
                                HorizontalDivider(modifier = Modifier.padding(start = 52.dp))
                            }
                        }
                    }
                }
            }
        }
    }

    uiState.pendingMediaVideo?.let { video ->
        AlertDialog(
            onDismissRequest = viewModel::closeMediaFormatPicker,
            title = { Text("Choose video format") },
            text = {
                when {
                    uiState.isDiscoveringMediaFormats -> Text("Discovering available formats…")
                    uiState.mediaFormatError != null -> Column {
                        Text(uiState.mediaFormatError.orEmpty())
                        TextButton(onClick = { viewModel.requestMediaFormats(video) }) {
                            Text("Retry")
                        }
                        if (uiState.mediaFormats.isEmpty()) {
                            TextButton(
                                onClick = { viewModel.downloadDetectedVideo(video) },
                                enabled = !uiState.isQueueingMedia,
                            ) {
                                Text(if (uiState.isQueueingMedia) "Adding download…" else "Download automatically")
                            }
                        }
                    }
                    else -> Column(
                        modifier = Modifier
                            .heightIn(max = 400.dp)
                            .verticalScroll(rememberScrollState()),
                    ) {
                        uiState.mediaQueueError?.let { error ->
                            Text(error, color = MaterialTheme.colorScheme.error)
                        }
                        TextButton(
                            onClick = { viewModel.downloadDetectedVideo(video) },
                            enabled = !uiState.isQueueingMedia,
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Text(if (uiState.isQueueingMedia) "Adding download…" else "Automatic best quality")
                        }
                        uiState.mediaFormats.forEach { format ->
                            TextButton(
                                onClick = {
                                    viewModel.downloadDetectedVideo(
                                        video,
                                        format.formatId,
                                        format.extension,
                                        format.label,
                                        format.title,
                                        format.platform,
                                    )
                                },
                                modifier = Modifier.fillMaxWidth(),
                                enabled = !uiState.isQueueingMedia,
                            ) {
                                Text(format.label)
                            }
                        }
                        if (uiState.mediaFormats.isEmpty()) {
                            val otherStreams = uiState.detectedVideos
                                .filter { it.suggestedFilename != video.suggestedFilename || it.url != video.url }
                            if (otherStreams.isNotEmpty()) {
                                Text(
                                    "Other detected streams:",
                                    style = MaterialTheme.typography.labelMedium,
                                    modifier = Modifier.padding(top = 8.dp, bottom = 4.dp),
                                )
                                otherStreams.forEach { stream ->
                                    TextButton(
                                        onClick = { viewModel.downloadDetectedVideo(stream) },
                                        modifier = Modifier.fillMaxWidth(),
                                    ) {
                                        Text(stream.suggestedFilename)
                                    }
                                }
                            } else {
                                Text(
                                    "No selectable formats were found. Automatic quality selection is still available.",
                                    style = MaterialTheme.typography.bodySmall,
                                    modifier = Modifier.padding(top = 8.dp),
                                )
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = viewModel::closeMediaFormatPicker) {
                    Text("Cancel")
                }
            },
        )
    }

    // â”€â”€ Dialogs â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    if (uiState.showHistory) {
        var historyQuery by remember { mutableStateOf("") }
        val filteredHistory = uiState.historyEntries.filter {
            historyQuery.isBlank() || it.url.contains(historyQuery, ignoreCase = true) ||
                (it.title ?: "").contains(historyQuery, ignoreCase = true)
        }
        AlertDialog(
            onDismissRequest = { viewModel.closeHistory() },
            title = { Text("History") },
            text = {
                Column {
                    OutlinedTextField(
                        value = historyQuery,
                        onValueChange = { historyQuery = it },
                        singleLine = true,
                        placeholder = { Text("Search history") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    if (filteredHistory.isEmpty()) {
                        Text("No history yet.", style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier.padding(top = 12.dp))
                    } else {
                        LazyColumn(
                            modifier = Modifier.fillMaxWidth().heightIn(max = 320.dp),
                            verticalArrangement = Arrangement.spacedBy(0.dp),
                        ) {
                            items(filteredHistory, key = { it.id }) { entry ->
                                Column(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clickable { viewModel.navigateToHistoryEntry(entry.url) }
                                        .padding(vertical = 8.dp, horizontal = 4.dp),
                                ) {
                                    Text(entry.title ?: entry.url, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    Text(entry.url, style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    Text(formatHistoryTimestamp(entry.timestampMs),
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                                HorizontalDivider()
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { viewModel.clearHistory() }) { Text("Clear") }
            },
            dismissButton = {
                TextButton(onClick = { viewModel.closeHistory() }) { Text("Close") }
            },
        )
    }

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
            onAdBlockingChange = viewModel::setAdBlockingEnabled,
            onFingerprintingProtectionChange = viewModel::setFingerprintingProtectionEnabled,
            onCryptominingProtectionChange = viewModel::setCryptominingProtectionEnabled,
            onThirdPartyCookieIsolationChange = viewModel::setThirdPartyCookieIsolationEnabled,
            onHttpsOnlyChange = viewModel::setHttpsOnlyEnabled,
            onDismiss = viewModel::closePrivacySettings,
        )
    }

    // â”€â”€ Card-based tab grid (Quick Win #2 + #3 swipe-to-dismiss) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    if (uiState.showTabOverview) {
        TabGridSheet(
            tabs = uiState.tabs,
            activeTabId = uiState.activeTabId,
            profiles = uiState.profiles,
            selectedProfileId = uiState.selectedProfileId,
            spaceAccentColor = spaceAccentColor,
            onSelect = { viewModel.selectTab(it); viewModel.closeTabOverview() },
            onClose = { viewModel.closeTab(it) },
            onCloseAll = { spaceId -> viewModel.closeAllTabs(spaceId) },
            onNewTab = { spaceId -> viewModel.newTab(); viewModel.closeTabOverview() },
            onDismiss = viewModel::closeTabOverview,
            onOpenBrowserMenu = {
                viewModel.closeTabOverview()
                onOpenBrowserMenu()
            },
        )
    }


    if (uiState.showBrowserSettings) {
        BrowserSettingsDialog(
            searchSuggestionsEnabled = uiState.browserSuggestionsEnabled,
            onSearchSuggestionsChanged = viewModel::setBrowserSuggestionsEnabled,
            onDismiss = viewModel::closeBrowserSettings,
        )
    }

    if (showBrowserUpdates) {
        BrowserUpdatesDialog(
            versionName = browserVersionName,
            onDismiss = { showBrowserUpdates = false },
            onCheckForUpdates = {
                showBrowserUpdates = false
                viewModel.navigate("https://github.com/MeherMankar/FetchX/releases/latest")
            },
        )
    }

    // ── Space switcher (Feature 2) ──────────────────────────────────────────
    if (uiState.showSpaceSwitcher) {
        SpaceSwitcherDialog(
            profiles = uiState.profiles,
            selectedProfileId = uiState.selectedProfileId,
            onSelect = { viewModel.switchProfile(it) },
            onDismiss = { viewModel.closeSpaceSwitcher() },
        )
    }

    // ── Theme picker (Feature 4) ────────────────────────────────────────────
    if (uiState.showThemePicker) {
        ThemePickerDialog(
            currentSeed = uiState.themeSeedColor,
            onApply = { seed ->
                viewModel.setThemeSeedColor(seed)
                onThemeColorChange(seed)
            },
            onDismiss = { viewModel.closeThemePicker() },
        )
    }

    // ── Search engine selector ─────────────────────────────────────────────
    if (uiState.showSearchEngineSelector) {
        SearchEngineSelectorDialog(
            current = uiState.selectedSearchEngine,
            onSelect = viewModel::selectSearchEngine,
            onDismiss = viewModel::closeSearchEngineSelector,
        )
    }

    if (uiState.showProfileSwitcher) {
        ProfileSwitcherDialog(
            profiles = uiState.profiles,
            selectedProfileId = uiState.selectedProfileId,
            onSelect = viewModel::switchProfile,
            onDismiss = viewModel::closeProfileSwitcher,
        )
    }

    if (uiState.showExtensions) {
        ExtensionsDialog(
            state = uiState.extensionManagerState,
            isImporting = uiState.isImportingExtension,
            firefoxAddonsState = uiState.firefoxAddonsCatalogState,
            onDismiss = viewModel::closeExtensions,
            onRefresh = viewModel::refreshExtensions,
            onInstall = { extensionPicker.launch(arrayOf("application/x-xpinstall")) },
            onSearchFirefoxAddons = viewModel::searchFirefoxAddons,
            onInstallFirefoxAddon = viewModel::installFirefoxAddon,
            onEnable = viewModel::enableExtension,
            onDisable = viewModel::disableExtension,
            onRemove = viewModel::removeExtension,
        )
    }
    uiState.extensionManagerState.permissionRequest?.let { request ->
        ExtensionPermissionDialog(
            request = request,
            onResolve = viewModel::resolveExtensionPermissionRequest,
        )
    }
}

private val DEFAULT_TOP_SITES = listOf(
    Essential("preset-google", "Google", "https://www.google.com", "G", "#4285F4"),
    Essential("preset-yt", "YouTube", "https://www.youtube.com", "Y", "#EA4335"),
    Essential("preset-wiki", "Wikipedia", "https://www.wikipedia.org", "W", "#333333"),
    Essential("preset-github", "GitHub", "https://github.com", "G", "#24292E"),
    Essential("preset-reddit", "Reddit", "https://www.reddit.com", "R", "#FF4500"),
    Essential("preset-duck", "DuckDuckGo", "https://duckduckgo.com", "D", "#DE5833"),
)

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun EssentialsPage(
    essentials: List<Essential>,
    spaceAccentColor: Color,
    profileName: String,
    blockedCount: Long,
    onNavigate: (String) -> Unit,
    onSearchTap: () -> Unit,
    onOpenBookmarks: () -> Unit,
    onOpenHistory: () -> Unit,
    onOpenDownloads: () -> Unit,
    onOpenTrackerDetails: () -> Unit,
    onOpenSpaces: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val displaySites = essentials.ifEmpty { DEFAULT_TOP_SITES }

    Box(
        modifier = modifier
            .fillMaxSize()
            .background(
                brush = androidx.compose.ui.graphics.Brush.verticalGradient(
                    colors = listOf(
                        spaceAccentColor.copy(alpha = 0.12f),
                        MaterialTheme.colorScheme.background,
                    )
                )
            )
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 28.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Spacer(Modifier.height(16.dp))

            // Brand Header with Space Pill
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.Center,
            ) {
                Surface(
                    shape = CircleShape,
                    color = spaceAccentColor,
                    modifier = Modifier.size(32.dp),
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            Icons.Default.Language,
                            contentDescription = null,
                            tint = Color.White,
                            modifier = Modifier.size(18.dp),
                        )
                    }
                }
                Spacer(Modifier.width(10.dp))
                Text(
                    text = "FetchX",
                    style = MaterialTheme.typography.headlineMedium.copy(
                        fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
                    ),
                    color = MaterialTheme.colorScheme.onBackground,
                )
            }

            Spacer(Modifier.height(8.dp))

            // Space Switcher Pill
            Surface(
                onClick = onOpenSpaces,
                shape = RoundedCornerShape(16.dp),
                color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.7f),
                border = BorderStroke(1.dp, spaceAccentColor.copy(alpha = 0.3f)),
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
                ) {
                    Surface(
                        shape = CircleShape,
                        color = spaceAccentColor,
                        modifier = Modifier.size(8.dp),
                    ) {}
                    Spacer(Modifier.width(6.dp))
                    Text(
                        text = profileName,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Icon(
                        Icons.Default.ArrowDropDown,
                        contentDescription = null,
                        modifier = Modifier.size(14.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            Spacer(Modifier.height(28.dp))

            // Hero Search Bar (tap to search)
            Surface(
                onClick = onSearchTap,
                shape = RoundedCornerShape(28.dp),
                color = MaterialTheme.colorScheme.surfaceVariant,
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.6f)),
                tonalElevation = 3.dp,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(52.dp),
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.padding(horizontal = 16.dp),
                ) {
                    Icon(
                        Icons.Default.Search,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(20.dp),
                    )
                    Spacer(Modifier.width(12.dp))
                    Text(
                        text = "Search or type URL",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                        modifier = Modifier.weight(1f),
                    )
                }
            }

            Spacer(Modifier.height(24.dp))

            // Quick Action Shortcuts Row
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceEvenly,
            ) {
                QuickActionChip(
                    icon = Icons.Default.Bookmarks,
                    label = "Bookmarks",
                    onClick = onOpenBookmarks,
                )
                QuickActionChip(
                    icon = Icons.Default.History,
                    label = "History",
                    onClick = onOpenHistory,
                )
                QuickActionChip(
                    icon = Icons.Default.Download,
                    label = "Downloads",
                    onClick = onOpenDownloads,
                )
                QuickActionChip(
                    icon = Icons.Default.Shield,
                    label = "Protection",
                    onClick = onOpenTrackerDetails,
                )
            }

            Spacer(Modifier.height(32.dp))

            // Favorites / Top Sites Header
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = "Favorites",
                    style = MaterialTheme.typography.titleMedium.copy(
                        fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold,
                    ),
                    color = MaterialTheme.colorScheme.onBackground,
                )
                if (essentials.isEmpty()) {
                    Text(
                        text = "Suggested",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            Spacer(Modifier.height(14.dp))

            // Favorites Grid (rounded cards with colorful favicons)
            LazyVerticalGrid(
                columns = GridCells.Fixed(4),
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 220.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
                userScrollEnabled = false,
            ) {
                gridItems(displaySites.take(8), key = { it.id }) { item ->
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier
                            .clickable { onNavigate(item.url) }
                            .padding(vertical = 4.dp),
                    ) {
                        Surface(
                            shape = RoundedCornerShape(16.dp),
                            color = try {
                                Color(android.graphics.Color.parseColor(item.colorHex))
                            } catch (_: Exception) { spaceAccentColor },
                            shadowElevation = 2.dp,
                            modifier = Modifier.size(52.dp),
                        ) {
                            Box(contentAlignment = Alignment.Center) {
                                Text(
                                    text = item.initial,
                                    style = MaterialTheme.typography.titleMedium.copy(
                                        fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
                                    ),
                                    color = Color.White,
                                )
                            }
                        }
                        Spacer(Modifier.height(6.dp))
                        Text(
                            text = item.title,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurface,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }

            Spacer(Modifier.height(28.dp))

            // Privacy & Tracking Protection Banner Card
            Surface(
                onClick = onOpenTrackerDetails,
                shape = RoundedCornerShape(18.dp),
                color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f),
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.4f)),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.padding(16.dp),
                ) {
                    Surface(
                        shape = CircleShape,
                        color = Color(0xFF34A853).copy(alpha = 0.15f),
                        modifier = Modifier.size(40.dp),
                    ) {
                        Box(contentAlignment = Alignment.Center) {
                            Icon(
                                Icons.Default.Shield,
                                contentDescription = null,
                                tint = Color(0xFF34A853),
                                modifier = Modifier.size(22.dp),
                            )
                        }
                    }
                    Spacer(Modifier.width(14.dp))
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = "Tracking Protection Active",
                            style = MaterialTheme.typography.titleSmall.copy(
                                fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold,
                            ),
                            color = MaterialTheme.colorScheme.onSurface,
                        )
                        Text(
                            text = if (blockedCount > 0) "$blockedCount tracking requests blocked" else "Shielding ads & tracking requests",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun QuickActionChip(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    label: String,
    onClick: () -> Unit,
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier.clickable(onClick = onClick),
    ) {
        Surface(
            shape = CircleShape,
            color = MaterialTheme.colorScheme.surfaceVariant,
            modifier = Modifier.size(44.dp),
        ) {
            Box(contentAlignment = Alignment.Center) {
                Icon(
                    icon,
                    contentDescription = label,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(20.dp),
                )
            }
        }
        Spacer(Modifier.height(6.dp))
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun BrowserBottomBar(
    canGoBack: Boolean,
    canGoForward: Boolean,
    tabCount: Int,
    onBack: () -> Unit,
    onForward: () -> Unit,
    onHome: () -> Unit,
    onNewTab: () -> Unit,
    onDownloads: () -> Unit,
    onTabs: () -> Unit,
    onMenu: () -> Unit,
    accentColor: Color,
    modifier: Modifier = Modifier,
) {
    Surface(
        color = MaterialTheme.colorScheme.surface.copy(alpha = 0.98f),
        tonalElevation = 4.dp,
        border = BorderStroke(0.5.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.4f)),
        modifier = modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Bottom))
                .height(56.dp)
                .padding(horizontal = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceAround,
        ) {
            // 1. Back button
            IconButton(
                onClick = onBack,
                enabled = canGoBack,
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.ArrowBack,
                    contentDescription = "Back",
                    tint = if (canGoBack) MaterialTheme.colorScheme.onSurface
                           else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.3f),
                )
            }

            // 2. Forward button
            IconButton(
                onClick = onForward,
                enabled = canGoForward,
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.ArrowForward,
                    contentDescription = "Forward",
                    tint = if (canGoForward) MaterialTheme.colorScheme.onSurface
                           else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.3f),
                )
            }

            // 3. Home button
            IconButton(onClick = onHome) {
                Icon(
                    Icons.Default.Home,
                    contentDescription = "Home",
                    tint = MaterialTheme.colorScheme.onSurface,
                )
            }

            // 4. Downloads button
            IconButton(onClick = onDownloads) {
                Icon(
                    Icons.Default.Download,
                    contentDescription = "Downloads",
                    tint = MaterialTheme.colorScheme.onSurface,
                )
            }

            // 5. Tabs Counter button (Standard browser tab badge)
            Surface(
                onClick = onTabs,
                shape = RoundedCornerShape(8.dp),
                border = BorderStroke(2.dp, MaterialTheme.colorScheme.onSurface.copy(alpha = 0.85f)),
                color = Color.Transparent,
                modifier = Modifier.size(28.dp),
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Text(
                        text = if (tabCount > 99) ":D" else tabCount.toString(),
                        style = MaterialTheme.typography.labelMedium.copy(
                            fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
                        ),
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                }
            }

            // 6. Menu (More options)
            IconButton(onClick = onMenu) {
                Icon(
                    Icons.Default.MoreVert,
                    contentDescription = "Browser menu",
                    tint = MaterialTheme.colorScheme.onSurface,
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun BrowserMenuSheet(
    activeTab: TabState?,
    uiState: BrowserUiState,
    spaceAccentColor: Color,
    onDismiss: () -> Unit,
    onNavigateHome: () -> Unit,
    onGoBack: () -> Unit,
    onGoForward: () -> Unit,
    onReload: () -> Unit,
    onToggleBookmark: () -> Unit,
    onDownloadMedia: () -> Unit,
    onNewTab: () -> Unit,
    onOpenTabs: () -> Unit,
    onToggleReaderMode: () -> Unit,
    onOpenTrackerDetails: () -> Unit,
    onOpenPrivacySettings: () -> Unit,
    onOpenBookmarks: () -> Unit,
    onOpenHistory: () -> Unit,
    onOpenDownloads: () -> Unit,
    onOpenExtensions: () -> Unit,
    onOpenSpaces: () -> Unit,
    onOpenThemePicker: () -> Unit,
    onToggleCompactMode: () -> Unit,
    onOpenSettings: () -> Unit,
    onOpenStorage: () -> Unit,
    onOpenUpdates: () -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        dragHandle = null,
        shape = RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(bottom = 32.dp),
        ) {
            // Header
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 20.dp, vertical = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Surface(
                        shape = CircleShape,
                        color = spaceAccentColor,
                        modifier = Modifier.size(24.dp),
                    ) {}
                    Spacer(Modifier.width(10.dp))
                    Text(
                        "Browser Menu",
                        style = MaterialTheme.typography.titleMedium.copy(
                            fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
                        ),
                    )
                }
                IconButton(onClick = onDismiss, modifier = Modifier.size(28.dp)) {
                    Icon(Icons.Default.Close, contentDescription = "Close menu", modifier = Modifier.size(18.dp))
                }
            }

            // Quick Actions Bar (Back, Forward, Reload, Bookmark, Download)
            Surface(
                color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.6f),
                shape = RoundedCornerShape(16.dp),
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp),
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(vertical = 6.dp),
                    horizontalArrangement = Arrangement.SpaceEvenly,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconButton(
                        onClick = onGoBack,
                        enabled = activeTab?.canGoBack == true,
                    ) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back",
                            tint = if (activeTab?.canGoBack == true) MaterialTheme.colorScheme.onSurface
                                   else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.3f),
                        )
                    }
                    IconButton(
                        onClick = onGoForward,
                        enabled = activeTab?.canGoForward == true,
                    ) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowForward,
                            contentDescription = "Forward",
                            tint = if (activeTab?.canGoForward == true) MaterialTheme.colorScheme.onSurface
                                   else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.3f),
                        )
                    }
                    IconButton(onClick = onReload) {
                        Icon(Icons.Default.Refresh, contentDescription = "Reload")
                    }
                    val isBookmarked = uiState.isCurrentPageBookmarked
                    val canBookmark = activeTab?.let { tab ->
                        (tab.url.startsWith("http://") || tab.url.startsWith("https://")) && !tab.isLoading
                    } == true
                    IconButton(
                        onClick = onToggleBookmark,
                        enabled = canBookmark,
                    ) {
                        Icon(
                            if (isBookmarked) Icons.Default.Star else Icons.Default.StarOutline,
                            contentDescription = if (isBookmarked) "Remove bookmark" else "Bookmark page",
                            tint = if (isBookmarked) spaceAccentColor
                                   else MaterialTheme.colorScheme.onSurface.copy(alpha = if (canBookmark) 1f else 0.3f),
                        )
                    }
                    val canDownload = activeTab?.url?.startsWith("http://") == true ||
                        activeTab?.url?.startsWith("https://") == true
                    IconButton(
                        onClick = onDownloadMedia,
                        enabled = canDownload,
                    ) {
                        Icon(
                            Icons.Default.Download,
                            contentDescription = "Download media",
                            tint = if (canDownload) MaterialTheme.colorScheme.primary
                                   else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.3f),
                        )
                    }
                }
            }

            Spacer(Modifier.height(12.dp))

            // Navigation Section
            BrowserMenuItemRow(
                icon = Icons.Default.Add,
                title = "New Tab",
                onClick = onNewTab,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.GridView,
                title = "Tabs Overview",
                badgeText = "${uiState.tabs.size} open",
                onClick = onOpenTabs,
            )
            if (uiState.readerModeAvailable || uiState.readerModeActive) {
                BrowserMenuItemRow(
                    icon = Icons.Default.AutoStories,
                    title = if (uiState.readerModeActive) "Turn Off Reader Mode" else "Reader Mode",
                    badgeText = if (uiState.readerModeActive) "Active" else null,
                    badgeColor = MaterialTheme.colorScheme.primaryContainer,
                    onClick = onToggleReaderMode,
                )
            }

            HorizontalDivider(modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp))

            // Privacy & Protection
            BrowserMenuItemRow(
                icon = Icons.Default.Shield,
                title = "Tracking Protection",
                badgeText = "${uiState.activeTabBlockedRequests} blocked",
                badgeColor = Color(0xFF34A853).copy(alpha = 0.2f),
                onClick = onOpenTrackerDetails,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Security,
                title = "Privacy & Security",
                onClick = onOpenPrivacySettings,
            )

            HorizontalDivider(modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp))

            // Library
            BrowserMenuItemRow(
                icon = Icons.Default.Bookmarks,
                title = "Bookmarks",
                badgeText = if (uiState.bookmarks.isNotEmpty()) "${uiState.bookmarks.size}" else null,
                onClick = onOpenBookmarks,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.History,
                title = "History",
                onClick = onOpenHistory,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Download,
                title = "Downloads",
                onClick = onOpenDownloads,
            )

            HorizontalDivider(modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp))

            // Customization & Settings
            BrowserMenuItemRow(
                icon = Icons.Default.Extension,
                title = "Extensions",
                badgeText = if (uiState.extensionManagerState.extensions.isNotEmpty())
                    "${uiState.extensionManagerState.extensions.size}" else null,
                onClick = onOpenExtensions,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Workspaces,
                title = "Spaces & Profiles",
                subtitle = uiState.selectedProfile?.name ?: "Default",
                onClick = onOpenSpaces,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Palette,
                title = "Theme & Appearance",
                onClick = onOpenThemePicker,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Fullscreen,
                title = "Compact Mode",
                onClick = onToggleCompactMode,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Settings,
                title = "Browser Settings",
                onClick = onOpenSettings,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Storage,
                title = "Storage Usage",
                onClick = onOpenStorage,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Update,
                title = "Check for Updates",
                onClick = onOpenUpdates,
            )
        }
    }
}

@Composable
private fun BrowserMenuItemRow(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    title: String,
    subtitle: String? = null,
    badgeText: String? = null,
    badgeColor: Color? = null,
    enabled: Boolean = true,
    onClick: () -> Unit,
) {
    Surface(
        onClick = onClick,
        enabled = enabled,
        color = Color.Transparent,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 20.dp, vertical = 11.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(
                shape = CircleShape,
                color = MaterialTheme.colorScheme.surfaceVariant,
                modifier = Modifier.size(36.dp),
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(
                        icon,
                        contentDescription = null,
                        modifier = Modifier.size(18.dp),
                        tint = if (enabled) MaterialTheme.colorScheme.onSurfaceVariant
                               else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.4f),
                    )
                }
            }
            Spacer(Modifier.width(14.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.bodyMedium.copy(
                        fontWeight = androidx.compose.ui.text.font.FontWeight.Medium,
                    ),
                    color = if (enabled) MaterialTheme.colorScheme.onSurface
                           else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.4f),
                )
                if (!subtitle.isNullOrBlank()) {
                    Text(
                        text = subtitle,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            if (!badgeText.isNullOrBlank()) {
                Surface(
                    shape = RoundedCornerShape(12.dp),
                    color = badgeColor ?: MaterialTheme.colorScheme.secondaryContainer,
                ) {
                    Text(
                        text = badgeText,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSecondaryContainer,
                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 3.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun SpaceSwitcherDialog(
    profiles: List<BrowserProfile>,
    selectedProfileId: String,
    onSelect: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Spaces") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                profiles.forEach { profile ->
                    val isSelected = profile.id == selectedProfileId
                    val accentColor = try {
                        Color(android.graphics.Color.parseColor(profile.accent))
                    } catch (_: Exception) { MaterialTheme.colorScheme.primary }
                    Surface(
                        onClick = { onSelect(profile.id) },
                        shape = MaterialTheme.shapes.medium,
                        color = if (isSelected)
                            accentColor.copy(alpha = 0.15f)
                        else MaterialTheme.colorScheme.surfaceVariant,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier.padding(12.dp),
                        ) {
                            Surface(
                                shape = CircleShape,
                                color = accentColor,
                                modifier = Modifier.size(24.dp),
                            ) {}
                            Column(
                                modifier = Modifier.weight(1f).padding(start = 12.dp),
                            ) {
                                Text(profile.name, style = MaterialTheme.typography.bodyMedium)
                                Text(
                                    profile.description,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            if (isSelected) {
                                Icon(
                                    Icons.Default.Star, null,
                                    tint = accentColor,
                                    modifier = Modifier.size(18.dp),
                                )
                            }
                        }
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text("Close") } },
    )
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ThemePickerDialog(
    currentSeed: Long,
    onApply: (Long) -> Unit,
    onDismiss: () -> Unit,
) {
    val presets = listOf(
        "Ocean" to 0xFF4F8EF7L,
        "Violet" to 0xFF7C6AF2L,
        "Emerald" to 0xFF3AAE7AL,
        "Sunset" to 0xFFFF6B35L,
        "Rose" to 0xFFE91E8CL,
        "Teal" to 0xFF00BCD4L,
        "Amber" to 0xFFFFC107L,
        "Indigo" to 0xFF3F51B5L,
        "Crimson" to 0xFFDC143CL,
        "Sage" to 0xFF6B8E5AL,
    )
    var selectedSeed by remember { mutableLongStateOf(currentSeed) }
    var hue by remember {
        val hsv = FloatArray(3)
        android.graphics.Color.colorToHSV(selectedSeed.toInt(), hsv)
        mutableStateOf(hsv[0])
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Theme Color") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                // Preview swatch
                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Surface(
                        shape = CircleShape,
                        color = Color(selectedSeed),
                        modifier = Modifier.size(48.dp),
                    ) {}
                    Column {
                        Text("Primary color", style = MaterialTheme.typography.labelMedium)
                        Text(
                            "#%06X".format(selectedSeed.and(0xFFFFFFL).toInt()),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                // Hue slider
                Text("Hue", style = MaterialTheme.typography.labelMedium)
                Slider(
                    value = hue,
                    onValueChange = { newHue ->
                        hue = newHue
                        val hsv = floatArrayOf(newHue, 0.7f, 0.85f)
                        val argb = android.graphics.Color.HSVToColor(hsv)
                        selectedSeed = (0xFF000000L or (argb.toLong() and 0xFFFFFFL))
                    },
                    valueRange = 0f..360f,
                    modifier = Modifier.fillMaxWidth(),
                )
                // Preset dots
                Text("Presets", style = MaterialTheme.typography.labelMedium)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    presets.forEach { (name, seed) ->
                        val isActive = selectedSeed == seed
                        Surface(
                            onClick = {
                                selectedSeed = seed
                                val hsv = FloatArray(3)
                                android.graphics.Color.colorToHSV(seed.toInt(), hsv)
                                hue = hsv[0]
                            },
                            shape = CircleShape,
                            color = Color(seed),
                            border = if (isActive)
                                androidx.compose.foundation.BorderStroke(2.dp, MaterialTheme.colorScheme.onBackground)
                            else null,
                            modifier = Modifier.size(36.dp).padding(2.dp),
                        ) {}
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onApply(selectedSeed) }) { Text("Apply") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SearchEngineSelectorDialog(
    current: SearchEngine,
    onSelect: (SearchEngine) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Search Engine") },
        text = {
            LazyColumn(modifier = Modifier.fillMaxWidth()) {
                items(SearchEngine.entries) { engine ->
                    val isSelected = engine == current
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { onSelect(engine) }
                            .padding(vertical = 10.dp, horizontal = 4.dp),
                    ) {
                        Surface(
                            shape = MaterialTheme.shapes.small,
                            color = if (isSelected)
                                MaterialTheme.colorScheme.primaryContainer
                            else
                                MaterialTheme.colorScheme.surfaceVariant,
                            modifier = Modifier.size(36.dp),
                        ) {
                            Box(contentAlignment = Alignment.Center) {
                                Text(
                                    text = engine.displayName.take(2),
                                    style = MaterialTheme.typography.labelMedium,
                                    color = if (isSelected)
                                        MaterialTheme.colorScheme.onPrimaryContainer
                                    else
                                        MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                        Column(
                            modifier = Modifier
                                .weight(1f)
                                .padding(start = 12.dp),
                        ) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    engine.displayName,
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                                if (engine.isPrivate) {
                                    Surface(
                                        color = MaterialTheme.colorScheme.tertiaryContainer,
                                        shape = MaterialTheme.shapes.extraSmall,
                                        modifier = Modifier.padding(start = 6.dp),
                                    ) {
                                        Text(
                                            "private",
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MaterialTheme.colorScheme.onTertiaryContainer,
                                            modifier = Modifier.padding(horizontal = 4.dp, vertical = 1.dp),
                                        )
                                    }
                                }
                            }
                            Text(
                                engine.homepage.removePrefix("https://"),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (isSelected) {
                            Icon(
                                Icons.Default.Star,
                                contentDescription = "Selected",
                                tint = MaterialTheme.colorScheme.primary,
                                modifier = Modifier.size(20.dp),
                            )
                        }
                    }
                    if (engine != SearchEngine.entries.last()) HorizontalDivider()
                }
            }
        },
        confirmButton = {},
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TabGridSheet(
    tabs: List<TabState>,
    activeTabId: String?,
    profiles: List<BrowserProfile> = emptyList(),
    selectedProfileId: String = "default",
    spaceAccentColor: Color = Color(0xFF4F8EF7),
    onSelect: (String) -> Unit,
    onClose: (String) -> Unit,
    onCloseAll: (String?) -> Unit,
    onNewTab: (String) -> Unit,
    onDismiss: () -> Unit,
    onOpenBrowserMenu: () -> Unit,
) {
    var searchQuery by rememberSaveable { mutableStateOf("") }
    var isGridView by rememberSaveable { mutableStateOf(true) }
    var filterSpaceId by rememberSaveable { mutableStateOf<String?>(null) }
    var showCloseAllConfirm by remember { mutableStateOf(false) }

    val visibleTabs = remember(tabs, searchQuery, filterSpaceId) {
        tabs.filter { tab ->
            val matchesSpace = filterSpaceId == null || tab.spaceId == filterSpaceId
            val matchesQuery = searchQuery.isBlank() ||
                tab.title.orEmpty().contains(searchQuery, ignoreCase = true) ||
                tab.url.contains(searchQuery, ignoreCase = true)
            matchesSpace && matchesQuery
        }
    }

    val backgroundColor = Color(0xFF0C1017)
    val cardBg = Color(0xFF161C26)
    val cardBorder = Color(0xFF263040)
    val previewSurfaceColor = Color(0xFF0E131B)
    val primaryTextColor = Color(0xFFF1F5F9)
    val secondaryTextColor = Color(0xFF94A3B8)

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier.fillMaxSize(),
            color = backgroundColor,
        ) {
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .windowInsetsPadding(WindowInsets.safeDrawing),
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(horizontal = 16.dp),
                ) {
                    // ── Header Bar ──────────────────────────────────────────
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(top = 12.dp, bottom = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            Text(
                                text = "Tabs",
                                style = MaterialTheme.typography.headlineSmall,
                                fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
                                color = primaryTextColor,
                            )
                            Surface(
                                shape = CircleShape,
                                color = spaceAccentColor.copy(alpha = 0.16f),
                            ) {
                                Text(
                                    text = "${tabs.size} open",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = spaceAccentColor,
                                    fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold,
                                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
                                )
                            }
                        }

                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            // Layout toggle (Grid vs List)
                            IconButton(
                                onClick = { isGridView = !isGridView },
                                modifier = Modifier.size(38.dp),
                            ) {
                                Icon(
                                    if (isGridView) Icons.AutoMirrored.Filled.List else Icons.Default.GridView,
                                    contentDescription = if (isGridView) "List view" else "Grid view",
                                    tint = secondaryTextColor,
                                    modifier = Modifier.size(20.dp),
                                )
                            }

                            // Close all tabs button
                            if (visibleTabs.isNotEmpty()) {
                                TextButton(
                                    onClick = { showCloseAllConfirm = true },
                                    colors = androidx.compose.material3.ButtonDefaults.textButtonColors(
                                        contentColor = Color(0xFFF87171),
                                    ),
                                ) {
                                    Text("Close all", style = MaterialTheme.typography.labelMedium)
                                }
                            }

                            // Done button to return
                            IconButton(
                                onClick = onDismiss,
                                modifier = Modifier.size(38.dp),
                            ) {
                                Icon(
                                    Icons.Default.Close,
                                    contentDescription = "Done",
                                    tint = primaryTextColor,
                                    modifier = Modifier.size(22.dp),
                                )
                            }
                        }
                    }

                    // ── Search Tabs Input ───────────────────────────────────
                    OutlinedTextField(
                        value = searchQuery,
                        onValueChange = { searchQuery = it },
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(bottom = 12.dp),
                        singleLine = true,
                        placeholder = { Text("Search your tabs", color = secondaryTextColor) },
                        leadingIcon = {
                            Icon(Icons.Default.Search, contentDescription = null, tint = secondaryTextColor)
                        },
                        trailingIcon = if (searchQuery.isNotEmpty()) {
                            {
                                IconButton(onClick = { searchQuery = "" }) {
                                    Icon(Icons.Default.Close, contentDescription = "Clear search", tint = secondaryTextColor)
                                }
                            }
                        } else null,
                        shape = RoundedCornerShape(16.dp),
                        colors = androidx.compose.material3.OutlinedTextFieldDefaults.colors(
                            focusedContainerColor = Color(0xFF171D27),
                            unfocusedContainerColor = Color(0xFF171D27),
                            focusedBorderColor = spaceAccentColor.copy(alpha = 0.5f),
                            unfocusedBorderColor = cardBorder.copy(alpha = 0.6f),
                            focusedTextColor = primaryTextColor,
                            unfocusedTextColor = primaryTextColor,
                        ),
                    )

                    // ── Space / Profile Filter Chips ────────────────────────
                    if (profiles.isNotEmpty()) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(bottom = 12.dp)
                                .horizontalScroll(rememberScrollState()),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            // "All" filter chip
                            val isAllSelected = filterSpaceId == null
                            Surface(
                                onClick = { filterSpaceId = null },
                                shape = CircleShape,
                                color = if (isAllSelected) spaceAccentColor.copy(alpha = 0.2f) else Color(0xFF171D27),
                                border = BorderStroke(
                                    1.dp,
                                    if (isAllSelected) spaceAccentColor else cardBorder.copy(alpha = 0.6f),
                                ),
                            ) {
                                Text(
                                    text = "All spaces (${tabs.size})",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = if (isAllSelected) spaceAccentColor else secondaryTextColor,
                                    fontWeight = if (isAllSelected) androidx.compose.ui.text.font.FontWeight.Bold else androidx.compose.ui.text.font.FontWeight.Normal,
                                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                                )
                            }

                            // Per-profile filter chips
                            profiles.forEach { profile ->
                                val isSelected = filterSpaceId == profile.id
                                val profileColor = try {
                                    Color(android.graphics.Color.parseColor(profile.accent))
                                } catch (_: Exception) {
                                    spaceAccentColor
                                }
                                val count = tabs.count { it.spaceId == profile.id }
                                Surface(
                                    onClick = { filterSpaceId = profile.id },
                                    shape = CircleShape,
                                    color = if (isSelected) profileColor.copy(alpha = 0.2f) else Color(0xFF171D27),
                                    border = BorderStroke(
                                        1.dp,
                                        if (isSelected) profileColor else cardBorder.copy(alpha = 0.6f),
                                    ),
                                ) {
                                    Row(
                                        verticalAlignment = Alignment.CenterVertically,
                                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                                    ) {
                                        Box(
                                            modifier = Modifier
                                                .size(8.dp)
                                                .background(profileColor, CircleShape),
                                        )
                                        Text(
                                            text = "${profile.name} ($count)",
                                            style = MaterialTheme.typography.labelMedium,
                                            color = if (isSelected) profileColor else secondaryTextColor,
                                            fontWeight = if (isSelected) androidx.compose.ui.text.font.FontWeight.Bold else androidx.compose.ui.text.font.FontWeight.Normal,
                                        )
                                    }
                                }
                            }
                        }
                    }

                    // ── Empty State ─────────────────────────────────────────
                    if (visibleTabs.isEmpty()) {
                        Box(
                            modifier = Modifier
                                .weight(1f)
                                .fillMaxWidth(),
                            contentAlignment = Alignment.Center,
                        ) {
                            Column(
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.spacedBy(14.dp),
                                modifier = Modifier.padding(32.dp),
                            ) {
                                Surface(
                                    shape = CircleShape,
                                    color = spaceAccentColor.copy(alpha = 0.12f),
                                    modifier = Modifier.size(72.dp),
                                ) {
                                    Box(contentAlignment = Alignment.Center) {
                                        Icon(
                                            Icons.Default.Language,
                                            contentDescription = null,
                                            tint = spaceAccentColor,
                                            modifier = Modifier.size(36.dp),
                                        )
                                    }
                                }
                                Text(
                                    text = if (searchQuery.isNotBlank()) "No tabs match your search" else "No open tabs here",
                                    style = MaterialTheme.typography.titleMedium,
                                    fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold,
                                    color = primaryTextColor,
                                )
                                Text(
                                    text = "Open a new tab to explore websites and browse freely",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = secondaryTextColor,
                                )
                                Surface(
                                    onClick = { onNewTab(filterSpaceId ?: selectedProfileId) },
                                    shape = RoundedCornerShape(14.dp),
                                    color = spaceAccentColor,
                                ) {
                                    Row(
                                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                                    ) {
                                        Icon(Icons.Default.Add, contentDescription = null, tint = Color.White, modifier = Modifier.size(18.dp))
                                        Text("Open new tab", color = Color.White, fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold)
                                    }
                                }
                            }
                        }
                    } else {
                        // ── Tab Cards Grid / List ───────────────────────────
                        LazyVerticalGrid(
                            columns = if (isGridView) GridCells.Fixed(2) else GridCells.Fixed(1),
                            modifier = Modifier.weight(1f),
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                            verticalArrangement = Arrangement.spacedBy(12.dp),
                            contentPadding = PaddingValues(bottom = 96.dp),
                        ) {
                            gridItems(visibleTabs, key = { it.id }) { tab ->
                                val isActive = tab.id == activeTabId

                                val dismissState = rememberSwipeToDismissBoxState(
                                    confirmValueChange = { value ->
                                        if (value == SwipeToDismissBoxValue.EndToStart ||
                                            value == SwipeToDismissBoxValue.StartToEnd
                                        ) {
                                            onClose(tab.id)
                                            true
                                        } else {
                                            false
                                        }
                                    },
                                )

                                val tabHost = remember(tab.url) {
                                    try {
                                        java.net.URI(tab.url).host?.removePrefix("www.").orEmpty()
                                    } catch (_: Exception) {
                                        ""
                                    }.ifBlank {
                                        tab.url.takeUnless { it == "about:blank" || it.isBlank() }
                                            ?.removePrefix("https://")
                                            ?.removePrefix("http://")
                                            ?.substringBefore("/")
                                            .orEmpty()
                                            .ifBlank { "New tab" }
                                    }
                                }

                                val tabSpace = profiles.firstOrNull { it.id == tab.spaceId }
                                val tabSpaceColor = tabSpace?.accent?.let {
                                    try { Color(android.graphics.Color.parseColor(it)) } catch (_: Exception) { spaceAccentColor }
                                } ?: spaceAccentColor

                                SwipeToDismissBox(
                                    state = dismissState,
                                    backgroundContent = {
                                        Surface(
                                            color = Color(0xFF7F1D1D),
                                            shape = RoundedCornerShape(18.dp),
                                            modifier = Modifier.fillMaxSize(),
                                        ) {
                                            Box(
                                                contentAlignment = Alignment.CenterEnd,
                                                modifier = Modifier.padding(end = 20.dp),
                                            ) {
                                                Icon(Icons.Default.Close, "Close tab", tint = Color.White)
                                            }
                                        }
                                    },
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .then(
                                            if (isGridView) Modifier.aspectRatio(0.78f)
                                            else Modifier.height(130.dp),
                                        ),
                                ) {
                                    Surface(
                                        onClick = { onSelect(tab.id) },
                                        shape = RoundedCornerShape(18.dp),
                                        color = if (isActive) cardBg else cardBg,
                                        border = BorderStroke(
                                            width = if (isActive) 2.dp else 1.dp,
                                            color = if (isActive) spaceAccentColor else cardBorder.copy(alpha = 0.8f),
                                        ),
                                        shadowElevation = if (isActive) 6.dp else 1.dp,
                                        modifier = Modifier.fillMaxSize(),
                                    ) {
                                        Column(
                                            modifier = Modifier
                                                .fillMaxSize()
                                                .background(
                                                    if (isActive) {
                                                        androidx.compose.ui.graphics.Brush.verticalGradient(
                                                            listOf(
                                                                spaceAccentColor.copy(alpha = 0.16f),
                                                                Color(0xFF131720),
                                                            ),
                                                        )
                                                    } else {
                                                        androidx.compose.ui.graphics.SolidColor(cardBg)
                                                    },
                                                )
                                                .padding(10.dp),
                                        ) {
                                            // Tab Card Header
                                            Row(
                                                verticalAlignment = Alignment.CenterVertically,
                                                modifier = Modifier.padding(bottom = 6.dp),
                                            ) {
                                                Surface(
                                                    shape = CircleShape,
                                                    color = if (isActive) spaceAccentColor else tabSpaceColor.copy(alpha = 0.25f),
                                                    modifier = Modifier.size(24.dp),
                                                ) {
                                                    Box(contentAlignment = Alignment.Center) {
                                                        Text(
                                                            text = tabHost.firstOrNull()?.uppercase() ?: "N",
                                                            style = MaterialTheme.typography.labelSmall,
                                                            fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
                                                            color = if (isActive) Color.White else tabSpaceColor,
                                                        )
                                                    }
                                                }

                                                Text(
                                                    text = tabHost,
                                                    style = MaterialTheme.typography.labelMedium,
                                                    fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold,
                                                    color = primaryTextColor,
                                                    maxLines = 1,
                                                    overflow = TextOverflow.Ellipsis,
                                                    modifier = Modifier
                                                        .weight(1f)
                                                        .padding(horizontal = 6.dp),
                                                )

                                                if (isActive) {
                                                    Surface(
                                                        shape = CircleShape,
                                                        color = spaceAccentColor.copy(alpha = 0.22f),
                                                        modifier = Modifier.padding(end = 4.dp),
                                                    ) {
                                                        Text(
                                                            text = "ACTIVE",
                                                            style = MaterialTheme.typography.labelSmall,
                                                            color = spaceAccentColor,
                                                            fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
                                                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                                                        )
                                                    }
                                                }

                                                IconButton(
                                                    onClick = { onClose(tab.id) },
                                                    modifier = Modifier.size(26.dp),
                                                ) {
                                                    Icon(
                                                        Icons.Default.Close,
                                                        contentDescription = "Close tab",
                                                        tint = secondaryTextColor,
                                                        modifier = Modifier.size(16.dp),
                                                    )
                                                }
                                            }

                                            // Tab Preview Box
                                            Surface(
                                                modifier = Modifier
                                                    .fillMaxWidth()
                                                    .weight(1f),
                                                shape = RoundedCornerShape(12.dp),
                                                color = previewSurfaceColor,
                                                border = BorderStroke(1.dp, Color(0xFF1E2533)),
                                            ) {
                                                Column(
                                                    modifier = Modifier
                                                        .fillMaxSize()
                                                        .padding(10.dp),
                                                    verticalArrangement = Arrangement.SpaceBetween,
                                                ) {
                                                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                                        Text(
                                                            text = tab.title?.takeIf { it.isNotBlank() } ?: tabHost,
                                                            style = MaterialTheme.typography.bodyMedium,
                                                            fontWeight = androidx.compose.ui.text.font.FontWeight.Medium,
                                                            color = primaryTextColor,
                                                            maxLines = if (isGridView) 3 else 1,
                                                            overflow = TextOverflow.Ellipsis,
                                                        )
                                                        Text(
                                                            text = tab.url.takeUnless { it == "about:blank" || it.isBlank() }
                                                                .orEmpty()
                                                                .ifBlank { "New tab page" },
                                                            style = MaterialTheme.typography.bodySmall,
                                                            color = secondaryTextColor,
                                                            maxLines = 1,
                                                            overflow = TextOverflow.Ellipsis,
                                                        )
                                                    }

                                                    Row(
                                                        modifier = Modifier.fillMaxWidth(),
                                                        verticalAlignment = Alignment.CenterVertically,
                                                        horizontalArrangement = Arrangement.SpaceBetween,
                                                    ) {
                                                        if (tab.isLoading) {
                                                            val progress = (tab.progress / 100f).coerceIn(0f, 1f)
                                                            if (progress > 0.05f) {
                                                                LinearProgressIndicator(
                                                                    progress = { progress },
                                                                    modifier = Modifier
                                                                        .width(60.dp)
                                                                        .height(3.dp),
                                                                    color = spaceAccentColor,
                                                                )
                                                            } else {
                                                                LinearProgressIndicator(
                                                                    modifier = Modifier
                                                                        .width(60.dp)
                                                                        .height(3.dp),
                                                                    color = spaceAccentColor,
                                                                )
                                                            }
                                                        } else {
                                                            Spacer(modifier = Modifier.width(1.dp))
                                                        }

                                                        if (tabSpace != null) {
                                                            Row(
                                                                verticalAlignment = Alignment.CenterVertically,
                                                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                                                            ) {
                                                                Box(
                                                                    modifier = Modifier
                                                                        .size(6.dp)
                                                                        .background(tabSpaceColor, CircleShape),
                                                                )
                                                                Text(
                                                                    text = tabSpace.name,
                                                                    style = MaterialTheme.typography.labelSmall,
                                                                    color = secondaryTextColor,
                                                                )
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // ── Floating Action Bar (New Tab & Menu) ───────────────────
                Surface(
                    shape = CircleShape,
                    color = Color(0xFF1E2636),
                    shadowElevation = 10.dp,
                    border = BorderStroke(1.dp, Color(0xFF333E52)),
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(bottom = 20.dp),
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 6.dp, vertical = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Surface(
                            onClick = { onNewTab(filterSpaceId ?: selectedProfileId) },
                            shape = CircleShape,
                            color = spaceAccentColor,
                            contentColor = Color.White,
                            modifier = Modifier.padding(horizontal = 4.dp),
                        ) {
                            Row(
                                modifier = Modifier.padding(horizontal = 20.dp, vertical = 10.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Icon(Icons.Default.Add, contentDescription = null, modifier = Modifier.size(20.dp))
                                Text(
                                    text = "New Tab",
                                    style = MaterialTheme.typography.labelLarge,
                                    fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
                                )
                            }
                        }

                        IconButton(
                            onClick = onOpenBrowserMenu,
                            modifier = Modifier.size(40.dp),
                        ) {
                            Icon(
                                Icons.Default.MoreVert,
                                contentDescription = "Browser menu",
                                tint = primaryTextColor,
                            )
                        }
                    }
                }
            }
        }
    }

    // Close all confirmation dialog
    if (showCloseAllConfirm) {
        AlertDialog(
            onDismissRequest = { showCloseAllConfirm = false },
            title = { Text("Close open tabs?") },
            text = { Text("This will close all ${visibleTabs.size} open tabs.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        onCloseAll(filterSpaceId)
                        showCloseAllConfirm = false
                    },
                    colors = androidx.compose.material3.ButtonDefaults.textButtonColors(
                        contentColor = MaterialTheme.colorScheme.error,
                    ),
                ) {
                    Text("Close All")
                }
            },
            dismissButton = {
                TextButton(onClick = { showCloseAllConfirm = false }) {
                    Text("Cancel")
                }
            },
        )
    }
}


@Composable
private fun ExtensionsDialog(
    state: ExtensionManagerState,
    isImporting: Boolean,
    firefoxAddonsState: FirefoxAddonsCatalogState,
    onDismiss: () -> Unit,
    onRefresh: () -> Unit,
    onInstall: () -> Unit,
    onSearchFirefoxAddons: (String) -> Unit,
    onInstallFirefoxAddon: (FirefoxAddon) -> Unit,
    onEnable: (ExtensionId) -> Unit,
    onDisable: (ExtensionId) -> Unit,
    onRemove: (ExtensionId) -> Unit,
) {
    var pendingRemoval by remember { mutableStateOf<ExtensionId?>(null) }
    var firefoxAddonsQuery by rememberSaveable { mutableStateOf("") }
    val isBusy = state.isBusy || isImporting
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Extensions") },
        text = {
            Column(
                modifier = Modifier
                    .heightIn(max = 440.dp)
                    .verticalScroll(rememberScrollState()),
            ) {
                Text(
                    "Search Mozilla's Firefox Add-ons catalog to install signed extensions, or install an XPI file. FetchX asks before granting requested permissions; private-browsing access is never granted. uBlock Origin is bundled and enabled by default.",
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(bottom = 8.dp),
                )
                Text(
                    "Firefox Add-ons",
                    style = MaterialTheme.typography.titleSmall,
                    modifier = Modifier.padding(top = 4.dp),
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    OutlinedTextField(
                        value = firefoxAddonsQuery,
                        onValueChange = { firefoxAddonsQuery = it },
                        label = { Text("Search add-ons") },
                        singleLine = true,
                        enabled = !isBusy && !firefoxAddonsState.isLoading,
                        modifier = Modifier.weight(1f),
                    )
                    TextButton(
                        onClick = { onSearchFirefoxAddons(firefoxAddonsQuery) },
                        enabled = !isBusy && !firefoxAddonsState.isLoading,
                    ) {
                        Text(if (firefoxAddonsState.isLoading) "Searching..." else "Search")
                    }
                }
                firefoxAddonsState.error?.let { error ->
                    Text(
                        text = error,
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(vertical = 4.dp),
                    )
                }
                if (
                    !firefoxAddonsState.isLoading &&
                    firefoxAddonsState.error == null &&
                    firefoxAddonsState.query.isNotBlank() &&
                    firefoxAddonsState.results.isEmpty()
                ) {
                    Text("No compatible add-ons found.", style = MaterialTheme.typography.bodySmall)
                }
                firefoxAddonsState.results.forEach { addon ->
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Column(modifier = Modifier.weight(1f)) {
                            Text(
                                text = addon.name,
                                style = MaterialTheme.typography.titleSmall,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Text(
                                text = "Version ${addon.version}",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            if (addon.summary.isNotBlank()) {
                                Text(
                                    text = addon.summary,
                                    style = MaterialTheme.typography.bodySmall,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                        }
                        TextButton(
                            onClick = { onInstallFirefoxAddon(addon) },
                            enabled = !isBusy &&
                                !firefoxAddonsState.isLoading &&
                                state.extensions.none { it.id.value == addon.id },
                        ) {
                            Text(if (state.extensions.any { it.id.value == addon.id }) "Installed" else "Install")
                        }
                    }
                    HorizontalDivider()
                }
                when {
                    state.isLoading -> Text("Loading installed extensions\u2026")
                    state.extensions.isEmpty() -> Text("No installed extensions.")
                }
                state.error?.let { error ->
                    Text(
                        text = error,
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(vertical = 8.dp),
                    )
                }
                state.extensions.forEach { extension ->
                    Column(modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
                        Text(
                            text = extension.metadata.name,
                            style = MaterialTheme.typography.titleSmall,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            text = "Version ${extension.metadata.version} \u00b7 ${extension.state.name.lowercase()}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (extension.requestedPermissions.isNotEmpty()) {
                            Text(
                                text = if (extension.id.value == UBLOCK_ORIGIN_EXTENSION_ID) {
                                    "Permissions approved for the bundled official ad blocker."
                                } else {
                                    "${extension.requestedPermissions.size} permission request(s) are not granted by FetchX."
                                },
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        if (extension.id.value == UBLOCK_ORIGIN_EXTENSION_ID) {
                            Text(
                                "Turn ad blocking on or off in Privacy controls.",
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Switch(
                                checked = extension.state == ExtensionState.ENABLED,
                                onCheckedChange = { enabled ->
                                    if (enabled) onEnable(extension.id) else onDisable(extension.id)
                                },
                                enabled = extension.id.value != UBLOCK_ORIGIN_EXTENSION_ID &&
                                    !isBusy && extension.state in setOf(
                                        ExtensionState.ENABLED,
                                        ExtensionState.DISABLED,
                                        ExtensionState.INSTALLED,
                                    ),
                            )
                            Text(if (extension.state == ExtensionState.ENABLED) "Enabled" else "Disabled")
                            if (extension.source == ExtensionSource.BUILT_IN) {
                                Text(
                                    text = " \u00b7 Built in",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            } else {
                                TextButton(
                                    onClick = { pendingRemoval = extension.id },
                                    enabled = !isBusy,
                                ) {
                                    Text("Remove")
                                }
                            }
                        }
                    }
                    HorizontalDivider()
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = onInstall,
                enabled = !isBusy,
            ) {
                Text(
                    when {
                        isImporting -> "Reading\u2026"
                        state.operationState == ExtensionState.INSTALLING -> "Installing\u2026"
                        else -> "Install from file"
                    },
                )
            }
        },
        dismissButton = {
            Row {
                TextButton(onClick = onRefresh, enabled = !isBusy) { Text("Refresh") }
                TextButton(onClick = onDismiss) { Text("Close") }
            }
        },
    )
    pendingRemoval?.let { id ->
        val extension = state.extensions.find { it.id == id }
        if (extension != null) {
            AlertDialog(
                onDismissRequest = { pendingRemoval = null },
                title = { Text("Remove ${extension.metadata.name}?") },
                text = {
                    Text("GeckoView will uninstall this extension and delete its extension data.")
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            pendingRemoval = null
                            onRemove(id)
                        },
                        enabled = !isBusy,
                    ) {
                        Text("Remove")
                    }
                },
                dismissButton = {
                    TextButton(onClick = { pendingRemoval = null }) { Text("Cancel") }
                },
            )
        } else {
            pendingRemoval = null
        }
    }
}

@Composable
private fun ExtensionPermissionDialog(
    request: ExtensionInstallPermissionRequest,
    onResolve: (Boolean) -> Unit,
) {
    AlertDialog(
        onDismissRequest = { onResolve(false) },
        title = { Text("Review extension permissions") },
        text = {
            Column(
                modifier = Modifier
                    .heightIn(max = 440.dp)
                    .verticalScroll(rememberScrollState()),
            ) {
                Text("${request.name} (${request.extensionId}) wants access to:")
                (request.permissions.map { "Permission: $it" } +
                    request.origins.map { "Site access: $it" } +
                    request.dataCollectionPermissions.map { "Data collection: $it" }).forEach { item ->
                    Text("- $item", style = MaterialTheme.typography.bodySmall)
                }
                Text(
                    "Approving grants these requested permissions and allows the listed data collection. Access to private browsing remains off.",
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(top = 8.dp),
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { onResolve(true) }) { Text("Approve & install") }
        },
        dismissButton = {
            TextButton(onClick = { onResolve(false) }) { Text("Cancel") }
        },
    )
}

@Composable
private fun ProfileSwitcherDialog(
    profiles: List<BrowserProfile>,
    selectedProfileId: String,
    onSelect: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Profiles") },
        text = {
            Column(
                modifier = Modifier
                    .heightIn(max = 280.dp)
                    .verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                profiles.forEach { profile ->
                    val isSelected = profile.id == selectedProfileId
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { onSelect(profile.id) }
                            .padding(vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Surface(
                            color = MaterialTheme.colorScheme.primaryContainer,
                            shape = MaterialTheme.shapes.small,
                            modifier = Modifier.size(32.dp),
                        ) {
                            Box(contentAlignment = Alignment.Center) {
                                Text(
                                    text = profile.name.firstOrNull()?.uppercase() ?: "P",
                                    style = MaterialTheme.typography.labelLarge,
                                )
                            }
                        }
                        Column(
                            modifier = Modifier
                                .weight(1f)
                                .padding(start = 12.dp),
                        ) {
                            Text(profile.name)
                            Text(
                                text = profile.description,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (isSelected) {
                            Text(
                                text = "Active",
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.primary,
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
private fun BrowserSettingsDialog(
    searchSuggestionsEnabled: Boolean,
    onSearchSuggestionsChanged: (Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    var compactTabs by remember { mutableStateOf(false) }
    var openLinksInBackground by remember { mutableStateOf(true) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Browser settings") },
        text = {
            Column(
                modifier = Modifier
                    .heightIn(max = 360.dp)
                    .verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    text = "General",
                    style = MaterialTheme.typography.titleSmall,
                )
                SettingToggleRow(
                    title = "Search suggestions",
                    checked = searchSuggestionsEnabled,
                    onCheckedChange = onSearchSuggestionsChanged,
                )
                SettingToggleRow(
                    title = "Compact tab strip",
                    checked = compactTabs,
                    onCheckedChange = { compactTabs = it },
                )
                SettingToggleRow(
                    title = "Open links in background",
                    checked = openLinksInBackground,
                    onCheckedChange = { openLinksInBackground = it },
                )
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Done") }
        },
    )
}

@Composable
private fun BrowserUpdatesDialog(
    versionName: String,
    onDismiss: () -> Unit,
    onCheckForUpdates: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Browser updates") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("FetchX version $versionName")
                Text(
                    "yt-dlp, FFmpeg, and aria2 are delivered with FetchX. " +
                        "Engine bundle updates are installed as browser updates.",
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = onCheckForUpdates) { Text("Check for updates") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Close") }
        },
    )
}

@Composable
private fun SettingToggleRow(
    title: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = title,
            modifier = Modifier.weight(1f),
        )
        Switch(checked = checked, onCheckedChange = onCheckedChange)
    }
}

@Composable
private fun PrivacySettingsDialog(
    settings: PrivacySettings,
    onAdBlockingChange: (Boolean) -> Unit,
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
                    title = "Block ads",
                    description = "Block known ad requests using GeckoView's built-in protection list. Some ads may still appear.",
                    checked = settings.adBlockingEnabled,
                    onCheckedChange = onAdBlockingChange,
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
    var bookmarkQuery by remember { mutableStateOf("") }
    val filteredBookmarks = bookmarks.filter {
        bookmarkQuery.isBlank() || it.title.contains(bookmarkQuery, ignoreCase = true) ||
            it.url.contains(bookmarkQuery, ignoreCase = true)
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Bookmarks") },
        text = {
            Column {
                OutlinedTextField(
                    value = bookmarkQuery,
                    onValueChange = { bookmarkQuery = it },
                    singleLine = true,
                    placeholder = { Text("Search bookmarks") },
                    modifier = Modifier.fillMaxWidth(),
                )
                if (filteredBookmarks.isEmpty()) {
                    Text(
                        text = "No bookmarks yet.",
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.padding(top = 12.dp),
                    )
                } else {
                    LazyColumn(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(max = 320.dp),
                    ) {
                        items(filteredBookmarks, key = { it.id }) { bookmark ->
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

@Composable
private fun VideoDownloadBanner(
    video: DetectedVideo,
    extraCount: Int,
    onDownload: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        tonalElevation = 6.dp,
        shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp),
        color = MaterialTheme.colorScheme.secondaryContainer,
        modifier = modifier.fillMaxWidth(),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
        ) {
            Icon(
                Icons.Default.Language,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSecondaryContainer,
                modifier = Modifier.size(20.dp),
            )
            Spacer(Modifier.width(8.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    "Video detected",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                )
                Text(
                    video.suggestedFilename,
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                )
                if (extraCount > 0) {
                    Text(
                        "+$extraCount more",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSecondaryContainer.copy(alpha = 0.7f),
                    )
                }
            }
            Spacer(Modifier.width(8.dp))
            TextButton(onClick = onDownload) {
                Text("Download", style = MaterialTheme.typography.labelMedium)
            }
            IconButton(onClick = onDismiss, modifier = Modifier.size(32.dp)) {
                Icon(Icons.Default.Close, "Dismiss", modifier = Modifier.size(16.dp))
            }
        }
    }
}

private fun formatHistoryTimestamp(epochMs: Long): String {
    val sdf = SimpleDateFormat("HH:mm", Locale.getDefault())
    return sdf.format(Date(epochMs))
}
