package dev.mydownloader.app.navigation

import androidx.compose.runtime.Composable
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import dev.mydownloader.bridge.BridgeInitializer
import dev.mydownloader.core.download.NoOpDownloadRepository
import dev.mydownloader.downloads.ui.DownloadsScreen
import dev.mydownloader.downloads.ui.DownloadsViewModel

/**
 * Root NavHost for the MyDownloader application.
 * Additional destinations will be registered here as sub-phases are implemented.
 */
@Composable
fun AppNavigation() {
    val navController = rememberNavController()

    // Resolve the repository from the bridge, or fall back to a no-op implementation
    // if the bridge failed to initialize (e.g. native library not yet present).
    val repository = BridgeInitializer.repositoryOrNull() ?: NoOpDownloadRepository()
    val vmFactory = DownloadsViewModel.Factory(repository)

    NavHost(
        navController = navController,
        startDestination = Screen.Downloads.route
    ) {
        composable(Screen.Downloads.route) {
            DownloadsScreen(viewModel = viewModel(factory = vmFactory))
        }
    }
}
