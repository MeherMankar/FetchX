package dev.fetchx.browser

import java.net.URI
import java.net.URISyntaxException
import java.util.Locale
import java.util.UUID

/**
 * A frequently visited website shown on the new-tab page.
 */
data class Essential(
    val id: String = UUID.randomUUID().toString(),
    val title: String,
    val url: String,
    val initial: String,
    val colorHex: String,
)

/** Builds new-tab shortcuts from successful browser history. */
class EssentialsStore {

    companion object {
        const val MAX_ITEMS = 12

        private val colors = listOf(
            "#4285F4", "#7C6AF2", "#3AAE7A", "#FF4500", "#DE5833", "#1DA1F2",
        )
    }

    fun mostVisited(history: List<HistoryEntry>): List<Essential> {
        val visits = history.mapNotNull { entry ->
            val uri = try {
                URI(entry.url)
            } catch (_: URISyntaxException) {
                return@mapNotNull null
            }
            val host = uri.host?.lowercase(Locale.ROOT)?.removePrefix("www.")
                ?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
            if (uri.scheme !in listOf("http", "https")) return@mapNotNull null

            val port = if (uri.port >= 0) ":${uri.port}" else ""
            Visit(host + port, host, uri, entry)
        }

        return visits.groupBy { it.key }
            .values
            .sortedWith(
                compareByDescending<List<Visit>> { it.size }
                    .thenByDescending { group -> group.maxOf { it.entry.timestampMs } },
            )
            .take(MAX_ITEMS)
            .map { group ->
                val mostRecent = group.maxBy { it.entry.timestampMs }
                val displayHost = mostRecent.host
                val displayTitle = mostRecent.entry.title?.takeIf { it.isNotBlank() } ?: displayHost
                val authority = mostRecent.uri.rawAuthority ?: displayHost
                val url = "${mostRecent.uri.scheme}://$authority"
                Essential(
                    title = displayTitle,
                    url = url,
                    initial = displayTitle.first().uppercase(),
                    colorHex = colors[(displayHost.hashCode() and Int.MAX_VALUE) % colors.size],
                )
            }
    }

    private data class Visit(
        val key: String,
        val host: String,
        val uri: URI,
        val entry: HistoryEntry,
    )
}
