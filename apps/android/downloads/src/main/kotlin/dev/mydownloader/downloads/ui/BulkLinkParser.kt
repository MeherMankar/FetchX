package dev.fetchx.downloads.ui

import dev.fetchx.bridge.BridgeInitializer
import java.net.URI

internal object BulkLinkParser {
    fun extract(text: String): List<String> {
        return BridgeInitializer.extractDownloadLinks(text).ifEmpty {
            val seen = linkedSetOf<String>()
            Regex("""https?://[^\s<>"']+""", RegexOption.IGNORE_CASE)
                .findAll(text.replace("&amp;", "&"))
                .forEach { match ->
                    val candidate = match.value.trimEnd('.', ',', ';', ':', '!', '?', ')', ']', '}')
                    val uri = runCatching { URI(candidate) }.getOrNull() ?: return@forEach
                    if (uri.scheme.equals("http", ignoreCase = true) ||
                        uri.scheme.equals("https", ignoreCase = true)
                    ) {
                        seen.add(candidate)
                    }
                }
            seen.toList()
        }
    }

    fun suggestedFileName(url: String): String {
        val sharedName = BridgeInitializer.suggestDownloadFilename(url)
        if (sharedName != "download") return sharedName
        val rawName = runCatching { URI(url).path.substringAfterLast('/') }
            .getOrElse { "" }
            .substringBefore('?')
            .ifBlank { "download" }
        return rawName
            .replace(Regex("[\\\\/:*?\"<>|\\u0000-\\u001F]"), "_")
            .trim('.', ' ')
            .ifBlank { "download" }
    }

    fun suggestedPackageName(url: String, filename: String = suggestedFileName(url)): String =
        BridgeInitializer.suggestDownloadPackageName(url, filename)
}
