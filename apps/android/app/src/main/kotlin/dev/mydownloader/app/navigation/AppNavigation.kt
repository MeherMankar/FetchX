package dev.fetchx.app.navigation

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Tab
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.Modifier
import android.net.Uri
import android.widget.Toast
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
import dev.fetchx.core.download.DownloadPriority
import kotlinx.coroutines.launch
import java.io.File

/**
 * Root NavHost for the FetchX application.
 *
 * A [NavigationBar] at the bottom provides browser actions and access to
 * Downloads while preserving the shared navigation back stack.
 */
@Composable
fun AppNavigation() {
    val navController = rememberNavController()
    val navBackStackEntry by navController.currentBackStackEntryAsState()
    val currentRoute = navBackStackEntry?.destination?.route
    var selectedBrowserItem by remember { mutableStateOf("home") }
    var homeRequest by remember { mutableLongStateOf(0L) }
    var searchRequest by remember { mutableLongStateOf(0L) }
    var tabsRequest by remember { mutableLongStateOf(0L) }
    var showBrowserMenu by remember { mutableStateOf(false) }

    // Browser theme seed — updated by BrowserScreen when user picks a color
    var browserThemeSeed by remember { mutableLongStateOf(0xFF4F8EF7L) }
    val seedColor = remember(browserThemeSeed) { Color(browserThemeSeed) }

    val repo = BridgeInitializer.repositoryOrNull()
    val bridgeAvailable = repo != null
    val repository = repo ?: NoOpDownloadRepository()
    val vmFactory = remember(repository, bridgeAvailable) {
        DownloadsViewModel.Factory(repository, bridgeAvailable)
    }

    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()
    val browserVersionName = remember(context) {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "unknown"
    }
    val downloadsDir: String = context.getExternalFilesDir(null)?.absolutePath
        ?: "${context.filesDir.absolutePath}/downloads"

    Scaffold(
        bottomBar = {
            NavigationBar {
                NavigationBarItem(
                    selected = currentRoute == Screen.Browser.route && selectedBrowserItem == "home",
                    onClick = {
                        selectedBrowserItem = "home"
                        showBrowserMenu = false
                        navigateToBrowser(navController)
                        homeRequest++
                    },
                    icon = { Icon(Icons.Default.Home, contentDescription = null) },
                )
                NavigationBarItem(
                    selected = currentRoute == Screen.Downloads.route,
                    onClick = {
                        showBrowserMenu = false
                        navController.navigate(Screen.Downloads.route) {
                            launchSingleTop = true
                        }
                    },
                    icon = { Icon(Icons.Default.FileDownload, contentDescription = "Downloads") },
                )
                NavigationBarItem(
                    selected = currentRoute == Screen.Browser.route && selectedBrowserItem == "search",
                    onClick = {
                        selectedBrowserItem = "search"
                        showBrowserMenu = false
                        navigateToBrowser(navController)
                        searchRequest++
                    },
                    icon = { Icon(Icons.Default.Search, contentDescription = null) },
                )
                NavigationBarItem(
                    selected = currentRoute == Screen.Browser.route && selectedBrowserItem == "tabs",
                    onClick = {
                        selectedBrowserItem = "tabs"
                        showBrowserMenu = false
                        navigateToBrowser(navController)
                        tabsRequest++
                    },
                    icon = { Icon(Icons.Default.Tab, contentDescription = "Tabs") },
                )
                NavigationBarItem(
                    selected = currentRoute == Screen.Browser.route && selectedBrowserItem == "menu",
                    onClick = {
                        selectedBrowserItem = "menu"
                        navigateToBrowser(navController)
                        showBrowserMenu = true
                    },
                    icon = { Icon(Icons.Default.MoreVert, contentDescription = null) },
                )
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
                        browserVersionName = browserVersionName,
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
                                            ),
                                            priority = DownloadPriority.DEFAULT,
                                        )
                                        Toast.makeText(context, "Media download added", Toast.LENGTH_SHORT).show()
                                        navController.navigate(Screen.Downloads.route) {
                                            launchSingleTop = true
                                        }
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
                        onThemeColorChange = { newSeed -> browserThemeSeed = newSeed },
                    )
                }
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
): String {
    val directory = File(downloadsDir)
    val baseName = sanitizeMediaFileName(title)
    val safeName = StorageManager(context).uniqueFileName(directory, baseName)
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
