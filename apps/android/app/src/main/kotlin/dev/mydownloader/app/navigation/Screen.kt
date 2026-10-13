package dev.fetchx.app.navigation

/**
 * Sealed class representing all top-level navigation destinations.
 */
sealed class Screen(val route: String) {
    /** Download manager — active, queued, completed, failed downloads. */
    data object Downloads : Screen("downloads")

    /** Browser — GeckoView-based web browser (Phase 20E). */
    data object Browser : Screen("browser")

    // Phase 20I: History/Bookmarks screen
    // data object History : Screen("history")

    // Phase 20N: Settings screen
    // data object Settings : Screen("settings")
}

