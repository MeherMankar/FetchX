package dev.fetchx.app.navigation

/**
 * Sealed class representing all top-level navigation destinations.
 */
sealed class Screen(val route: String) {
    /** Download manager — active, queued, completed, failed downloads. */
    data object Downloads : Screen("downloads")

    /** Browser — GeckoView-based web browser (Phase 20E). */
    data object Browser : Screen("browser")

    /** Read-only breakdown of FetchX-owned storage. */
    data object Storage : Screen("storage")

    /** Detail screen for a torrent download (file tree, telemetry, actions). */
    data class TorrentDetail(val downloadId: String) : Screen("torrent-detail/$downloadId") {
        companion object {
            const val ROUTE_PATTERN = "torrent-detail/{downloadId}"
            fun route(id: String) = "torrent-detail/$id"
        }
    }

    // Phase 20I: History/Bookmarks screen
    // data object History : Screen("history")

    // Phase 20N: Settings screen
    // data object Settings : Screen("settings")
}
