package dev.fetchx.app.navigation

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Language
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Tab
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.Modifier
import android.net.Uri
import android.content.ActivityNotFoundException
import android.widget.Toast
import androidx.core.content.FileProvider
import androidx.compose.runtime.rememberCoroutineScope
import dev.fetchx.app.ui.theme.FetchXTheme
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import androidx.navigation.NavHostController
import dev.fetchx.downloads.storage.StorageManager
import dev.fetchx.bridge.BridgeInitializer
import dev.fetchx.browser.BrowserScreen
import dev.fetchx.core.download.NoOpDownloadRepository
import dev.fetchx.downloads.ui.DownloadsScreen
import dev.fetchx.downloads.ui.DownloadsViewModel
import dev.fetchx.downloads.ui.TorrentDetailScreen
import dev.fetchx.core.download.DownloadPriority
import dev.fetchx.app.storage.StorageScreen
import kotlinx.coroutines.launch
import kotlinx.coroutines.CancellationException
import androidx.compose.runtime.LaunchedEffect
import java.io.File

/**
 * Root NavHost for the FetchX application.
 *
 * A [NavigationBar] at the bottom provides browser actions and access to
 * Downloads while preserving the shared navigation back stack.
 */
@Composable
fun AppNavigation(initialUrl: String? = null, pendingIntentUrl: androidx.compose.runtime.MutableState<String?> = androidx.compose.runtime.mutableStateOf(null), pendingDownloadId: androidx.compose.runtime.MutableState<String?> = androidx.compose.runtime.mutableStateOf(null)) {
    val navController = rememberNavController()
    val navBackStackEntry by navController.currentBackStackEntryAsState()
    val currentRoute = navBackStackEntry?.destination?.route
    var selectedBrowserItem by remember { mutableStateOf("home") }
    var homeRequest by remember { mutableLongStateOf(0L) }
    var searchRequest by remember { mutableLongStateOf(0L) }
    var tabsRequest by remember { mutableLongStateOf(0L) }
    var showBrowserMenu by remember { mutableStateOf(false) }
    // URL to navigate to on startup (from ACTION_VIEW intent)
    var pendingUrl by remember { mutableStateOf(if (initialUrl?.startsWith("magnet:") == true) null else initialUrl) }
    // Magnet URI to add as download (from ACTION_VIEW magnet: intent)
    var pendingMagnet by remember { mutableStateOf(if (initialUrl?.startsWith("magnet:") == true) initialUrl else null) }

    // Observe new intents arriving via onNewIntent (activity already running).
    val newIntentUrl = pendingIntentUrl.value
    if (newIntentUrl != null) {
        pendingIntentUrl.value = null   // consume it
        if (newIntentUrl.startsWith("magnet:", ignoreCase = true)) {
            pendingMagnet = newIntentUrl
        } else {
            pendingUrl = newIntentUrl
        }
    }

    // Pending download ID from notification tap — consumed below in a LaunchedEffect
    // once the ViewModel is available to look up the download's engine.
    var pendingOpenDownloadId by remember { mutableStateOf<String?>(null) }
    val newDownloadId = pendingDownloadId.value
    if (newDownloadId != null) {
        pendingDownloadId.value = null        // consume from activity state
        pendingOpenDownloadId = newDownloadId // hand off to local state for LaunchedEffect
    }

    // Browser theme seed — updated by BrowserScreen when user picks a color
    var browserThemeSeed by remember { mutableLongStateOf(0xFF4F8EF7L) }
    val seedColor = remember(browserThemeSeed) { Color(browserThemeSeed) }

    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()

    val repo = BridgeInitializer.repositoryOrNull()
    val bridgeAvailable = repo != null
    val repository = repo ?: NoOpDownloadRepository()
    val vmFactory = remember(repository, bridgeAvailable) {
        DownloadsViewModel.Factory(repository, bridgeAvailable)
    }

    // If app was launched with a magnet: URI, submit it as a download once bridge is ready.
    val magnetToSubmit = pendingMagnet
    if (magnetToSubmit != null && bridgeAvailable) {
        pendingMagnet = null
        coroutineScope.launch {
            try {
                (repository as? dev.fetchx.core.download.TorrentDownloadRepository)
                    ?.addMagnet(magnetToSubmit, DownloadPriority.DEFAULT)
                Toast.makeText(context, "Magnet download added", Toast.LENGTH_SHORT).show()
                navController.navigate(Screen.Downloads.route) { launchSingleTop = true }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (e: Exception) {
                Toast.makeText(context, e.message ?: "Could not add magnet download", Toast.LENGTH_LONG).show()
            }
        }
    }

    val browserVersionName = remember(context) {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "unknown"
    }
    val downloadsDir = BridgeInitializer.publicDownloadStorageOrNull()?.defaultDirectoryPath()
        ?: java.io.File(
            android.os.Environment.getExternalStoragePublicDirectory(
                android.os.Environment.DIRECTORY_DOWNLOADS,
            ),
            "FetchX",
        ).absolutePath

    // Handle notification deep-link: navigate to Downloads, then to TorrentDetailScreen
    // for torrent downloads (engine == "native_torrent" or "aria2").
    val openDownloadId = pendingOpenDownloadId
    if (openDownloadId != null) {
        LaunchedEffect(openDownloadId) {
            pendingOpenDownloadId = null     // consume before any suspension
            navController.navigate(Screen.Downloads.route) { launchSingleTop = true }
            // Wait for the bridge/repository to be ready before resolving the
            // download. On a cold start the bridge is not yet initialized, so
            // observeDownloads() would emit an empty snapshot and the torrent
            // engine check would fail, leaving the user on Downloads. Reuse the
            // same bounded-readiness wait as the magnet ACTION_VIEW path.
            val readyRepo = awaitRepositoryReady(repositoryProvider = { BridgeInitializer.repositoryOrNull() })
                ?: return@LaunchedEffect  // bridge never came up — stay on Downloads
            val item = readyRepo.getDownload(openDownloadId)
            val engine = item?.engine ?: ""
            val isTorrent = engine.equals("native_torrent", ignoreCase = true) ||
                engine.equals("aria2", ignoreCase = true)
            if (isTorrent) {
                navController.navigate(Screen.TorrentDetail.route(openDownloadId))
            }
        }
    }

    Scaffold(
        bottomBar = {
            if (currentRoute != Screen.Browser.route) {
                NavigationBar {
                    NavigationBarItem(
                        selected = currentRoute == Screen.Browser.route,
                        onClick = {
                            navigateToBrowser(navController)
                        },
                        icon = { Icon(Icons.Default.Language, contentDescription = "Browser") },
                        label = { Text("Browser") },
                    )
                    NavigationBarItem(
                        selected = currentRoute == Screen.Downloads.route,
                        onClick = {
                            navController.navigate(Screen.Downloads.route) {
                                launchSingleTop = true
                            }
                        },
                        icon = { Icon(Icons.Default.FileDownload, contentDescription = "Downloads") },
                        label = { Text("Downloads") },
                    )
                }
            }
        },
    ) { innerPadding ->
        NavHost(
            navController = navController,
            startDestination = Screen.Browser.route,
            modifier = Modifier.padding(innerPadding),
        ) {
            composable(Screen.Downloads.route) {
                DownloadsScreen(
                    viewModel = viewModel(factory = vmFactory),
                    downloadsDir = downloadsDir,
                    onOpenTorrentDetail = { id ->
                        navController.navigate(Screen.TorrentDetail.route(id))
                    },
                    onPlayFile = { path ->
                        val uri = if (path.startsWith("content://")) {
                            Uri.parse(path)
                        } else {
                            val file = java.io.File(path)
                            try {
                                androidx.core.content.FileProvider.getUriForFile(
                                    context,
                                    "${context.packageName}.fileprovider",
                                    file,
                                )
                            } catch (_: Exception) {
                                null
                            }
                        }
                        if (uri == null) {
                            android.widget.Toast.makeText(
                                context,
                                "Unable to open file: path is not accessible",
                                android.widget.Toast.LENGTH_LONG,
                            ).show()
                        } else {
                            val intent = android.content.Intent(android.content.Intent.ACTION_VIEW).apply {
                                setDataAndType(uri, "video/*")
                                addFlags(android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION)
                                addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
                            }
                            try {
                                context.startActivity(
                                    android.content.Intent.createChooser(intent, "Play with...")
                                        .addFlags(android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION),
                                )
                            } catch (_: ActivityNotFoundException) {
                                Toast.makeText(context, "No video player is installed", Toast.LENGTH_LONG).show()
                            }
                        }
                    },
                )
            }
            composable(Screen.Browser.route) {
                FetchXTheme(seedColor = seedColor) {
                    BrowserScreen(
                        homeRequest = homeRequest,
                        searchRequest = searchRequest,
                        tabsRequest = tabsRequest,
                        onHomeRequestHandled = { homeRequest = 0L },
                        onSearchRequestHandled = { searchRequest = 0L },
                        onTabsRequestHandled = { tabsRequest = 0L },
                        showBrowserMenu = showBrowserMenu,
                        onBrowserMenuDismiss = { showBrowserMenu = false },
                        onOpenBrowserMenu = { showBrowserMenu = true },
                        onOpenStorage = {
                            navController.navigate(Screen.Storage.route) {
                                launchSingleTop = true
                            }
                        },
                        onNavigateToDownloads = {
                            navController.navigate(Screen.Downloads.route) {
                                launchSingleTop = true
                            }
                        },
                        browserVersionName = browserVersionName,
                        initialNavigateUrl = pendingUrl.also { pendingUrl = null },
                        onDownloadCurrentPage = { url, title ->
                            if (!bridgeAvailable) {
                                Toast.makeText(context, "Download service is unavailable", Toast.LENGTH_LONG).show()
                            } else {
                                coroutineScope.launch {
                                    try {
                                        repository.addMediaDownload(
                                            url = canonicalizeMediaUrl(url),
                                            fileName = title.ifBlank { "Media download" },
                                            destinationPath = mediaDownloadDestination(
                                                downloadsDir,
                                                title,
                                                context,
                                                repo.reservedPublicFileNames(),
                                            ),
                                            priority = DownloadPriority.DEFAULT,
                                        )
                                        Toast.makeText(context, "Media download added", Toast.LENGTH_SHORT).show()
                                        navController.navigate(Screen.Downloads.route) {
                                            launchSingleTop = true
                                        }
                                    } catch (cancelled: CancellationException) {
                                        throw cancelled
                                    } catch (error: Exception) {
                                        Toast.makeText(
                                            context,
                                            error.message ?: "Could not add media download",
                                            Toast.LENGTH_LONG,
                                        ).show()
                                    }
                                }
                            }
                        },
                        onInterceptBrowserDownload = { url, filename ->
                            val browserRepo = repo
                            if (browserRepo == null) {
                                Toast.makeText(
                                    context,
                                    "FetchX is unavailable; this browser response cannot be resumed.",
                                    Toast.LENGTH_LONG,
                                ).show()
                            } else {
                                coroutineScope.launch {
                                    try {
                                        browserRepo.addBrowserDownload(url, filename)
                                        Toast.makeText(
                                            context,
                                            "Download added to FetchX",
                                            Toast.LENGTH_SHORT,
                                        ).show()
                                        navController.navigate(Screen.Downloads.route) {
                                            launchSingleTop = true
                                        }
                                    } catch (cancelled: CancellationException) {
                                        throw cancelled
                                    } catch (error: Exception) {
                                        Toast.makeText(
                                            context,
                                            error.message ?: "Could not add browser download",
                                            Toast.LENGTH_LONG,
                                        ).show()
                                    }
                                }
                            }
                        },
                        onPlayVideo = { url ->
                            val scheme = try {
                                android.net.Uri.parse(url).scheme?.lowercase()
                            } catch (_: Exception) {
                                null
                            }
                            if (scheme == "http" || scheme == "https") {
                                val intent = android.content.Intent(android.content.Intent.ACTION_VIEW).apply {
                                    setDataAndType(android.net.Uri.parse(url), "video/*")
                                    addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
                                }
                                try {
                                    context.startActivity(intent)
                                } catch (_: ActivityNotFoundException) {
                                    Toast.makeText(context, "No video player is installed", Toast.LENGTH_LONG).show()
                                }
                            } else {
                                android.widget.Toast.makeText(
                                    context,
                                    "This URL cannot be opened in a video player",
                                    android.widget.Toast.LENGTH_LONG,
                                ).show()
                            }
                        },
                        onThemeColorChange = { newSeed -> browserThemeSeed = newSeed },
                    )
                }
            }
            composable(Screen.Storage.route) {
                StorageScreen(onBack = { navController.popBackStack() })
            }
            composable(Screen.TorrentDetail.ROUTE_PATTERN) { backStackEntry ->
                val downloadId = backStackEntry.arguments?.getString("downloadId") ?: ""
                TorrentDetailScreen(
                    downloadId = downloadId,
                    viewModel = viewModel(factory = vmFactory),
                    onBack = { navController.popBackStack() },
                )
            }
        }
    }
}

internal fun sanitizeMediaFileName(title: String): String =
    title.ifBlank { "Media download" }
        // Strip characters that are illegal in file names or enable path traversal.
        .replace(Regex("[/\\\\:*?\"<>|]"), "_")
        .trimStart('.')
        .ifBlank { "Media download" }
        .let { if (it.endsWith(".mp4", ignoreCase = true)) it else "$it.mp4" }

internal fun mediaDownloadDestination(
    downloadsDir: String,
    title: String,
    context: android.content.Context,
    reservedFileNames: Set<String> = emptySet(),
): String {
    val directory = File(downloadsDir)
    val baseName = sanitizeMediaFileName(title)
    val safeName = BridgeInitializer.publicDownloadStorageOrNull()
        ?.uniquePublicFileName(baseName, reservedFileNames = reservedFileNames)
        ?: StorageManager(context).uniqueFileName(directory, baseName, reservedFileNames)
    return File(directory, safeName).absolutePath
}

internal fun canonicalizeMediaUrl(url: String): String {
    val uri = try {
        java.net.URI(url)
    } catch (_: Exception) {
        return url
    }
    val host = uri.host ?: return url
    if (!host.equals("m.youtube.com", ignoreCase = true)) return url

    if (uri.path == "/watch") {
        val query = uri.query ?: return url
        val videoId = query.split('&')
            .map { it.split('=', limit = 2) }
            .firstOrNull { it.first() == "v" && it.size == 2 }
            ?.get(1) ?: return url
        return "https://www.youtube.com/watch?v=$videoId"
    }
    return java.net.URI(
        uri.scheme ?: "https",
        null, // Do not forward any embedded credentials from the source URI.
        "www.youtube.com",
        uri.port,
        uri.path,
        uri.query,
        uri.fragment,
    ).toASCIIString()
}

private fun navigateToBrowser(navController: NavHostController) {
    navController.navigate(Screen.Browser.route) {
        popUpTo(Screen.Browser.route) {
            inclusive = false
        }
        launchSingleTop = true
    }
}
