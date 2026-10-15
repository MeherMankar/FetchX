package dev.fetchx.browser

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.activity.compose.rememberLauncherForActivityResult
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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.GridView
import androidx.compose.material.icons.filled.Language
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.OndemandVideo
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Star
import androidx.compose.material3.AlertDialog
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
    onDownloadCurrentPage: (url: String, title: String) -> Unit = { _, _ -> },
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
    var showBrowserUpdates by rememberSaveable { mutableStateOf(false) }

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

    // ── Compact mode: full-screen web with floating dot ────────────────────
    if (uiState.compactMode) {
        Box(modifier = Modifier.fillMaxSize()) {
            val context = LocalContext.current
            AndroidView(
                factory = { ctx ->
                    GeckoView(ctx).also { viewModel.attachView(it, GeckoRuntimeHolder.getOrCreate(ctx)) }
                },
                modifier = Modifier
                    .fillMaxSize()
                    .pointerInput(Unit) {
                        detectTapGestures(onDoubleTap = { viewModel.toggleCompactMode() })
                    },
            )
            // Tiny colored dot to exit compact mode
            Surface(
                onClick = { viewModel.toggleCompactMode() },
                shape = CircleShape,
                color = spaceAccentColor.copy(alpha = 0.85f),
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .padding(bottom = 24.dp)
                    .size(28.dp),
            ) {}
        }
        return
    }

    Scaffold(
        // ── Full-width address bar ───────────────────────────────────────────
        topBar = {
            Column {
                Surface(
                    color = MaterialTheme.colorScheme.surface.copy(alpha = 0.95f).let {
                        androidx.compose.ui.graphics.lerp(it, spaceAccentColor, 0.08f)
                    },
                    modifier = Modifier.windowInsetsPadding(WindowInsets.statusBars),
                ) {
                    Box(
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
                    ) {
                        if (isUrlBarExpanded) {
                            OutlinedTextField(
                                value = urlFieldValue,
                                onValueChange = { tfv ->
                                    urlFieldValue = tfv
                                    viewModel.onUrlInputChanged(tfv.text)
                                },
                                singleLine = true,
                                placeholder = { Text("Search or enter URL") },
                                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Go),
                                keyboardActions = KeyboardActions(onGo = {
                                    isUrlBarExpanded = false
                                    onNavigate()
                                }),
                                modifier = Modifier.fillMaxWidth().focusRequester(urlFocusRequester),
                                shape = MaterialTheme.shapes.large,
                                leadingIcon = {
                                    TextButton(
                                        onClick = { viewModel.openSearchEngineSelector() },
                                        contentPadding = PaddingValues(horizontal = 6.dp),
                                    ) {
                                        Text(
                                            uiState.selectedSearchEngine.displayName.take(2),
                                            style = MaterialTheme.typography.labelSmall,
                                            color = if (uiState.selectedSearchEngine.isPrivate)
                                                MaterialTheme.colorScheme.tertiary
                                            else MaterialTheme.colorScheme.primary,
                                        )
                                    }
                                },
                                trailingIcon = {
                                    IconButton(onClick = { isUrlBarExpanded = false }) {
                                        Icon(Icons.Default.Close, "Close")
                                    }
                                },
                            )
                        } else {
                            val url = uiState.activeTab?.url.orEmpty()
                            val address = if (url == "about:blank" || url.isBlank()) {
                                "Search or enter URL"
                            } else {
                                url.removePrefix("https://").removePrefix("http://")
                                    .substringBefore("/")
                            }
                            Surface(
                                onClick = {
                                    isUrlBarExpanded = true
                                    viewModel.onUrlInputChanged(
                                        url.takeUnless { it == "about:blank" }.orEmpty(),
                                    )
                                },
                                shape = CircleShape,
                                color = MaterialTheme.colorScheme.surfaceVariant,
                                modifier = Modifier.fillMaxWidth().height(48.dp),
                            ) {
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    modifier = Modifier.padding(horizontal = 12.dp),
                                ) {
                                    Icon(
                                        Icons.Default.Search,
                                        contentDescription = null,
                                        modifier = Modifier.size(18.dp),
                                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                    Text(
                                        address,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        modifier = Modifier.padding(start = 8.dp),
                                    )
                                }
                            }
                        }
                    }
                }
                if (uiState.activeTab?.isLoading == true) {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
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
                        tonalElevation = 3.dp,
                        shape = MaterialTheme.shapes.medium,
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp),
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
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clickable {
                                            viewModel.applySuggestion(suggestion)
                                            isUrlBarExpanded = false
                                        }
                                        .padding(horizontal = 12.dp, vertical = 8.dp),
                                ) {
                                    Icon(
                                        Icons.Default.Search,
                                        null,
                                        tint = MaterialTheme.colorScheme.primary,
                                        modifier = Modifier.size(16.dp),
                                    )
                                    Column(modifier = Modifier.weight(1f).padding(start = 8.dp)) {
                                        Text(
                                            suggestion.title,
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                            style = MaterialTheme.typography.bodySmall,
                                        )
                                        Text(
                                            suggestion.category,
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        // ── Bottom navigation ────────────────────────────────────────────────
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

            if (showBrowserMenu) {
                ModalBottomSheet(onDismissRequest = onBrowserMenuDismiss) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .verticalScroll(rememberScrollState())
                            .padding(bottom = 24.dp),
                    ) {
                        Text(
                            "Browser menu",
                            style = MaterialTheme.typography.titleLarge,
                            modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
                        )
                        DropdownMenuItem(
                            text = { Text("Home") },
                            onClick = { onBrowserMenuDismiss(); viewModel.navigate("about:blank") },
                        )
                        DropdownMenuItem(
                            text = { Text("Bookmarks") },
                            onClick = { onBrowserMenuDismiss(); viewModel.openBookmarks() },
                        )
                        DropdownMenuItem(
                            text = { Text("Download media") },
                            enabled = activeTab?.url?.startsWith("http://") == true ||
                                activeTab?.url?.startsWith("https://") == true,
                            onClick = {
                                val tab = activeTab
                                if (tab != null) {
                                    onBrowserMenuDismiss()
                                    onDownloadCurrentPage(tab.url, tab.title.orEmpty())
                                }
                            },
                        )
                        DropdownMenuItem(
                            text = { Text("Tracking protection · ${uiState.activeTabBlockedRequests} blocked") },
                            onClick = { onBrowserMenuDismiss(); viewModel.openTrackerDetails() },
                        )
                        DropdownMenuItem(
                            text = { Text("Privacy") },
                            onClick = { onBrowserMenuDismiss(); viewModel.openPrivacySettings() },
                        )
                        DropdownMenuItem(
                            text = { Text("Extensions") },
                            onClick = { onBrowserMenuDismiss(); viewModel.openExtensions() },
                        )
                        DropdownMenuItem(
                            text = { Text("Back") },
                            enabled = uiState.activeTab?.canGoBack == true,
                            onClick = { onBrowserMenuDismiss(); viewModel.goBack() },
                        )
                        DropdownMenuItem(
                            text = { Text("Forward") },
                            enabled = uiState.activeTab?.canGoForward == true,
                            onClick = { onBrowserMenuDismiss(); viewModel.goForward() },
                        )
                        DropdownMenuItem(
                            text = {
                                Text(if (uiState.isCurrentPageBookmarked) "Remove bookmark" else "Bookmark page")
                            },
                            enabled = uiState.activeTab?.let { tab ->
                                (tab.url.startsWith("http://") || tab.url.startsWith("https://")) && !tab.isLoading
                            } == true,
                            onClick = { onBrowserMenuDismiss(); viewModel.toggleBookmark() },
                        )
                        DropdownMenuItem(
                            text = { Text("Settings") },
                            onClick = { onBrowserMenuDismiss(); viewModel.openBrowserSettings() },
                        )
                        DropdownMenuItem(
                            text = { Text("Browser updates") },
                            onClick = {
                                onBrowserMenuDismiss()
                                showBrowserUpdates = true
                            },
                        )
                        DropdownMenuItem(
                            text = { Text("Theme") },
                            onClick = { onBrowserMenuDismiss(); viewModel.openThemePicker() },
                        )
                        DropdownMenuItem(
                            text = { Text("Compact mode") },
                            onClick = { onBrowserMenuDismiss(); viewModel.toggleCompactMode() },
                        )
                        DropdownMenuItem(
                            text = { Text("New tab") },
                            onClick = { onBrowserMenuDismiss(); viewModel.newTabInCurrentSpace() },
                        )
                        DropdownMenuItem(
                            text = { Text("Profile") },
                            onClick = { onBrowserMenuDismiss(); viewModel.openProfileSwitcher() },
                        )
                        DropdownMenuItem(
                            text = { Text("Space") },
                            onClick = { onBrowserMenuDismiss(); viewModel.openSpaceSwitcher() },
                        )
                        DropdownMenuItem(
                            text = { Text("History") },
                            onClick = { onBrowserMenuDismiss(); viewModel.openHistory() },
                        )
                        DropdownMenuItem(
                            text = { Text("Tabs (${uiState.tabs.size})") },
                            onClick = { onBrowserMenuDismiss(); viewModel.openTabOverview() },
                        )
                        DropdownMenuItem(
                            text = { Text("Reader mode") },
                            enabled = uiState.readerModeAvailable || uiState.readerModeActive,
                            onClick = { onBrowserMenuDismiss(); viewModel.toggleReaderMode() },
                        )
                        DropdownMenuItem(
                            text = { Text("Reload") },
                            onClick = { onBrowserMenuDismiss(); viewModel.reload() },
                        )
                    }
                }
            }

            // GeckoView
            val context = LocalContext.current
            AndroidView(
                factory = { ctx ->
                    GeckoView(ctx).also { viewModel.attachView(it, GeckoRuntimeHolder.getOrCreate(ctx)) }
                },
                modifier = Modifier
                    .fillMaxSize()
                    .pointerInput(Unit) {
                        detectTapGestures(onDoubleTap = { viewModel.toggleCompactMode() })
                    },
            )

            // ── Essentials / new-tab page (Feature 3) ─────────────────────────
            if (uiState.showEssentials) {
                EssentialsPage(
                    essentials = uiState.essentials,
                    spaceAccentColor = spaceAccentColor,
                    onNavigate = { viewModel.navigateToEssential(it) },
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
                            if (count == 1) "1 video detected — tap to download"
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
                                "Choose video to download",
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
                                    Spacer(Modifier.width(8.dp))
                                    Icon(
                                        Icons.Default.Download,
                                        contentDescription = "Download",
                                        tint = MaterialTheme.colorScheme.primary,
                                        modifier = Modifier.size(20.dp),
                                    )
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
                    }
                    else -> Column(
                        modifier = Modifier
                            .heightIn(max = 400.dp)
                            .verticalScroll(rememberScrollState()),
                    ) {
                        TextButton(
                            onClick = { viewModel.downloadDetectedVideo(video) },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Text("Automatic best quality")
                        }
                        uiState.mediaFormats.forEach { format ->
                            TextButton(
                                onClick = {
                                    viewModel.downloadDetectedVideo(
                                        video,
                                        format.formatId,
                                        format.extension,
                                    )
                                },
                                modifier = Modifier.fillMaxWidth(),
                            ) {
                                Text(format.label)
                            }
                        }
                        if (uiState.mediaFormats.isEmpty()) {
                            Text(
                                "No selectable formats were found. Automatic quality selection is still available.",
                                style = MaterialTheme.typography.bodySmall,
                                modifier = Modifier.padding(top = 8.dp),
                            )
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
            onSelect = { viewModel.selectTab(it); viewModel.closeTabOverview() },
            onClose = { viewModel.closeTab(it) },
            onNewTab = { viewModel.newTab(); viewModel.closeTabOverview() },
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

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun EssentialsPage(
    essentials: List<Essential>,
    spaceAccentColor: Color,
    onNavigate: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier = modifier
            .fillMaxSize()
            .background(
                brush = androidx.compose.ui.graphics.Brush.verticalGradient(
                    colors = listOf(
                        spaceAccentColor.copy(alpha = 0.15f),
                        MaterialTheme.colorScheme.background,
                    )
                )
            )
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Spacer(Modifier.height(32.dp))
            Text(
                text = "Most visited",
                style = MaterialTheme.typography.headlineMedium,
                color = MaterialTheme.colorScheme.onBackground,
            )
            Spacer(Modifier.height(32.dp))
            if (essentials.isEmpty()) {
                Text(
                    text = "Sites you visit will appear here.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                LazyVerticalGrid(
                    columns = GridCells.Fixed(3),
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(16.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                    contentPadding = PaddingValues(4.dp),
                ) {
                    gridItems(essentials, key = { it.id }) { item ->
                        Column(
                            horizontalAlignment = Alignment.CenterHorizontally,
                            modifier = Modifier.clickable { onNavigate(item.url) },
                        ) {
                            Surface(
                                shape = CircleShape,
                                color = try {
                                    Color(android.graphics.Color.parseColor(item.colorHex))
                                } catch (_: Exception) { spaceAccentColor },
                                modifier = Modifier.size(56.dp),
                            ) {
                                Box(contentAlignment = Alignment.Center) {
                                    Text(
                                        text = item.initial,
                                        style = MaterialTheme.typography.titleLarge,
                                        color = Color.White,
                                    )
                                }
                            }
                            Spacer(Modifier.height(6.dp))
                            Text(
                                text = item.title,
                                style = MaterialTheme.typography.labelSmall,
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
    onSelect: (String) -> Unit,
    onClose: (String) -> Unit,
    onNewTab: () -> Unit,
    onDismiss: () -> Unit,
    onOpenBrowserMenu: () -> Unit,
) {
    var searchQuery by rememberSaveable { mutableStateOf("") }
    var isGridView by rememberSaveable { mutableStateOf(true) }
    val visibleTabs = tabs.filter { tab ->
        searchQuery.isBlank() ||
            tab.title.orEmpty().contains(searchQuery, ignoreCase = true) ||
            tab.url.contains(searchQuery, ignoreCase = true)
    }
    val backgroundColor = Color(0xFF101319)
    val cardColor = Color(0xFF292D36)
    val selectedCardColor = Color(0xFF9CCBFF)
    val primaryTextColor = Color(0xFFF2F3F7)
    val secondaryTextColor = Color(0xFFB8BEC9)

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier.fillMaxSize(),
            color = backgroundColor,
        ) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .windowInsetsPadding(WindowInsets.safeDrawing)
                    .padding(horizontal = 16.dp),
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 12.dp, bottom = 20.dp),
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Surface(
                        shape = CircleShape,
                        color = cardColor,
                    ) {
                        Row(
                            modifier = Modifier.padding(4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            IconButton(
                                onClick = { isGridView = false },
                                modifier = Modifier
                                    .size(48.dp)
                                    .background(
                                        if (!isGridView) Color(0xFF3A3F49) else Color.Transparent,
                                        CircleShape,
                                    ),
                            ) {
                                Box(contentAlignment = Alignment.Center) {
                                    androidx.compose.foundation.Canvas(
                                        modifier = Modifier.size(28.dp),
                                    ) {
                                        drawRoundRect(
                                            color = if (!isGridView) selectedCardColor else secondaryTextColor,
                                            cornerRadius = androidx.compose.ui.geometry.CornerRadius(5.dp.toPx()),
                                            style = androidx.compose.ui.graphics.drawscope.Stroke(width = 2.dp.toPx()),
                                        )
                                    }
                                    Text(
                                        text = tabs.size.toString(),
                                        style = MaterialTheme.typography.labelMedium,
                                        color = if (!isGridView) selectedCardColor else secondaryTextColor,
                                    )
                                }
                            }
                            IconButton(
                                onClick = { isGridView = true },
                                modifier = Modifier
                                    .size(48.dp)
                                    .background(
                                        if (isGridView) Color(0xFF3A3F49) else Color.Transparent,
                                        CircleShape,
                                    ),
                            ) {
                                Icon(
                                    imageVector = Icons.Default.GridView,
                                    contentDescription = "Grid view",
                                    tint = if (isGridView) selectedCardColor else secondaryTextColor,
                                )
                            }
                        }
                    }
                }

                OutlinedTextField(
                    value = searchQuery,
                    onValueChange = { searchQuery = it },
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(bottom = 20.dp),
                    singleLine = true,
                    placeholder = { Text("Search your tabs", color = secondaryTextColor) },
                    leadingIcon = {
                        Icon(Icons.Default.Search, contentDescription = null, tint = secondaryTextColor)
                    },
                    trailingIcon = if (searchQuery.isNotEmpty()) {
                        {
                            IconButton(onClick = { searchQuery = "" }) {
                                Icon(Icons.Default.Close, contentDescription = "Clear search")
                            }
                        }
                    } else {
                        null
                    },
                    shape = CircleShape,
                    colors = androidx.compose.material3.OutlinedTextFieldDefaults.colors(
                        focusedContainerColor = cardColor,
                        unfocusedContainerColor = cardColor,
                        focusedBorderColor = Color.Transparent,
                        unfocusedBorderColor = Color.Transparent,
                        focusedTextColor = primaryTextColor,
                        unfocusedTextColor = primaryTextColor,
                    ),
                )

                LazyVerticalGrid(
                    columns = if (isGridView) GridCells.Adaptive(minSize = 160.dp) else GridCells.Fixed(1),
                    modifier = Modifier.weight(1f),
                    horizontalArrangement = Arrangement.spacedBy(14.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                    contentPadding = PaddingValues(bottom = 20.dp),
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
                        SwipeToDismissBox(
                            state = dismissState,
                            backgroundContent = {
                                Surface(
                                    color = Color(0xFF692F39),
                                    shape = RoundedCornerShape(28.dp),
                                    modifier = Modifier.fillMaxSize(),
                                ) {
                                    Box(
                                        contentAlignment = Alignment.CenterEnd,
                                        modifier = Modifier.padding(end = 18.dp),
                                    ) {
                                        Icon(Icons.Default.Close, "Close tab", tint = Color.White)
                                    }
                                }
                            },
                            modifier = Modifier
                                .fillMaxWidth()
                                .then(
                                    if (isGridView) Modifier.aspectRatio(0.72f)
                                    else Modifier.height(220.dp),
                                ),
                        ) {
                            Surface(
                                onClick = { onSelect(tab.id) },
                                shape = RoundedCornerShape(28.dp),
                                color = if (isActive) selectedCardColor else cardColor,
                                modifier = Modifier.fillMaxSize(),
                            ) {
                                Column(modifier = Modifier.padding(10.dp)) {
                                    Row(
                                        verticalAlignment = Alignment.CenterVertically,
                                        modifier = Modifier.padding(bottom = 8.dp),
                                    ) {
                                        Surface(
                                            shape = CircleShape,
                                            color = if (isActive) Color(0xFF2589E8) else Color(0xFF59616E),
                                            modifier = Modifier.size(28.dp),
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
                                        Text(
                                            text = tab.title?.takeIf { it.isNotBlank() }
                                                ?: tab.url.takeUnless { it.isBlank() || it == "about:blank" }
                                                    ?.removePrefix("https://")
                                                    ?.removePrefix("http://")
                                                    ?.substringBefore("/")
                                                    .orEmpty()
                                                    .ifBlank { "New tab" },
                                            style = MaterialTheme.typography.titleSmall,
                                            color = if (isActive) Color(0xFF123A5B) else primaryTextColor,
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                            modifier = Modifier
                                                .weight(1f)
                                                .padding(horizontal = 8.dp),
                                        )
                                        IconButton(
                                            onClick = { onClose(tab.id) },
                                            modifier = Modifier.size(32.dp),
                                        ) {
                                            Icon(
                                                Icons.Default.Close,
                                                contentDescription = "Close tab",
                                                tint = if (isActive) Color(0xFF123A5B) else secondaryTextColor,
                                                modifier = Modifier.size(20.dp),
                                            )
                                        }
                                    }

                                    Surface(
                                        modifier = Modifier
                                            .fillMaxWidth()
                                            .weight(1f),
                                        shape = RoundedCornerShape(20.dp),
                                        color = Color(0xFF11151B),
                                    ) {
                                        Column(
                                            modifier = Modifier.padding(12.dp),
                                            verticalArrangement = Arrangement.spacedBy(10.dp),
                                        ) {
                                            Row(verticalAlignment = Alignment.CenterVertically) {
                                                Text(
                                                    text = tab.title?.takeIf { it.isNotBlank() }
                                                        ?: "New tab",
                                                    style = MaterialTheme.typography.labelMedium,
                                                    color = Color(0xFFE6E9EF),
                                                    maxLines = 1,
                                                    overflow = TextOverflow.Ellipsis,
                                                    modifier = Modifier.weight(1f),
                                                )
                                                if (tab.isLoading) {
                                                    androidx.compose.material3.CircularProgressIndicator(
                                                        modifier = Modifier.size(14.dp),
                                                        strokeWidth = 2.dp,
                                                        color = selectedCardColor,
                                                    )
                                                }
                                            }
                                            Text(
                                                text = tab.url.takeUnless { it == "about:blank" }
                                                    ?.removePrefix("https://")
                                                    ?.removePrefix("http://")
                                                    .orEmpty()
                                                    .ifBlank { "Start browsing" },
                                                style = MaterialTheme.typography.bodySmall,
                                                color = secondaryTextColor,
                                                maxLines = 2,
                                                overflow = TextOverflow.Ellipsis,
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 8.dp, bottom = 8.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Surface(
                        onClick = onNewTab,
                        shape = RoundedCornerShape(18.dp),
                        color = selectedCardColor,
                        modifier = Modifier.size(64.dp),
                    ) {
                        Box(contentAlignment = Alignment.Center) {
                            Icon(
                                Icons.Default.Add,
                                contentDescription = "New tab",
                                tint = Color(0xFF123A5B),
                                modifier = Modifier.size(32.dp),
                            )
                        }
                    }
                    IconButton(
                        onClick = onOpenBrowserMenu,
                        modifier = Modifier.align(Alignment.CenterEnd),
                    ) {
                        Icon(
                            Icons.Default.MoreVert,
                            contentDescription = "Browser menu",
                            tint = secondaryTextColor,
                        )
                    }
                }
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
                    state.isLoading -> Text("Loading installed extensionsâ€¦")
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
                            text = "Version ${extension.metadata.version} Â· ${extension.state.name.lowercase()}",
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
                                    text = " Â· Built in",
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
                        isImporting -> "Readingâ€¦"
                        state.operationState == ExtensionState.INSTALLING -> "Installingâ€¦"
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
                    request.origins.map { "Site access: $it" }).forEach { item ->
                    Text("- $item", style = MaterialTheme.typography.bodySmall)
                }
                Text(
                    "Approving grants these requested permissions. Access to private browsing remains off.",
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

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun TabOverviewDialog(
    tabs: List<TabState>,
    activeTabId: String?,
    onSelect: (String) -> Unit,
    onClose: (String) -> Unit,
    onNewTab: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Tabs") },
        text = {
            Column(
                modifier = Modifier
                    .heightIn(max = 420.dp)
                    .verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    tabs.forEach { tab ->
                        val isActive = tab.id == activeTabId
                        Surface(
                            color = if (isActive) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant,
                            shape = MaterialTheme.shapes.medium,
                            modifier = Modifier
                                .width(150.dp)
                                .clickable { onSelect(tab.id) },
                        ) {
                            Column(
                                modifier = Modifier.padding(10.dp),
                                verticalArrangement = Arrangement.spacedBy(6.dp),
                            ) {
                                Text(
                                    text = tab.title ?: tab.url,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.SpaceBetween,
                                ) {
                                    Text(
                                        text = if (isActive) "Active" else "Open",
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                    IconButton(
                                        onClick = { onClose(tab.id) },
                                        modifier = Modifier.size(20.dp),
                                    ) {
                                        Icon(
                                            imageVector = Icons.Default.Close,
                                            contentDescription = "Close tab",
                                            modifier = Modifier.size(16.dp),
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
                TextButton(onClick = onNewTab) {
                    Icon(
                        imageVector = Icons.Default.Add,
                        contentDescription = null,
                        modifier = Modifier.size(16.dp),
                    )
                    Text("New tab", modifier = Modifier.padding(start = 8.dp))
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Done") }
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
