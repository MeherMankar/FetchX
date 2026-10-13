package dev.mydownloader.app.navigation

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import dev.mydownloader.bridge.BridgeInitializer
import dev.mydownloader.browser.BrowserScreen
import dev.mydownloader.core.download.NoOpDownloadRepository
import dev.mydownloader.downloads.ui.DownloadsScreen
import dev.mydownloader.downloads.ui.DownloadsViewModel

/**
 * Root NavHost for the MyDownloader application.
 *
 * A [NavigationBar] at the bottom lets users switch between the Downloads
 * and Browser top-level destinations. The NavController is shared so
 * back-stack state is maintained.
 */
@Composable
fun AppNavigation() {
    val navController = rememberNavController()
    val navBackStackEntry by navController.currentBackStackEntryAsState()
    val currentRoute = navBackStackEntry?.destination?.route

    val repo = BridgeInitializer.repositoryOrNull()
    val bridgeAvailable = repo != null
    val repository = repo ?: NoOpDownloadRepository()
    val vmFactory = remember(repository, bridgeAvailable) {
        DownloadsViewModel.Factory(repository, bridgeAvailable)
    }

    val context = LocalContext.current
    val downloadsDir: String = context.getExternalFilesDir(null)?.absolutePath
        ?: "${context.filesDir.absolutePath}/downloads"

    Scaffold(
        bottomBar = {
            NavigationBar {
                NavigationBarItem(
                    selected = currentRoute == Screen.Downloads.route,
                    onClick = {
                        navController.navigate(Screen.Downloads.route) {
                            popUpTo(Screen.Downloads.route) { inclusive = true }
                            launchSingleTop = true
                        }
                    },
                    icon = { Icon(Icons.AutoMirrored.Filled.List, contentDescription = null) },
                    label = { Text("Downloads") },
                )
                NavigationBarItem(
                    selected = currentRoute == Screen.Browser.route,
                    onClick = {
                        navController.navigate(Screen.Browser.route) {
                            popUpTo(Screen.Downloads.route)
                            launchSingleTop = true
                        }
                    },
                    icon = { Icon(Icons.Default.Search, contentDescription = null) },
                    label = { Text("Browser") },
                )
            }
        },
    ) { _ ->
        NavHost(
            navController = navController,
            startDestination = Screen.Downloads.route,
        ) {
            composable(Screen.Downloads.route) {
                DownloadsScreen(
                    viewModel = viewModel(factory = vmFactory),
                    downloadsDir = downloadsDir,
                )
            }
            composable(Screen.Browser.route) {
                BrowserScreen(
                    onNavigateUp = {
                        navController.navigate(Screen.Downloads.route) {
                            popUpTo(Screen.Downloads.route) { inclusive = true }
                            launchSingleTop = true
                        }
                    },
                )
            }
        }
    }
}
