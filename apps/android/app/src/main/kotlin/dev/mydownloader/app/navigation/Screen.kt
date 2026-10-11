package dev.mydownloader.app.navigation

/**
 * Sealed class representing all top-level navigation destinations.
 * Additional screens (Browser, Settings, etc.) will be added in later phases.
 */
sealed class Screen(val route: String) {
    /** Download manager — active, queued, completed, failed downloads. */
    data object Downloads : Screen("downloads")

    // Phase 20F: Browser screen
    // data object Browser : Screen("browser")

    // Phase 20I: History/Bookmarks screen
    // data object History : Screen("history")

    // Phase 20N: Settings screen
    // data object Settings : Screen("settings")
}
