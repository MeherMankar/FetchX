package dev.fetchx.browser

import android.widget.Toast
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
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.material.icons.automirrored.filled.MenuBook
import androidx.compose.material.icons.filled.Eco
import androidx.compose.material.icons.filled.Explore
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
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.DesktopWindows
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Extension
import androidx.compose.material.icons.filled.FindInPage
import androidx.compose.material.icons.filled.Fullscreen
import androidx.compose.material.icons.filled.GridView
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Language
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.NorthWest
import androidx.compose.material.icons.filled.OpenInBrowser
import androidx.compose.material.icons.filled.OndemandVideo
import androidx.compose.material.icons.filled.Palette
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.Share
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
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.DropdownMenu
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.rememberModalBottomSheetState
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
import androidx.compose.runtime.rememberCoroutineScope
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
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.zIndex
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadState
import androidx.lifecycle.viewmodel.compose.viewModel as composeViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.mozilla.geckoview.GeckoSession
import org.mozilla.geckoview.GeckoView
import java.util.Date

/**
 * Full-screen browser powered by GeckoView, with multi-tab support and history.
 *
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun BrowserScreen(
    themeSeedColor: Long = 0xFF4F8EF7L,
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
    onInterceptBrowserDownload: suspend (url: String, filename: String?) -> Unit = { _, _ -> },
    ongoingDownloadCount: Int = 0,
    downloads: List<DownloadItem> = emptyList(),
    onPlayVideo: (String) -> Unit = {},
    browserVersionName: String = "unknown",
    initialNavigateUrl: String? = null,
    viewModel: BrowserViewModel = composeViewModel(factory = BrowserViewModel.Factory),
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()
    val activeTab = uiState.activeTab
    val extensionPicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri -> uri?.let(viewModel::installExtension) }
    val androidPermissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { results ->
        viewModel.resolveNextAndroidRuntimePermissionRequest(results)
    }
    val androidPermissionRequest = uiState.androidRuntimePermissionRequests.firstOrNull()
    LaunchedEffect(androidPermissionRequest?.id) {
        androidPermissionRequest?.let { request ->
            androidPermissionLauncher.launch(request.permissions.toTypedArray())
        }
    }
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
    BackHandler(enabled = uiState.findInPage != null) {
        keyboardController?.hide()
        viewModel.closeFindInPage()
    }

    val findFocusRequester = remember { FocusRequester() }
    LaunchedEffect(uiState.findInPage?.tabId) {
        if (uiState.findInPage != null) {
            delay(100)
            findFocusRequester.requestFocus()
            keyboardController?.show()
        }
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
                    if (!isUrlBarExpanded && uiState.tabs.size > 1) {
                        BrowserTabStrip(
                            tabs = uiState.tabs,
                            activeTabId = uiState.activeTabId,
                            spaceAccentColor = spaceAccentColor,
                            onSelectTab = { viewModel.selectTab(it) },
                            onCloseTab = { viewModel.closeTab(it) },
                            onNewTab = { viewModel.newTabInCurrentSpace() },
                        )
                    }

                Surface(
                    color = MaterialTheme.colorScheme.surface,
                    border = BorderStroke(0.5.dp, MaterialTheme.colorScheme.outlineVariant),
                    modifier = Modifier.windowInsetsPadding(WindowInsets.statusBars),
                    tonalElevation = 3.dp,
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
                                        shape = RoundedCornerShape(10.dp),
                                        color = MaterialTheme.colorScheme.surfaceVariant,
                                        border = BorderStroke(0.5.dp, MaterialTheme.colorScheme.outlineVariant),
                                        modifier = Modifier.padding(start = 6.dp),
                                    ) {
                                        Row(
                                            verticalAlignment = Alignment.CenterVertically,
                                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 4.dp),
                                        ) {
                                            SearchEngineIcon(
                                                engine = uiState.selectedSearchEngine,
                                                size = 20.dp,
                                            )
                                            Spacer(Modifier.width(2.dp))
                                            Icon(
                                                Icons.Default.ArrowDropDown,
                                                contentDescription = "Select engine",
                                                modifier = Modifier.size(16.dp),
                                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
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
                        val isInternalPage = url.startsWith("about:")
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
                                    when {
                                        isBlank -> viewModel.openSearchEngineSelector()
                                        isInternalPage -> Unit
                                        else -> viewModel.openTrackerDetails()
                                    }
                                },
                                modifier = Modifier.size(36.dp),
                            ) {
                                if (isBlank) {
                                    SearchEngineIcon(
                                        engine = uiState.selectedSearchEngine,
                                        size = 20.dp,
                                    )
                                } else if (isInternalPage) {
                                    Icon(
                                        Icons.Default.Info,
                                        contentDescription = "Internal page",
                                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                        modifier = Modifier.size(18.dp),
                                    )
                                } else if (isHttps) {
                                    Icon(
                                        Icons.Default.Lock,
                                        contentDescription = "Secure HTTPS",
                                        tint = MaterialTheme.colorScheme.tertiary,
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

                            // Address bar capsule (Motrix/Arc-inspired clean pill)
                            Surface(
                                onClick = {
                                    isUrlBarExpanded = true
                                    viewModel.onUrlInputChanged(
                                        url.takeUnless { it == "about:blank" }.orEmpty(),
                                    )
                                },
                                shape = RoundedCornerShape(22.dp),
                                color = MaterialTheme.colorScheme.surfaceVariant,
                                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
                                modifier = Modifier
                                    .weight(1f)
                                    .height(42.dp),
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
                                onClick = {
                                    if (isLoading) viewModel.stopLoading() else viewModel.reload()
                                },
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
                    tabCount = uiState.tabs.size,
                    ongoingDownloadCount = ongoingDownloadCount,
                    onBack = { viewModel.goBack() },
                    onHome = {
                    viewModel.navigate("about:blank")
                    viewModel.syncUrlInput("")
                    },
                    onDownloads = onNavigateToDownloads,
                    onTabs = { viewModel.openTabOverview() },
                    onMenu = openMenu,
                    accentColor = spaceAccentColor,
                )
            }
        },
    ) { innerPadding ->
        Box(modifier = Modifier.padding(innerPadding).fillMaxSize()) {
            // Observe the active tab's CloudflareBypassController StateFlow and push to ViewModel
            LaunchedEffect(uiState.activeTabId) {
                val controller = viewModel.activeCfController() ?: return@LaunchedEffect
                controller.state.collect { state ->
                    viewModel.onCfBypassStateChanged(state)
                }
            }

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

            // Cloudflare bypass banner — shown above all content while a challenge is active
            val cfState = uiState.cloudflareBypassState
            AnimatedVisibility(
                visible = cfState is CloudflareBypassState.Bypassing || cfState is CloudflareBypassState.Failed,
                enter = slideInVertically { -it },
                exit = slideOutVertically { -it },
                modifier = Modifier.align(Alignment.TopCenter).zIndex(10f),
            ) {
                Surface(
                    color = if (cfState is CloudflareBypassState.Failed)
                        MaterialTheme.colorScheme.errorContainer
                    else
                        MaterialTheme.colorScheme.tertiaryContainer,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        if (cfState is CloudflareBypassState.Bypassing) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(18.dp),
                                strokeWidth = 2.dp,
                                color = MaterialTheme.colorScheme.onTertiaryContainer,
                            )
                        } else {
                            Icon(
                                Icons.Default.Shield,
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.error,
                                modifier = Modifier.size(18.dp),
                            )
                        }
                        Text(
                            text = when (cfState) {
                                is CloudflareBypassState.Bypassing ->
                                    "Bypassing ${cfState.provider.displayName} security check\u2026"
                                is CloudflareBypassState.Failed ->
                                    "${cfState.provider.displayName} bypass failed: ${cfState.reason}"
                                else -> ""
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = if (cfState is CloudflareBypassState.Failed)
                                MaterialTheme.colorScheme.onErrorContainer
                            else
                                MaterialTheme.colorScheme.onTertiaryContainer,
                            modifier = Modifier.weight(1f),
                        )
                        if (cfState is CloudflareBypassState.Failed) {
                            TextButton(onClick = { viewModel.resetCloudflareBypass() }) {
                                Text("Dismiss")
                            }
                        }
                    }
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
                    onFindInPage = {
                        closeMenu()
                        if (!viewModel.openFindInPage()) {
                            Toast.makeText(context, "No active page to search.", Toast.LENGTH_SHORT).show()
                        }
                    },
                    onSharePage = {
                        closeMenu()
                        viewModel.shareCurrentPage()
                    },
                    onCopyUrl = {
                        closeMenu()
                        viewModel.copyCurrentPageUrl()
                    },
                    onOpenExternally = {
                        closeMenu()
                        viewModel.openCurrentPageExternally()
                    },
                    onToggleDesktopSite = {
                        closeMenu()
                        viewModel.toggleDesktopSite()
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
                    onOpenFetchxRemote = {
                        closeMenu()
                        if (!viewModel.clickFetchxRemoteAction()) {
                            Toast.makeText(
                                context,
                                "FetchX Remote extension is not ready",
                                Toast.LENGTH_SHORT,
                            ).show()
                        }
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
            AndroidView(
                factory = { ctx ->
                    GeckoView(ctx).also { viewModel.attachView(it, GeckoRuntimeHolder.getOrCreate(ctx)) }
                },
                onRelease = { view ->
                    view.releaseSession()
                },
                modifier = Modifier.fillMaxSize(),
            )

            uiState.findInPage?.let { findState ->
                FindInPagePanel(
                    state = findState,
                    focusRequester = findFocusRequester,
                    onQueryChange = viewModel::updateFindQuery,
                    onPrevious = viewModel::findPreviousMatch,
                    onNext = viewModel::findNextMatch,
                    onClear = { viewModel.updateFindQuery("") },
                    onClose = {
                        keyboardController?.hide()
                        viewModel.closeFindInPage()
                    },
                    modifier = Modifier
                        .align(Alignment.TopCenter)
                        .padding(horizontal = 8.dp, vertical = 8.dp)
                        .zIndex(2f),
                )
            }

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
                    recentEntries = uiState.essentials,
                    ongoingDownloadCount = ongoingDownloadCount,
                    onOpenTabs = { viewModel.openTabOverview() },
                    onNavigateRecent = { viewModel.navigateToEssential(it) },
                    onOpenBookmarks = { viewModel.openBookmarks() },
                    onOpenHistory = { viewModel.openHistory() },
                    onOpenDownloads = onNavigateToDownloads,
                    modifier = Modifier.fillMaxSize(),
                )
            }

            downloads.firstOrNull { item ->
                item.state is DownloadState.Active ||
                    item.state is DownloadState.Queued ||
                    item.state is DownloadState.MetadataAcquiring ||
                    item.state is DownloadState.Recovering ||
                    item.state is DownloadState.Seeding
            }?.let { item ->
                BrowserDownloadStatus(
                    item = item,
                    onClick = onNavigateToDownloads,
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(
                            horizontal = 16.dp,
                            vertical = 12.dp,
                        )
                        .padding(
                            bottom = if (uiState.showVideoDownloadBar && uiState.detectedVideos.isNotEmpty()) {
                                88.dp
                            } else {
                                0.dp
                            },
                        ),
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
                                            if (isDirectMediaUrl(video.url)) {
                                                viewModel.downloadDetectedVideo(video)
                                            } else {
                                                viewModel.requestMediaFormats(video)
                                            }
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
                                            if (isDirectMediaUrl(video.url)) {
                                                viewModel.downloadDetectedVideo(video)
                                            } else {
                                                viewModel.requestMediaFormats(video)
                                            }
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

    uiState.pendingDownloads.firstOrNull()?.let { download ->
        ModalBottomSheet(
            onDismissRequest = viewModel::clearPendingDownload,
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 24.dp)
                    .padding(bottom = 24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("Download with FetchX?", style = MaterialTheme.typography.titleLarge)
                Text(
                    text = download.filename?.takeIf(String::isNotBlank) ?: "Unknown filename",
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                browserDownloadMetadataLabel(
                    filename = download.filename,
                    mimeType = download.mimeType,
                    contentLength = download.contentLength,
                )?.let { metadata ->
                    Text(
                        text = metadata,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Surface(
                    shape = RoundedCornerShape(12.dp),
                    color = MaterialTheme.colorScheme.surfaceContainerHighest,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Text(
                            "Save to",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            "FetchX app storage",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
                Text(
                    "If you cancel, the browser cannot continue this response.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.End,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    TextButton(onClick = viewModel::clearPendingDownload) {
                        Text("Cancel")
                    }
                    TextButton(
                        onClick = {
                            viewModel.clearPendingDownload()
                            coroutineScope.launch {
                                try {
                                    onInterceptBrowserDownload(download.url, download.filename)
                                    val result = magnetSnackbarHostState.showSnackbar(
                                        message = "Download started",
                                        actionLabel = "Downloads",
                                        duration = SnackbarDuration.Short,
                                    )
                                    if (result == SnackbarResult.ActionPerformed) {
                                        onNavigateToDownloads()
                                    }
                                } catch (cancelled: CancellationException) {
                                    throw cancelled
                                } catch (error: Exception) {
                                    magnetSnackbarHostState.showSnackbar(
                                        message = error.message ?: "Could not start download",
                                        duration = SnackbarDuration.Long,
                                    )
                                }
                            }
                        },
                    ) {
                        Text("Download")
                    }
                }
            }
        }
    }

    // â”€â”€ Dialogs â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    if (uiState.showHistory) {
        BrowserHistoryScreen(
            entries = uiState.historyEntries,
            onDismiss = viewModel::closeHistory,
            onOpen = viewModel::navigateToHistoryEntry,
            onDelete = viewModel::removeHistoryEntry,
            onClear = viewModel::clearHistory,
        )
    }

    if (uiState.showBookmarks) {
        BrowserBookmarksScreen(
            bookmarks = uiState.bookmarks,
            onDismiss = viewModel::closeBookmarks,
            onOpen = viewModel::navigateToBookmark,
            onEdit = viewModel::updateBookmark,
            onDelete = viewModel::removeBookmark,
        )
    }

    if (uiState.showTrackerDetails) {
        SitePrivacyDialog(
            url = uiState.activeTab?.url.orEmpty(),
            trackerProtectionEnabled = uiState.trackerBlockingEnabled,
            blockedRequests = uiState.activeTabBlockedRequests,
            blockedHosts = uiState.activeTabBlockedHosts,
            onTrackerProtectionChange = viewModel::setTrackerBlockingEnabled,
            onDismiss = viewModel::closeTrackerDetails,
        )
    }

    uiState.sitePermissionPrompt?.let { prompt ->
        SitePermissionPromptDialog(
            prompt = prompt,
            onResolve = { allowed -> viewModel.resolveSitePermissionPrompt(prompt.id, allowed) },
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
            searchEngine = uiState.selectedSearchEngine,
            searchSuggestionsEnabled = uiState.browserSuggestionsEnabled,
            profileName = uiState.selectedProfile?.name ?: "Default",
            versionName = browserVersionName,
            onSearchSuggestionsChanged = viewModel::setBrowserSuggestionsEnabled,
            onDismiss = viewModel::closeBrowserSettings,
            onOpenSearchEngine = {
                viewModel.closeBrowserSettings()
                viewModel.openSearchEngineSelector()
            },
            onOpenThemePicker = {
                viewModel.closeBrowserSettings()
                viewModel.openThemePicker()
            },
            onOpenPrivacy = {
                viewModel.closeBrowserSettings()
                viewModel.openPrivacySettings()
            },
            onOpenTrackingProtection = {
                viewModel.closeBrowserSettings()
                viewModel.openTrackerDetails()
            },
            onOpenHistory = {
                viewModel.closeBrowserSettings()
                viewModel.openHistory()
            },
            onOpenTabs = {
                viewModel.closeBrowserSettings()
                viewModel.openTabOverview()
            },
            onOpenSpaces = {
                viewModel.closeBrowserSettings()
                viewModel.openSpaceSwitcher()
            },
            onOpenDownloads = {
                viewModel.closeBrowserSettings()
                onNavigateToDownloads()
            },
            onOpenUpdates = {
                viewModel.closeBrowserSettings()
                showBrowserUpdates = true
            },
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
            currentSeed = themeSeedColor,
            onApply = { seed ->
                viewModel.closeThemePicker()
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

    uiState.fetchxRemotePopupSession?.let { session ->
        FetchxRemotePopupDialog(
            session = session,
            onDismiss = viewModel::closeFetchxRemotePopup,
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

@Composable
private fun FetchxRemotePopupDialog(
    session: GeckoSession,
    onDismiss: () -> Unit,
) {
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier
                .fillMaxWidth(0.94f)
                .height(440.dp),
            shape = RoundedCornerShape(20.dp),
            tonalElevation = 8.dp,
        ) {
            Column {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(48.dp)
                        .padding(start = 16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "FetchX Remote",
                        style = MaterialTheme.typography.titleMedium,
                        modifier = Modifier.weight(1f),
                    )
                    IconButton(onClick = onDismiss) {
                        Icon(Icons.Default.Close, contentDescription = "Close FetchX Remote")
                    }
                }
                AndroidView(
                    factory = { context -> GeckoView(context).also { it.setSession(session) } },
                    onRelease = { it.releaseSession() },
                    modifier = Modifier.weight(1f).fillMaxWidth(),
                )
            }
        }
    }
}

@Composable
private fun EssentialsPage(
    recentEntries: List<Essential>,
    ongoingDownloadCount: Int,
    onOpenTabs: () -> Unit,
    onNavigateRecent: (String) -> Unit,
    onOpenBookmarks: () -> Unit,
    onOpenHistory: () -> Unit,
    onOpenDownloads: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier = modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 16.dp),
            horizontalAlignment = Alignment.Start,
        ) {
            Spacer(Modifier.height(12.dp))

            Row(
                modifier = Modifier.align(Alignment.CenterHorizontally),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Surface(
                    shape = RoundedCornerShape(10.dp),
                    color = MaterialTheme.colorScheme.primaryContainer,
                    modifier = Modifier.size(32.dp),
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            Icons.Default.Language,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onPrimaryContainer,
                            modifier = Modifier.size(18.dp),
                        )
                    }
                }
                Spacer(Modifier.width(10.dp))
                Text(
                    text = "FetchX",
                    style = MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.Bold),
                    color = MaterialTheme.colorScheme.onBackground,
                )
            }

            Spacer(Modifier.height(8.dp))
            Text(
                text = "Browse • Download • Manage",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.align(Alignment.CenterHorizontally),
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = "Search or enter an address above to get started.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.align(Alignment.CenterHorizontally),
                textAlign = TextAlign.Center,
            )

            Spacer(Modifier.height(28.dp))

            Text(
                text = "Quick access",
                style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Bold),
                color = MaterialTheme.colorScheme.onBackground,
            )
            Spacer(Modifier.height(12.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                EssentialsAction(
                    Icons.Default.Download,
                    "Downloads",
                    onOpenDownloads,
                    Modifier.weight(1f),
                    badgeText = ongoingDownloadCount.takeIf { it > 0 }?.let {
                        if (it > 99) "99+" else it.toString()
                    },
                    contentDescription = if (ongoingDownloadCount > 0) {
                        "Downloads, $ongoingDownloadCount in progress"
                    } else {
                        "Downloads"
                    },
                )
                EssentialsAction(
                    Icons.Default.GridView,
                    "Tabs",
                    onOpenTabs,
                    Modifier.weight(1f),
                )
                EssentialsAction(
                    Icons.Default.Bookmarks,
                    "Bookmarks",
                    onOpenBookmarks,
                    Modifier.weight(1f),
                )
                EssentialsAction(
                    Icons.Default.History,
                    "History",
                    onOpenHistory,
                    Modifier.weight(1f),
                )
            }

            if (recentEntries.isNotEmpty()) {
                Spacer(Modifier.height(28.dp))
                Text(
                    text = "Frequently visited",
                    style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Bold),
                    color = MaterialTheme.colorScheme.onBackground,
                )
                Spacer(Modifier.height(8.dp))
                recentEntries.take(4).forEach { entry ->
                    Surface(
                        onClick = { onNavigateRecent(entry.url) },
                        shape = RoundedCornerShape(12.dp),
                        color = MaterialTheme.colorScheme.surfaceVariant,
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 3.dp),
                    ) {
                        Column(modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
                            Text(
                                text = entry.title,
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Text(
                                text = entry.url,
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun EssentialsAction(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    badgeText: String? = null,
    contentDescription: String = label,
) {
    Surface(
        onClick = onClick,
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.surfaceVariant,
        modifier = modifier.height(76.dp),
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
            modifier = Modifier.fillMaxSize(),
        ) {
            BadgedBox(
                badge = {
                    if (badgeText != null) {
                        Badge { Text(badgeText) }
                    }
                },
            ) {
                Icon(
                    icon,
                    contentDescription = contentDescription,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(20.dp),
                )
            }
            Spacer(Modifier.height(6.dp))
            Text(
                text = label,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
        }
    }
}

@Composable
private fun BrowserDownloadStatus(
    item: DownloadItem,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val state = item.state
    val progress = when (state) {
        is DownloadState.Active -> state.totalBytes
            ?.takeIf { it > 0L }
            ?.let { (state.downloadedBytes.toFloat() / it).coerceIn(0f, 1f) }
        else -> null
    }
    val status = when (state) {
        is DownloadState.Active -> "Downloading"
        is DownloadState.Queued -> "Queued"
        is DownloadState.MetadataAcquiring -> "Preparing torrent"
        is DownloadState.Recovering -> "Recovering download"
        is DownloadState.Seeding -> "Seeding"
        else -> return
    }
    val detail = when (state) {
        is DownloadState.Active -> {
            val total = state.totalBytes
            if (total != null && total > 0L) {
                "${progress?.let { (it * 100).toInt() } ?: 0}% · ${formatDownloadSize(state.downloadedBytes)} / ${formatDownloadSize(total)}"
            } else {
                "${formatDownloadSize(state.downloadedBytes)} downloaded"
            }
        }
        is DownloadState.Seeding -> state.totalBytes?.let { formatDownloadSize(it) }
        else -> null
    }

    Surface(
        onClick = onClick,
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        tonalElevation = 4.dp,
        modifier = modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Default.Download,
                    contentDescription = "Download status",
                    tint = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.size(20.dp),
                )
                Spacer(Modifier.width(12.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = item.fileName,
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = buildString {
                            append(status)
                            detail?.let { append(" · ").append(it) }
                        },
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Text(
                    text = "Downloads",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.primary,
                )
            }
            if (state is DownloadState.Active) {
                if (progress != null) {
                    LinearProgressIndicator(
                        progress = { progress },
                        modifier = Modifier.fillMaxWidth(),
                    )
                } else {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                }
            }
        }
    }
}


@Composable
private fun BrowserBottomBar(
    canGoBack: Boolean,
    tabCount: Int,
    ongoingDownloadCount: Int,
    onBack: () -> Unit,
    onHome: () -> Unit,
    onDownloads: () -> Unit,
    onTabs: () -> Unit,
    onMenu: () -> Unit,
    accentColor: Color,
    modifier: Modifier = Modifier,
) {
    Surface(
        color = MaterialTheme.colorScheme.surface,
        border = BorderStroke(0.5.dp, MaterialTheme.colorScheme.outlineVariant),
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
                modifier = Modifier.size(48.dp),
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.ArrowBack,
                    contentDescription = "Back",
                    tint = if (canGoBack) MaterialTheme.colorScheme.onSurface
                    else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.38f),
                    modifier = Modifier.size(20.dp),
                )
            }

            // 2. Home button
            IconButton(
                onClick = onHome,
                modifier = Modifier.size(48.dp),
            ) {
                Icon(
                    Icons.Default.Home,
                    contentDescription = "Home",
                    tint = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.size(20.dp),
                )
            }

            // 3. Tabs badge
            IconButton(
                onClick = onTabs,
                modifier = Modifier.size(48.dp),
            ) {
                Surface(
                    shape = RoundedCornerShape(6.dp),
                    border = BorderStroke(1.5.dp, MaterialTheme.colorScheme.onSurface),
                    color = Color.Transparent,
                    modifier = Modifier.size(22.dp),
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Text(
                            text = if (tabCount > 99) "∞" else tabCount.toString(),
                            style = MaterialTheme.typography.labelSmall.copy(
                                fontWeight = FontWeight.Bold,
                                fontSize = 11.sp,
                            ),
                            color = MaterialTheme.colorScheme.onSurface,
                        )
                    }
                }
            }

            // 4. Downloads
            IconButton(
                onClick = onDownloads,
                modifier = Modifier.size(48.dp),
            ) {
                BadgedBox(
                    badge = {
                        if (ongoingDownloadCount > 0) {
                            Badge {
                                Text(if (ongoingDownloadCount > 99) "99+" else ongoingDownloadCount.toString())
                            }
                        }
                    },
                ) {
                    Icon(
                        Icons.Default.Download,
                        contentDescription = if (ongoingDownloadCount > 0) {
                            "Downloads, $ongoingDownloadCount in progress"
                        } else {
                            "Downloads"
                        },
                        tint = MaterialTheme.colorScheme.onSurface,
                        modifier = Modifier.size(20.dp),
                    )
                }
            }

            // 5. 3-dot Menu
            IconButton(
                onClick = onMenu,
                modifier = Modifier.size(48.dp),
            ) {
                Icon(
                    Icons.Default.MoreVert,
                    contentDescription = "Menu",
                    tint = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.size(20.dp),
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
    onFindInPage: () -> Unit,
    onSharePage: () -> Unit,
    onCopyUrl: () -> Unit,
    onOpenExternally: () -> Unit,
    onToggleDesktopSite: () -> Unit,
    onOpenTrackerDetails: () -> Unit,
    onOpenPrivacySettings: () -> Unit,
    onOpenBookmarks: () -> Unit,
    onOpenHistory: () -> Unit,
    onOpenDownloads: () -> Unit,
    onOpenFetchxRemote: () -> Unit,
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
        containerColor = MaterialTheme.colorScheme.surface,
        contentColor = MaterialTheme.colorScheme.onSurface,
        scrimColor = MaterialTheme.colorScheme.scrim.copy(alpha = 0.5f),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(bottom = 32.dp),
        ) {
            // ── Header ─────────────────────────────────────────────────────
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 20.dp, vertical = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Surface(
                        shape = RoundedCornerShape(8.dp),
                        color = spaceAccentColor,
                        modifier = Modifier.size(24.dp),
                    ) {}
                    Spacer(Modifier.width(10.dp))
                    Text(
                        "FetchX",
                        style = MaterialTheme.typography.titleMedium.copy(
                            fontWeight = FontWeight.Bold,
                        ),
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                }
                Surface(
                    onClick = onDismiss,
                    shape = CircleShape,
                    color = MaterialTheme.colorScheme.surfaceVariant,
                    modifier = Modifier.size(30.dp),
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(Icons.Default.Close, contentDescription = "Close menu",
                            tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(16.dp))
                    }
                }
            }

            // Quick Actions Grid (labeled icon buttons)
            Surface(
                color = MaterialTheme.colorScheme.surfaceContainer,
                shape = RoundedCornerShape(16.dp),
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp),
            ) {
                val isBookmarked = uiState.isCurrentPageBookmarked
                val canBookmark = activeTab?.let { tab ->
                    (tab.url.startsWith("http://") || tab.url.startsWith("https://")) && !tab.isLoading
                } == true
                val canDownload = activeTab?.url?.startsWith("http://") == true ||
                    activeTab?.url?.startsWith("https://") == true

                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 8.dp, vertical = 10.dp),
                    horizontalArrangement = Arrangement.SpaceEvenly,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    // Back
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.clickable(
                            enabled = activeTab?.canGoBack == true,
                            onClick = onGoBack,
                        ),
                    ) {
                        Surface(
                            shape = RoundedCornerShape(12.dp),
                            color = if (activeTab?.canGoBack == true) MaterialTheme.colorScheme.surfaceVariant
                                   else MaterialTheme.colorScheme.surface,
                            modifier = Modifier.size(42.dp),
                        ) {
                            Box(contentAlignment = Alignment.Center) {
                                Icon(
                                    Icons.AutoMirrored.Filled.ArrowBack,
                                    contentDescription = "Back",
                                    tint = if (activeTab?.canGoBack == true) MaterialTheme.colorScheme.onSurface
                                           else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f),
                                    modifier = Modifier.size(20.dp),
                                )
                            }
                        }
                        Spacer(Modifier.height(4.dp))
                        Text("Back", style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    // Forward
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.clickable(
                            enabled = activeTab?.canGoForward == true,
                            onClick = onGoForward,
                        ),
                    ) {
                        Surface(
                            shape = RoundedCornerShape(12.dp),
                            color = if (activeTab?.canGoForward == true) MaterialTheme.colorScheme.surfaceVariant
                                   else MaterialTheme.colorScheme.surface,
                            modifier = Modifier.size(42.dp),
                        ) {
                            Box(contentAlignment = Alignment.Center) {
                                Icon(
                                    Icons.AutoMirrored.Filled.ArrowForward,
                                    contentDescription = "Forward",
                                    tint = if (activeTab?.canGoForward == true) MaterialTheme.colorScheme.onSurface
                                           else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f),
                                    modifier = Modifier.size(20.dp),
                                )
                            }
                        }
                        Spacer(Modifier.height(4.dp))
                        Text("Forward", style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    // Reload
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.clickable(onClick = onReload),
                    ) {
                        Surface(
                            shape = RoundedCornerShape(12.dp),
                            color = MaterialTheme.colorScheme.surfaceVariant,
                            modifier = Modifier.size(42.dp),
                        ) {
                            Box(contentAlignment = Alignment.Center) {
                                Icon(Icons.Default.Refresh, contentDescription = "Reload",
                                    tint = MaterialTheme.colorScheme.onSurface, modifier = Modifier.size(20.dp))
                            }
                        }
                        Spacer(Modifier.height(4.dp))
                        Text("Reload", style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    // Bookmark
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.clickable(
                            enabled = canBookmark,
                            onClick = onToggleBookmark,
                        ),
                    ) {
                        Surface(
                            shape = RoundedCornerShape(12.dp),
                            color = if (isBookmarked) MaterialTheme.colorScheme.primaryContainer
                                   else if (canBookmark) MaterialTheme.colorScheme.surfaceVariant
                                   else MaterialTheme.colorScheme.surface,
                            border = if (isBookmarked) BorderStroke(1.dp, MaterialTheme.colorScheme.primary) else null,
                            modifier = Modifier.size(42.dp),
                        ) {
                            Box(contentAlignment = Alignment.Center) {
                                Icon(
                                    if (isBookmarked) Icons.Default.Star else Icons.Default.StarOutline,
                                    contentDescription = if (isBookmarked) "Remove bookmark" else "Bookmark",
                                    tint = if (isBookmarked) MaterialTheme.colorScheme.onPrimaryContainer
                                           else if (canBookmark) MaterialTheme.colorScheme.onSurface
                                           else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f),
                                    modifier = Modifier.size(20.dp),
                                )
                            }
                        }
                        Spacer(Modifier.height(4.dp))
                        Text(if (isBookmarked) "Saved" else "Save",
                            style = MaterialTheme.typography.labelSmall,
                            color = if (isBookmarked) MaterialTheme.colorScheme.primary
                                   else MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    // Download
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.clickable(
                            enabled = canDownload,
                            onClick = onDownloadMedia,
                        ),
                    ) {
                        Surface(
                            shape = RoundedCornerShape(12.dp),
                            color = if (canDownload) MaterialTheme.colorScheme.primaryContainer
                                   else MaterialTheme.colorScheme.surface,
                            border = if (canDownload) BorderStroke(1.dp, MaterialTheme.colorScheme.primary) else null,
                            modifier = Modifier.size(42.dp),
                        ) {
                            Box(contentAlignment = Alignment.Center) {
                                Icon(
                                    Icons.Default.Download,
                                    contentDescription = "Download",
                                    tint = if (canDownload) MaterialTheme.colorScheme.onPrimaryContainer
                                           else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f),
                                    modifier = Modifier.size(20.dp),
                                )
                            }
                        }
                        Spacer(Modifier.height(4.dp))
                        Text("Grab", style = MaterialTheme.typography.labelSmall,
                            color = if (canDownload) MaterialTheme.colorScheme.primary
                                   else MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }

            Spacer(Modifier.height(12.dp))

            if (activeTab != null) {
                val canUseWebPageActions = isHttpOrHttpsPageUrl(activeTab.url)
                Text(
                    text = "PAGE",
                    style = MaterialTheme.typography.labelSmall.copy(
                        fontWeight = FontWeight.Bold,
                        letterSpacing = 1.2.sp,
                    ),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 24.dp, vertical = 6.dp),
                )
                BrowserMenuItemRow(
                    icon = Icons.Default.FindInPage,
                    title = "Find in Page",
                    iconTint = MaterialTheme.colorScheme.onSurface,
                    onClick = onFindInPage,
                )
                BrowserMenuItemRow(
                    icon = Icons.Default.Share,
                    title = "Share page",
                    iconTint = MaterialTheme.colorScheme.onSurface,
                    enabled = canUseWebPageActions,
                    onClick = onSharePage,
                )
                BrowserMenuItemRow(
                    icon = Icons.Default.ContentCopy,
                    title = "Copy URL",
                    iconTint = MaterialTheme.colorScheme.onSurface,
                    enabled = canUseWebPageActions,
                    onClick = onCopyUrl,
                )
                BrowserMenuItemRow(
                    icon = Icons.Default.OpenInBrowser,
                    title = "Open in external browser",
                    iconTint = MaterialTheme.colorScheme.onSurface,
                    enabled = canUseWebPageActions,
                    onClick = onOpenExternally,
                )
                BrowserMenuItemRow(
                    icon = Icons.Default.DesktopWindows,
                    title = "Desktop site",
                    iconTint = MaterialTheme.colorScheme.onSurface,
                    badgeText = if (activeTab.desktopSiteEnabled) "On" else "Off",
                    badgeColor = if (activeTab.desktopSiteEnabled) {
                        MaterialTheme.colorScheme.primaryContainer
                    } else {
                        MaterialTheme.colorScheme.surfaceVariant
                    },
                    enabled = canUseWebPageActions,
                    onClick = onToggleDesktopSite,
                )
                Spacer(Modifier.height(8.dp))
            }

            // ── Section: Tabs & Viewing ────────────────────────────────────
            Text(
                text = "TABS & VIEWING",
                style = MaterialTheme.typography.labelSmall.copy(
                    fontWeight = FontWeight.Bold,
                    letterSpacing = 1.2.sp,
                ),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 6.dp),
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Add,
                title = "New Tab",
                iconTint = MaterialTheme.colorScheme.onSurface,
                onClick = onNewTab,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.GridView,
                title = "Tabs Overview",
                iconTint = MaterialTheme.colorScheme.onSurface,
                badgeText = "${uiState.tabs.size} open",
                onClick = onOpenTabs,
            )
            Spacer(Modifier.height(8.dp))

            // ── Section: Privacy & Security ────────────────────────────────
            Text(
                text = "PRIVACY & SECURITY",
                style = MaterialTheme.typography.labelSmall.copy(
                    fontWeight = FontWeight.Bold,
                    letterSpacing = 1.2.sp,
                ),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 6.dp),
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Shield,
                title = "Tracking Protection",
                iconTint = MaterialTheme.colorScheme.onSurface,
                badgeText = "${uiState.activeTabBlockedRequests} blocked",
                badgeColor = MaterialTheme.colorScheme.tertiaryContainer,
                onClick = onOpenTrackerDetails,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Security,
                title = "Privacy & Security",
                iconTint = MaterialTheme.colorScheme.onSurface,
                onClick = onOpenPrivacySettings,
            )

            Spacer(Modifier.height(8.dp))

            // ── Section: Library ───────────────────────────────────────────
            Text(
                text = "LIBRARY",
                style = MaterialTheme.typography.labelSmall.copy(
                    fontWeight = FontWeight.Bold,
                    letterSpacing = 1.2.sp,
                ),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 6.dp),
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Download,
                title = "Downloads",
                iconTint = MaterialTheme.colorScheme.onSurface,
                onClick = onOpenDownloads,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Bookmarks,
                title = "Bookmarks",
                iconTint = MaterialTheme.colorScheme.onSurface,
                badgeText = if (uiState.bookmarks.isNotEmpty()) "${uiState.bookmarks.size}" else null,
                onClick = onOpenBookmarks,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.History,
                title = "History",
                iconTint = MaterialTheme.colorScheme.onSurface,
                onClick = onOpenHistory,
            )

            Spacer(Modifier.height(8.dp))

            // ── Section: Preferences ───────────────────────────────────────
            Text(
                text = "PREFERENCES",
                style = MaterialTheme.typography.labelSmall.copy(
                    fontWeight = FontWeight.Bold,
                    letterSpacing = 1.2.sp,
                ),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 6.dp),
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Extension,
                title = "FetchX Remote",
                subtitle = "Connect or add a download",
                iconTint = MaterialTheme.colorScheme.onSurface,
                onClick = onOpenFetchxRemote,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Extension,
                title = "Extensions",
                iconTint = MaterialTheme.colorScheme.onSurface,
                badgeText = if (uiState.extensionManagerState.extensions.isNotEmpty())
                    "${uiState.extensionManagerState.extensions.size}" else null,
                onClick = onOpenExtensions,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Workspaces,
                title = "Spaces & Profiles",
                subtitle = uiState.selectedProfile?.name ?: "Default",
                iconTint = MaterialTheme.colorScheme.onSurface,
                onClick = onOpenSpaces,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Palette,
                title = "Theme & Appearance",
                iconTint = MaterialTheme.colorScheme.onSurface,
                onClick = onOpenThemePicker,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Fullscreen,
                title = "Compact Mode",
                iconTint = MaterialTheme.colorScheme.onSurfaceVariant,
                onClick = onToggleCompactMode,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Settings,
                title = "Settings",
                iconTint = MaterialTheme.colorScheme.onSurfaceVariant,
                onClick = onOpenSettings,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Storage,
                title = "Storage Usage",
                iconTint = MaterialTheme.colorScheme.onSurfaceVariant,
                onClick = onOpenStorage,
            )
            BrowserMenuItemRow(
                icon = Icons.Default.Update,
                title = "Check for Updates",
                iconTint = MaterialTheme.colorScheme.onSurfaceVariant,
                onClick = onOpenUpdates,
            )
        }
    }
}

@Composable
private fun FindInPagePanel(
    state: FindInPageState,
    focusRequester: FocusRequester,
    onQueryChange: (String) -> Unit,
    onPrevious: () -> Unit,
    onNext: () -> Unit,
    onClear: () -> Unit,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val keyboardController = LocalSoftwareKeyboardController.current
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        contentColor = MaterialTheme.colorScheme.onSurface,
        tonalElevation = 4.dp,
        shadowElevation = 4.dp,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 8.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "Find in Page",
                    style = MaterialTheme.typography.titleSmall,
                    modifier = Modifier.weight(1f),
                )
                IconButton(
                    onClick = onClose,
                    modifier = Modifier.size(48.dp),
                ) {
                    Icon(
                        Icons.Default.Close,
                        contentDescription = "Close Find in Page",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedTextField(
                    value = state.query,
                    onValueChange = onQueryChange,
                    modifier = Modifier
                        .weight(1f)
                        .focusRequester(focusRequester),
                    placeholder = { Text("Search this page") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(
                        onSearch = { keyboardController?.hide() },
                    ),
                    trailingIcon = {
                        if (state.query.isNotEmpty()) {
                            IconButton(
                                onClick = onClear,
                                modifier = Modifier.size(48.dp),
                            ) {
                                Icon(
                                    Icons.Default.Clear,
                                    contentDescription = "Clear search",
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    },
                )
                if (state.query.isNotBlank()) {
                    Text(
                        text = if (state.error != null) "—"
                        else "${if (state.found) state.currentMatch else 0} / ${state.totalMatches}",
                        style = MaterialTheme.typography.labelLarge,
                        color = if (state.error != null) MaterialTheme.colorScheme.error
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.semantics {
                            contentDescription = state.error
                                ?: "${if (state.found) state.currentMatch else 0} of ${state.totalMatches} matches"
                        },
                    )
                }
            }
            state.error?.let { error ->
                Text(
                    text = error,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(top = 4.dp),
                )
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
            ) {
                IconButton(
                    onClick = onPrevious,
                    enabled = state.query.isNotBlank() && state.totalMatches > 0,
                    modifier = Modifier
                        .size(48.dp)
                        .semantics { contentDescription = "Previous match" },
                ) {
                    Icon(
                        Icons.AutoMirrored.Filled.ArrowBack,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                IconButton(
                    onClick = onNext,
                    enabled = state.query.isNotBlank() && state.totalMatches > 0,
                    modifier = Modifier
                        .size(48.dp)
                        .semantics { contentDescription = "Next match" },
                ) {
                    Icon(
                        Icons.AutoMirrored.Filled.ArrowForward,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
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
    iconTint: Color? = null,
    enabled: Boolean = true,
    onClick: () -> Unit,
) {
    val resolvedIconTint = iconTint ?: MaterialTheme.colorScheme.onSurfaceVariant
    Surface(
        onClick = onClick,
        enabled = enabled,
        color = MaterialTheme.colorScheme.surface,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp)
                .padding(horizontal = 20.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(
                shape = RoundedCornerShape(10.dp),
                color = resolvedIconTint.copy(alpha = 0.12f),
                modifier = Modifier.size(36.dp),
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(
                        icon,
                        contentDescription = null,
                        modifier = Modifier.size(18.dp),
                        tint = if (enabled) resolvedIconTint
                               else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f),
                    )
                }
            }
            Spacer(Modifier.width(14.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.bodyMedium.copy(
                        fontWeight = FontWeight.Medium,
                    ),
                    color = if (enabled) MaterialTheme.colorScheme.onSurface
                           else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f),
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
                    shape = RoundedCornerShape(10.dp),
                    color = badgeColor ?: MaterialTheme.colorScheme.surfaceVariant,
                ) {
                    Text(
                        text = badgeText,
                        style = MaterialTheme.typography.labelSmall.copy(
                            fontWeight = FontWeight.Medium,
                        ),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
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
                            modifier = Modifier
                                .size(36.dp)
                                .padding(2.dp)
                                .semantics {
                                    contentDescription = name
                                    selected = isActive
                                },
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


@Composable
fun SearchEngineIcon(
    engine: SearchEngine,
    modifier: Modifier = Modifier,
    size: androidx.compose.ui.unit.Dp = 20.dp,
) {
    Surface(
        shape = RoundedCornerShape(size * 0.28f),
        color = Color(0xFF2B2A33),
        border = BorderStroke(0.5.dp, Color(0xFF383741)),
        modifier = modifier.size(size),
    ) {
        Box(contentAlignment = Alignment.Center) {
            when (engine) {
                SearchEngine.GOOGLE -> {
                    Text(
                        text = "G",
                        style = MaterialTheme.typography.labelMedium.copy(
                            fontWeight = FontWeight.Bold,
                            fontSize = (size.value * 0.55f).sp,
                            lineHeight = (size.value * 0.55f).sp,
                        ),
                        color = Color(0xFFFBFBFE),
                    )
                }
                SearchEngine.BING -> {
                    Text(
                        text = "b",
                        style = MaterialTheme.typography.labelMedium.copy(
                            fontWeight = FontWeight.Bold,
                            fontSize = (size.value * 0.58f).sp,
                            lineHeight = (size.value * 0.58f).sp,
                        ),
                        color = Color(0xFFFBFBFE),
                    )
                }
                SearchEngine.DUCKDUCKGO -> {
                    Text(
                        text = "D",
                        style = MaterialTheme.typography.labelMedium.copy(
                            fontWeight = FontWeight.Bold,
                            fontSize = (size.value * 0.55f).sp,
                            lineHeight = (size.value * 0.55f).sp,
                        ),
                        color = Color(0xFFFBFBFE),
                    )
                }
                SearchEngine.BRAVE -> {
                    Icon(
                        Icons.Default.Shield,
                        contentDescription = engine.displayName,
                        tint = Color(0xFFFBFBFE),
                        modifier = Modifier.size(size * 0.60f),
                    )
                }
                SearchEngine.STARTPAGE -> {
                    Icon(
                        Icons.Default.Search,
                        contentDescription = engine.displayName,
                        tint = Color(0xFFFBFBFE),
                        modifier = Modifier.size(size * 0.60f),
                    )
                }
                SearchEngine.ECOSIA -> {
                    Icon(
                        Icons.Default.Eco,
                        contentDescription = engine.displayName,
                        tint = Color(0xFFFBFBFE),
                        modifier = Modifier.size(size * 0.60f),
                    )
                }
                SearchEngine.KAGI -> {
                    Icon(
                        Icons.Default.Star,
                        contentDescription = engine.displayName,
                        tint = Color(0xFFFBFBFE),
                        modifier = Modifier.size(size * 0.60f),
                    )
                }
                SearchEngine.MOJEEK -> {
                    Icon(
                        Icons.Default.Explore,
                        contentDescription = engine.displayName,
                        tint = Color(0xFFFBFBFE),
                        modifier = Modifier.size(size * 0.60f),
                    )
                }
                SearchEngine.WIKIPEDIA -> {
                    Icon(
                        Icons.AutoMirrored.Filled.MenuBook,
                        contentDescription = engine.displayName,
                        tint = Color(0xFFFBFBFE),
                        modifier = Modifier.size(size * 0.55f),
                    )
                }
                SearchEngine.YOUTUBE -> {
                    Icon(
                        Icons.Default.PlayArrow,
                        contentDescription = engine.displayName,
                        tint = Color(0xFFFBFBFE),
                        modifier = Modifier.size(size * 0.62f),
                    )
                }
            }
        }
    }
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
                        SearchEngineIcon(
                            engine = engine,
                            size = 32.dp,
                        )
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
    var showSearchField by rememberSaveable { mutableStateOf(false) }
    var filterSpaceId by rememberSaveable { mutableStateOf<String?>(null) }
    var showCloseAllConfirm by remember { mutableStateOf(false) }
    var showMenu by remember { mutableStateOf(false) }

    val visibleTabs = remember(tabs, searchQuery, filterSpaceId) {
        tabs.filter { tab ->
            val matchesSpace = filterSpaceId == null || tab.spaceId == filterSpaceId
            val matchesQuery = searchQuery.isBlank() ||
                tab.title.orEmpty().contains(searchQuery, ignoreCase = true) ||
                tab.url.contains(searchQuery, ignoreCase = true)
            matchesSpace && matchesQuery
        }
    }

    val backgroundColor = MaterialTheme.colorScheme.background
    val cardBg = MaterialTheme.colorScheme.surfaceContainer
    val cardBorder = MaterialTheme.colorScheme.outlineVariant
    val previewSurfaceColor = MaterialTheme.colorScheme.surfaceContainerLow
    val primaryTextColor = MaterialTheme.colorScheme.onSurface
    val secondaryTextColor = MaterialTheme.colorScheme.onSurfaceVariant

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
                        .padding(horizontal = 14.dp),
                ) {
                    // ── Modern Browser Tab Switcher App Bar ─────────────────────
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(top = 10.dp, bottom = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        // Title & Tab count pill
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Text(
                                text = "Tabs",
                                style = MaterialTheme.typography.titleLarge,
                                fontWeight = FontWeight.Bold,
                                color = primaryTextColor,
                            )
                            Surface(
                                shape = CircleShape,
                                color = spaceAccentColor.copy(alpha = 0.16f),
                            ) {
                                Text(
                                    text = "${tabs.size}",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = spaceAccentColor,
                                    fontWeight = FontWeight.Bold,
                                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                                )
                            }
                        }

                        // Keep the workspace actions limited to search, new tab, and options.
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(2.dp),
                        ) {
                            // Search toggle
                            IconButton(
                                onClick = { showSearchField = !showSearchField },
                                modifier = Modifier.size(36.dp),
                            ) {
                                Icon(
                                    Icons.Default.Search,
                                    contentDescription = "Search tabs",
                                    tint = if (showSearchField || searchQuery.isNotEmpty()) spaceAccentColor else secondaryTextColor,
                                    modifier = Modifier.size(20.dp),
                                )
                            }

                            // Quick '+' New Tab
                            IconButton(
                                onClick = { onNewTab(filterSpaceId ?: selectedProfileId) },
                                modifier = Modifier.size(36.dp),
                            ) {
                                Icon(
                                    Icons.Default.Add,
                                    contentDescription = "New tab",
                                    tint = primaryTextColor,
                                    modifier = Modifier.size(22.dp),
                                )
                            }

                            // Overflow menu (Close all tabs, etc.)
                            Box {
                                IconButton(
                                    onClick = { showMenu = true },
                                    modifier = Modifier.size(36.dp),
                                ) {
                                    Icon(
                                        Icons.Default.MoreVert,
                                        contentDescription = "Tab options",
                                        tint = secondaryTextColor,
                                        modifier = Modifier.size(20.dp),
                                    )
                                }
                                DropdownMenu(
                                    expanded = showMenu,
                                    onDismissRequest = { showMenu = false },
                                ) {
                                    DropdownMenuItem(
                                        text = { Text("New tab") },
                                        leadingIcon = { Icon(Icons.Default.Add, null) },
                                        onClick = {
                                            showMenu = false
                                            onNewTab(filterSpaceId ?: selectedProfileId)
                                        },
                                    )
                                    if (visibleTabs.isNotEmpty()) {
                                        DropdownMenuItem(
                                            text = { Text("Close all tabs", color = MaterialTheme.colorScheme.error) },
                                            leadingIcon = { Icon(Icons.Default.Close, null, tint = MaterialTheme.colorScheme.error) },
                                            onClick = {
                                                showMenu = false
                                                showCloseAllConfirm = true
                                            },
                                        )
                                    }
                                }
                            }

                            // Done button (Chrome / Firefox style)
                            TextButton(
                                onClick = onDismiss,
                                contentPadding = PaddingValues(horizontal = 8.dp),
                            ) {
                                Text(
                                    text = "Done",
                                    fontWeight = FontWeight.Bold,
                                    color = spaceAccentColor,
                                )
                            }
                        }
                    }

                    // ── Search Tabs Input (Collapsible) ─────────────────────
                    if (showSearchField || searchQuery.isNotEmpty()) {
                        OutlinedTextField(
                            value = searchQuery,
                            onValueChange = { searchQuery = it },
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(bottom = 10.dp),
                            singleLine = true,
                            placeholder = { Text("Search your tabs", color = secondaryTextColor) },
                            leadingIcon = {
                                Icon(Icons.Default.Search, contentDescription = null, tint = secondaryTextColor)
                            },
                            trailingIcon = {
                                IconButton(onClick = {
                                    searchQuery = ""
                                    showSearchField = false
                                }) {
                                    Icon(Icons.Default.Close, contentDescription = "Close search", tint = secondaryTextColor)
                                }
                            },
                            shape = RoundedCornerShape(14.dp),
                            colors = OutlinedTextFieldDefaults.colors(
                                focusedContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                                unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                                focusedBorderColor = spaceAccentColor.copy(alpha = 0.6f),
                                unfocusedBorderColor = cardBorder.copy(alpha = 0.6f),
                                focusedTextColor = primaryTextColor,
                                unfocusedTextColor = primaryTextColor,
                            ),
                        )
                    }

                    // ── Space / Profile Filter Chips ────────────────────────
                    if (profiles.isNotEmpty()) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(bottom = 10.dp)
                                .horizontalScroll(rememberScrollState()),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            val isAllSelected = filterSpaceId == null
                            Surface(
                                onClick = { filterSpaceId = null },
                                shape = CircleShape,
                                color = if (isAllSelected) {
                                    spaceAccentColor.copy(alpha = 0.2f)
                                } else {
                                    MaterialTheme.colorScheme.surfaceContainer
                                },
                                border = BorderStroke(
                                    1.dp,
                                    if (isAllSelected) spaceAccentColor else cardBorder.copy(alpha = 0.6f),
                                ),
                            ) {
                                Text(
                                    text = "All spaces (${tabs.size})",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = if (isAllSelected) spaceAccentColor else secondaryTextColor,
                                    fontWeight = if (isAllSelected) FontWeight.Bold else FontWeight.Normal,
                                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                                )
                            }

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
                                    color = if (isSelected) {
                                        profileColor.copy(alpha = 0.2f)
                                    } else {
                                        MaterialTheme.colorScheme.surfaceContainer
                                    },
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
                                            fontWeight = if (isSelected) FontWeight.Bold else FontWeight.Normal,
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
                                    modifier = Modifier.size(68.dp),
                                ) {
                                    Box(contentAlignment = Alignment.Center) {
                                        Icon(
                                            Icons.Default.Language,
                                            contentDescription = null,
                                            tint = spaceAccentColor,
                                            modifier = Modifier.size(34.dp),
                                        )
                                    }
                                }
                                Text(
                                    text = if (searchQuery.isNotBlank()) "No tabs match your search" else "No open tabs",
                                    style = MaterialTheme.typography.titleMedium,
                                    fontWeight = FontWeight.SemiBold,
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
                                        modifier = Modifier.padding(horizontal = 18.dp, vertical = 10.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                                    ) {
                                        Icon(Icons.Default.Add, contentDescription = null, modifier = Modifier.size(18.dp))
                                        Text("Open new tab", fontWeight = FontWeight.SemiBold)
                                    }
                                }
                            }
                        }
                    } else {
                        // ── Tab Cards Grid / List (Modern Browser Cards) ────
                        LazyVerticalGrid(
                            columns = GridCells.Fixed(1),
                            modifier = Modifier.weight(1f),
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                            verticalArrangement = Arrangement.spacedBy(12.dp),
                            contentPadding = PaddingValues(top = 4.dp, bottom = 96.dp),
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
                                            color = MaterialTheme.colorScheme.errorContainer,
                                            shape = RoundedCornerShape(16.dp),
                                            modifier = Modifier.fillMaxSize(),
                                        ) {
                                            Box(
                                                contentAlignment = Alignment.CenterEnd,
                                                modifier = Modifier.padding(end = 20.dp),
                                            ) {
                                                Icon(
                                                    Icons.Default.Close,
                                                    "Close tab",
                                                    tint = MaterialTheme.colorScheme.onErrorContainer,
                                                )
                                            }
                                        }
                                    },
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .height(96.dp),
                                ) {
                                    // ── Standard Browser Card Structure ────
                                    Surface(
                                        onClick = { onSelect(tab.id) },
                                        shape = RoundedCornerShape(16.dp),
                                        color = cardBg,
                                        border = BorderStroke(
                                            width = if (isActive) 2.5.dp else 1.dp,
                                            color = if (isActive) spaceAccentColor else cardBorder.copy(alpha = 0.7f),
                                        ),
                                        shadowElevation = if (isActive) 8.dp else 2.dp,
                                        modifier = Modifier.fillMaxSize(),
                                    ) {
                                        Column(
                                            modifier = Modifier.fillMaxSize(),
                                        ) {
                                            // Card Window Title Bar (Favicon + Title + Close '✕')
                                            Row(
                                                verticalAlignment = Alignment.CenterVertically,
                                                modifier = Modifier
                                                    .fillMaxWidth()
                                                    .background(
                                                        if (isActive) spaceAccentColor.copy(alpha = 0.14f)
                                                        else MaterialTheme.colorScheme.surfaceContainerHigh
                                                    )
                                                    .padding(horizontal = 10.dp, vertical = 7.dp),
                                                horizontalArrangement = Arrangement.spacedBy(6.dp),
                                            ) {
                                                Icon(
                                                    imageVector = Icons.Default.Language,
                                                    contentDescription = null,
                                                    tint = if (isActive) spaceAccentColor else secondaryTextColor,
                                                    modifier = Modifier.size(15.dp),
                                                )

                                                Text(
                                                    text = tab.title?.takeIf { it.isNotBlank() } ?: tabHost,
                                                    style = MaterialTheme.typography.labelMedium,
                                                    fontWeight = if (isActive) FontWeight.Bold else FontWeight.SemiBold,
                                                    color = primaryTextColor,
                                                    maxLines = 1,
                                                    overflow = TextOverflow.Ellipsis,
                                                    modifier = Modifier.weight(1f),
                                                )

                                                IconButton(
                                                    onClick = { onClose(tab.id) },
                                                    modifier = Modifier.size(22.dp),
                                                ) {
                                                    Icon(
                                                        Icons.Default.Close,
                                                        contentDescription = "Close tab",
                                                        tint = secondaryTextColor,
                                                        modifier = Modifier.size(14.dp),
                                                    )
                                                }
                                            }

                                            // Keep the card compact and metadata-only until real tab thumbnails exist.
                                            Box(
                                                modifier = Modifier
                                                    .fillMaxWidth()
                                                    .weight(1f)
                                                    .background(previewSurfaceColor)
                                                    .padding(horizontal = 12.dp),
                                            ) {
                                                Row(
                                                    modifier = Modifier.fillMaxSize(),
                                                    verticalAlignment = Alignment.CenterVertically,
                                                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                                                ) {
                                                    Surface(
                                                        shape = CircleShape,
                                                        color = tabSpaceColor.copy(alpha = 0.16f),
                                                        modifier = Modifier.size(32.dp),
                                                    ) {
                                                        Box(contentAlignment = Alignment.Center) {
                                                            Icon(
                                                                Icons.Default.Language,
                                                                contentDescription = null,
                                                                tint = tabSpaceColor,
                                                                modifier = Modifier.size(18.dp),
                                                            )
                                                        }
                                                    }
                                                    Column(modifier = Modifier.weight(1f)) {
                                                        Text(
                                                            text = tabHost,
                                                            style = MaterialTheme.typography.bodyMedium,
                                                            color = primaryTextColor,
                                                            maxLines = 1,
                                                            overflow = TextOverflow.Ellipsis,
                                                        )
                                                        Text(
                                                            text = tab.url,
                                                            style = MaterialTheme.typography.labelSmall,
                                                            color = secondaryTextColor,
                                                            maxLines = 1,
                                                            overflow = TextOverflow.Ellipsis,
                                                        )
                                                        if (tab.isLoading) {
                                                            LinearProgressIndicator(
                                                                progress = { (tab.progress / 100f).coerceIn(0f, 1f) },
                                                                modifier = Modifier
                                                                    .fillMaxWidth()
                                                                    .padding(top = 5.dp)
                                                                    .height(2.dp),
                                                                color = spaceAccentColor,
                                                            )
                                                        }
                                                    }
                                                }
                                            }

                                            Row(
                                                modifier = Modifier
                                                    .fillMaxWidth()
                                                    .padding(horizontal = 12.dp, vertical = 4.dp),
                                                verticalAlignment = Alignment.CenterVertically,
                                                horizontalArrangement = Arrangement.End,
                                            ) {
                                                if (tabSpace != null) {
                                                    Row(
                                                        verticalAlignment = Alignment.CenterVertically,
                                                        horizontalArrangement = Arrangement.spacedBy(3.dp),
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

                Row(
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(bottom = 16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    ExtendedFloatingActionButton(
                        onClick = { onNewTab(filterSpaceId ?: selectedProfileId) },
                        icon = { Icon(Icons.Default.Add, contentDescription = null) },
                        text = { Text("New tab") },
                        containerColor = spaceAccentColor,
                    )
                    IconButton(onClick = onOpenBrowserMenu) {
                        Icon(Icons.Default.MoreVert, contentDescription = "Browser menu")
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

// ── Horizontal Desktop/Tablet-Style Tab Strip ──────────────────────────────
@Composable
private fun BrowserTabStrip(
    tabs: List<TabState>,
    activeTabId: String?,
    spaceAccentColor: Color,
    onSelectTab: (String) -> Unit,
    onCloseTab: (String) -> Unit,
    onNewTab: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        color = Color(0xFF0F141C),
        modifier = modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 6.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            tabs.forEach { tab ->
                val isActive = tab.id == activeTabId
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

                Surface(
                    onClick = { onSelectTab(tab.id) },
                    shape = RoundedCornerShape(topStart = 8.dp, topEnd = 8.dp, bottomStart = 4.dp, bottomEnd = 4.dp),
                    color = if (isActive) Color(0xFF1E2636) else Color(0xFF141923),
                    border = BorderStroke(
                        width = 1.dp,
                        color = if (isActive) spaceAccentColor.copy(alpha = 0.6f) else Color(0xFF263040).copy(alpha = 0.5f),
                    ),
                    modifier = Modifier
                        .widthIn(min = 120.dp, max = 180.dp)
                        .height(34.dp),
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxSize()
                            .padding(horizontal = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Icon(
                            imageVector = Icons.Default.Language,
                            contentDescription = null,
                            tint = if (isActive) spaceAccentColor else Color(0xFF93929D),
                            modifier = Modifier.size(14.dp),
                        )
                        Text(
                            text = tab.title?.takeIf { it.isNotBlank() } ?: tabHost,
                            style = MaterialTheme.typography.labelMedium,
                            fontWeight = if (isActive) FontWeight.SemiBold else FontWeight.Normal,
                            color = if (isActive) Color(0xFFFBFBFE) else Color(0xFF93929D),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                        IconButton(
                            onClick = { onCloseTab(tab.id) },
                            modifier = Modifier.size(20.dp),
                        ) {
                            Icon(
                                imageVector = Icons.Default.Close,
                                contentDescription = "Close tab",
                                tint = Color(0xFF93929D),
                                modifier = Modifier.size(12.dp),
                            )
                        }
                    }
                }
            }

            // '+' New Tab button
            IconButton(
                onClick = onNewTab,
                modifier = Modifier.size(30.dp),
            ) {
                Icon(
                    imageVector = Icons.Default.Add,
                    contentDescription = "New Tab",
                    tint = Color(0xFF93929D),
                    modifier = Modifier.size(16.dp),
                )
            }
        }
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
    searchEngine: SearchEngine,
    searchSuggestionsEnabled: Boolean,
    profileName: String,
    versionName: String,
    onSearchSuggestionsChanged: (Boolean) -> Unit,
    onDismiss: () -> Unit,
    onOpenSearchEngine: () -> Unit,
    onOpenThemePicker: () -> Unit,
    onOpenPrivacy: () -> Unit,
    onOpenTrackingProtection: () -> Unit,
    onOpenHistory: () -> Unit,
    onOpenTabs: () -> Unit,
    onOpenSpaces: () -> Unit,
    onOpenDownloads: () -> Unit,
    onOpenUpdates: () -> Unit,
) {
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier
                .fillMaxSize()
                .padding(horizontal = 12.dp, vertical = 24.dp)
                .widthIn(max = 640.dp),
            shape = MaterialTheme.shapes.extraLarge,
            color = MaterialTheme.colorScheme.surface,
            contentColor = MaterialTheme.colorScheme.onSurface,
        ) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .windowInsetsPadding(WindowInsets.safeDrawing),
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(start = 24.dp, end = 12.dp, top = 12.dp, bottom = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Settings",
                        style = MaterialTheme.typography.headlineSmall,
                        modifier = Modifier.weight(1f),
                    )
                    IconButton(onClick = onDismiss) {
                        Icon(Icons.Default.Close, contentDescription = "Close settings")
                    }
                }
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 20.dp),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    SettingsSection("Appearance") {
                        SettingsActionRow(
                            title = "Theme & appearance",
                            subtitle = "Follows Android light/dark mode; choose accent color",
                            onClick = onOpenThemePicker,
                        )
                    }
                    SettingsSection("Browser") {
                        SettingsActionRow(
                            title = "Search engine",
                            subtitle = searchEngine.displayName,
                            onClick = onOpenSearchEngine,
                        )
                        SettingToggleRow(
                            title = "Search suggestions",
                            checked = searchSuggestionsEnabled,
                            onCheckedChange = onSearchSuggestionsChanged,
                        )
                        SettingsActionRow(
                            title = "Downloads",
                            subtitle = "View downloads",
                            onClick = onOpenDownloads,
                        )
                    }
                    SettingsSection("Privacy & Security") {
                        SettingsActionRow(
                            title = "Tracking protection",
                            subtitle = "View protection for this site",
                            onClick = onOpenTrackingProtection,
                        )
                        SettingsActionRow(
                            title = "Privacy controls",
                            subtitle = "Browser-wide protections and HTTPS-only mode",
                            onClick = onOpenPrivacy,
                        )
                        SettingsActionRow(
                            title = "Clear browsing history",
                            subtitle = "Review and clear history",
                            onClick = onOpenHistory,
                        )
                    }
                    SettingsSection("Tabs") {
                        SettingsActionRow(
                            title = "Tab overview",
                            subtitle = "Switch or close tabs",
                            onClick = onOpenTabs,
                        )
                        SettingsActionRow(
                            title = "Spaces & profiles",
                            subtitle = "Current profile: $profileName",
                            onClick = onOpenSpaces,
                        )
                    }
                    SettingsSection("About") {
                        SettingsActionRow(
                            title = "FetchX",
                            subtitle = "Version $versionName · Check for updates",
                            onClick = onOpenUpdates,
                        )
                    }
                }
                TextButton(
                    onClick = onDismiss,
                    modifier = Modifier
                        .align(Alignment.End)
                        .padding(horizontal = 16.dp, vertical = 8.dp),
                ) {
                    Text("Done")
                }
            }
        }
    }
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
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = title,
            modifier = Modifier.weight(1f),
        )
        Switch(
            checked = checked,
            onCheckedChange = onCheckedChange,
            modifier = Modifier.semantics {
                contentDescription = title
                stateDescription = if (checked) "On" else "Off"
            },
        )
    }
}

@Composable
private fun SettingsSection(
    title: String,
    content: @Composable () -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(top = 16.dp, bottom = 4.dp),
        )
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        content()
    }
}

@Composable
private fun SettingsActionRow(
    title: String,
    subtitle: String,
    onClick: () -> Unit,
) {
    Surface(
        onClick = onClick,
        color = MaterialTheme.colorScheme.surface,
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp),
    ) {
        Column(
            modifier = Modifier.padding(vertical = 8.dp),
            verticalArrangement = Arrangement.Center,
        ) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(
                subtitle,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
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

/** Returns true for URLs that point directly to a media file and can be
 * downloaded without yt-dlp format extraction.
 */
private fun isDirectMediaUrl(url: String): Boolean = BrowserViewModel.isDirectMediaUrl(url)

@Composable
private fun VideoDownloadBanner(
    video: DetectedVideo,
    extraCount: Int,
    onDownload: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        shape = RoundedCornerShape(topStart = 18.dp, topEnd = 18.dp),
        color = Color(0xFF2B2A33),
        border = BorderStroke(1.dp, Color(0xFF383741)),
        shadowElevation = 8.dp,
        modifier = modifier.fillMaxWidth(),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            // Video icon in accent circle
            Surface(
                shape = RoundedCornerShape(12.dp),
                color = Color(0xFF4F8EF7).copy(alpha = 0.15f),
                modifier = Modifier.size(40.dp),
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(
                        Icons.Default.OndemandVideo,
                        contentDescription = null,
                        tint = Color(0xFF4F8EF7),
                        modifier = Modifier.size(22.dp),
                    )
                }
            }
            Spacer(Modifier.width(12.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    "Video detected",
                    style = MaterialTheme.typography.labelMedium.copy(
                        fontWeight = FontWeight.SemiBold,
                    ),
                    color = Color(0xFFFBFBFE),
                )
                Text(
                    video.suggestedFilename,
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    color = Color(0xFF93929D),
                )
                if (extraCount > 0) {
                    Text(
                        "+$extraCount more",
                        style = MaterialTheme.typography.labelSmall,
                        color = Color(0xFF93929D),
                    )
                }
            }
            Spacer(Modifier.width(8.dp))
            // Download pill button
            Surface(
                onClick = onDownload,
                shape = RoundedCornerShape(12.dp),
                color = Color(0xFF4F8EF7),
            ) {
                Text(
                    "Download",
                    style = MaterialTheme.typography.labelMedium.copy(
                        fontWeight = FontWeight.SemiBold,
                    ),
                    color = Color.White,
                    modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp),
                )
            }
            Spacer(Modifier.width(6.dp))
            Surface(
                onClick = onDismiss,
                shape = CircleShape,
                color = Color(0xFF383741),
                modifier = Modifier.size(30.dp),
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(Icons.Default.Close, "Dismiss",
                        tint = Color(0xFF93929D), modifier = Modifier.size(14.dp))
                }
            }
        }
    }
}
