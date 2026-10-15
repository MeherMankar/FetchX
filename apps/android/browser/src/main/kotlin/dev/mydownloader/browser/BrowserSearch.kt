package dev.fetchx.browser

import java.net.IDN
import java.net.URI
import java.net.URLEncoder
import java.util.Locale

sealed interface SearchIntent {
    data object Empty : SearchIntent
    data class NavigateUrl(val url: String) : SearchIntent
    data class LocalSearch(val query: String) : SearchIntent
    data class WebSearch(val query: String, val url: String) : SearchIntent
}

enum class BrowserSearchResultType(val label: String) {
    Bookmark("Bookmark"),
    History("History"),
    Tab("Open tab"),
}

data class BrowserSearchResult(
    val id: String,
    val type: BrowserSearchResultType,
    val title: String,
    val url: String,
    val timestampMs: Long?,
    val tabId: String? = null,
)

interface WebSearchProvider {
    fun searchUrl(query: String): String
}

class UrlTemplateSearchProvider(private val template: String) : WebSearchProvider {
    init {
        require(template.count { it == '%' } == 1 && template.contains("%s")) {
            "Search URL must contain exactly one %s placeholder"
        }
        val parts = template.split("%s")
        val prefix = parts[0]
        val suffix = parts[1]
        val prefixUri = URI(prefix)
        require(
            prefixUri.scheme.equals("https", ignoreCase = true) &&
                !prefixUri.host.isNullOrBlank() &&
                prefixUri.rawUserInfo == null &&
                !template.contains('#'),
        ) {
            "Search URL must be an HTTPS URL with a host and no credentials or fragment"
        }
        require(!suffix.contains("%s")) {
            "Search URL must contain exactly one %s placeholder"
        }
    }

    override fun searchUrl(query: String): String =
        template.replace("%s", URLEncoder.encode(query, Charsets.UTF_8.name()))
}

object SearchIntentClassifier {
    private val fileExtension =
        Regex("""\.(?:mp4|mkv|webm|avi|mov|mp3|m4a|flac|wav|pdf|zip|rar|7z|apk|exe|torrent)$""", RegexOption.IGNORE_CASE)

    fun classify(
        input: String,
        localResults: List<BrowserSearchResult>,
        provider: WebSearchProvider,
    ): SearchIntent {
        val query = input.trim()
        if (query.isEmpty()) return SearchIntent.Empty

        val explicitHttpUrl = Regex("""^https?://""", RegexOption.IGNORE_CASE).containsMatchIn(query)
        if (explicitHttpUrl) {
            normalizeBrowserUrl(query)?.let { return SearchIntent.NavigateUrl(it) }
        } else if (fileExtension.containsMatchIn(query)) {
            return SearchIntent.LocalSearch(query)
        } else {
            normalizeBrowserUrl(query)?.let { return SearchIntent.NavigateUrl(it) }
        }
        if (fileExtension.containsMatchIn(query) || localResults.isNotEmpty()) {
            return SearchIntent.LocalSearch(query)
        }
        return SearchIntent.WebSearch(query, provider.searchUrl(query))
    }

    fun normalizeBrowserUrl(input: String): String? {
        val value = input.trim()
        if (value.isEmpty() || value.any(Char::isWhitespace)) return null
        val explicitScheme = Regex("""^[A-Za-z][A-Za-z0-9+.-]*:""").containsMatchIn(value)
        val explicitHttpUrl = Regex("""^https?://""", RegexOption.IGNORE_CASE).containsMatchIn(value)
        val hostWithPort = Regex("""^[^/?#]+:\d+(?:[/?#]|$)""").containsMatchIn(value)
        if (explicitScheme && !explicitHttpUrl && !hostWithPort) return null
        val candidate = if (explicitHttpUrl) {
            val scheme = value.substringBefore(':').lowercase(Locale.ROOT)
            if (scheme != "http" && scheme != "https") return null
            value
        } else {
            "https://$value"
        }
        return try {
            val parsed = URI(candidate).normalize()
            val scheme = parsed.scheme?.lowercase(Locale.ROOT) ?: return null
            val host = parsed.host ?: return null
            if (scheme !in setOf("http", "https") || parsed.rawUserInfo != null) return null
            if (!validHost(host) || parsed.port > 65535) return null
            parsed.toASCIIString()
        } catch (_: Exception) {
            null
        }
    }

    private fun validHost(rawHost: String): Boolean {
        val host = rawHost.removePrefix("[").removeSuffix("]").trimEnd('.')
        if (host.equals("localhost", ignoreCase = true)) return true
        if (host.contains(':')) {
            return host.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' || it == ':' || it == '.' }
        }
        if (host.matches(Regex("""\d{1,3}(?:\.\d{1,3}){3}"""))) {
            return host.split('.').all { octet ->
                octet.toIntOrNull()?.let { it in 0..255 } == true
            }
        }
        val ascii = try {
            IDN.toASCII(host)
        } catch (_: IllegalArgumentException) {
            return false
        }
        if (ascii.length > 253 || !ascii.contains('.')) return false
        val labels = ascii.split('.')
        if (labels.any { it.isEmpty() || it.length > 63 || it.startsWith('-') || it.endsWith('-') }) {
            return false
        }
        val topLevel = labels.last()
        return (topLevel.length >= 2 && topLevel.all(Char::isLetter)) ||
            topLevel.startsWith("xn--", ignoreCase = true)
    }
}

object BrowserLocalSearch {
    const val MAX_RESULTS = 20

    fun search(
        query: String,
        bookmarks: List<Bookmark>,
        history: List<HistoryEntry>,
        tabs: List<TabState>,
        limit: Int = MAX_RESULTS,
    ): List<BrowserSearchResult> {
        val normalized = query.trim().lowercase(Locale.ROOT)
        if (normalized.isEmpty() || limit <= 0) return emptyList()

        val candidates = buildList {
            bookmarks.forEach { bookmark ->
                if (matches(normalized, bookmark.title, bookmark.url)) {
                    add(
                        BrowserSearchResult(
                            id = bookmark.id,
                            type = BrowserSearchResultType.Bookmark,
                            title = bookmark.title,
                            url = bookmark.url,
                            timestampMs = bookmark.updatedAtMs,
                        ),
                    )
                }
            }
            history.forEach { entry ->
                val title = entry.title?.takeIf(String::isNotBlank) ?: hostLabel(entry.url)
                if (matches(normalized, title, entry.url)) {
                    add(
                        BrowserSearchResult(
                            id = entry.id,
                            type = BrowserSearchResultType.History,
                            title = title,
                            url = entry.url,
                            timestampMs = entry.timestampMs,
                        ),
                    )
                }
            }
            tabs.forEach { tab ->
                if (tab.url != "about:blank" && matches(normalized, tab.title.orEmpty(), tab.url)) {
                    add(
                        BrowserSearchResult(
                            id = tab.id,
                            type = BrowserSearchResultType.Tab,
                            title = tab.title?.takeIf(String::isNotBlank) ?: hostLabel(tab.url),
                            url = tab.url,
                            timestampMs = null,
                            tabId = tab.id,
                        ),
                    )
                }
            }
        }

        return candidates
            .distinctBy { "${it.type}:${it.id}" }
            .sortedWith(
                compareBy<BrowserSearchResult> { relevance(normalized, it.title, it.url) }
                    .thenByDescending { it.timestampMs ?: Long.MIN_VALUE },
            )
            .take(limit)
    }

    private fun matches(query: String, title: String, url: String): Boolean =
        title.contains(query, ignoreCase = true) || url.contains(query, ignoreCase = true)

    private fun relevance(query: String, title: String, url: String): Int = when {
        title.equals(query, ignoreCase = true) -> 0
        title.startsWith(query, ignoreCase = true) -> 1
        title.contains(query, ignoreCase = true) -> 2
        url.contains(query, ignoreCase = true) -> 3
        else -> 4
    }

    private fun hostLabel(url: String): String =
        try {
            URI(url).host?.removePrefix("www.") ?: url
        } catch (_: Exception) {
            url
        }
}
