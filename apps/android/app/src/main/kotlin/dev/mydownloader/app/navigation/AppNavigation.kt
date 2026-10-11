package dev.mydownloader.app.navigation

import androidx.compose.runtime.Composable
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import dev.mydownloader.downloads.ui.DownloadsScreen

/**
 * Root NavHost for the MyDownloader application.
 * Additional destinations will be registered here as sub-phases are implemented.
 */
@Composable
fun AppNavigation() {
    val navController = rememberNavController()

    NavHost(
        navController = navController,
        startDestination = Screen.Downloads.route
    ) {
        composable(Screen.Downloads.route) {
            DownloadsScreen()
        }
    }
}
