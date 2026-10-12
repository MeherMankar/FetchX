package dev.mydownloader.app.navigation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
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
    // Capture the result once so bridgeAvailable and repository are always consistent.
    val repo = BridgeInitializer.repositoryOrNull()
    val bridgeAvailable = repo != null
    val repository = repo ?: NoOpDownloadRepository()
    // Wrap in remember{} so the factory is not reconstructed on every recomposition.
    // The factory depends only on repository and bridgeAvailable, both of which are
    // stable for the lifetime of this composable (bridge state never changes mid-session).
    val vmFactory = remember(repository, bridgeAvailable) {
        DownloadsViewModel.Factory(repository, bridgeAvailable)
    }

    // Resolve the downloads directory path inside the Composable using LocalContext
    // so the ViewModel never holds an Android Context.
    val context = LocalContext.current
    val downloadsDir: String = context.getExternalFilesDir(null)?.absolutePath
        ?: "${context.filesDir.absolutePath}/downloads"

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
    }
}
