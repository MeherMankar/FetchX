package dev.fetchx.browser

import java.util.Locale

internal data class BrowserDownloadMetadata(
    val mimeType: String?,
    val contentLength: Long?,
)

internal fun browserDownloadMetadata(headers: Map<String, String>): BrowserDownloadMetadata {
    val mimeType = headers.entries
        .firstOrNull { (name, _) -> name.equals("content-type", ignoreCase = true) }
        ?.value
        ?.substringBefore(';')
        ?.trim()
        ?.takeIf(String::isNotBlank)
    val contentLength = headers.entries
        .firstOrNull { (name, _) -> name.equals("content-length", ignoreCase = true) }
        ?.value
        ?.toLongOrNull()
        ?.takeIf { it >= 0L }
    return BrowserDownloadMetadata(mimeType, contentLength)
}

internal fun browserDownloadMetadataLabel(
    filename: String?,
    mimeType: String?,
    contentLength: Long?,
): String? {
    val type = mimeType
        ?.takeIf { '/' in it }
        ?: filename
            ?.substringAfterLast('.', "")
            ?.takeIf(String::isNotBlank)
            ?.uppercase(Locale.ROOT)
    val size = contentLength?.takeIf { it >= 0L }?.let(::formatDownloadSize)
    val details = listOfNotNull(type, size)
    return details.takeIf { it.isNotEmpty() }?.joinToString(" · ")
}

internal fun formatDownloadSize(bytes: Long): String {
    if (bytes < 1024L) return "$bytes B"
    val units = listOf("KB", "MB", "GB", "TB")
    var value = bytes.toDouble()
    var unitIndex = -1
    do {
        value /= 1024.0
        unitIndex += 1
    } while (value >= 1024.0 && unitIndex < units.lastIndex)
    return String.format(Locale.ROOT, "%.1f %s", value, units[unitIndex])
}
