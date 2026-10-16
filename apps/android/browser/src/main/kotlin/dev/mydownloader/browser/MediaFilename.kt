package dev.fetchx.browser

private val UNSAFE_FILENAME_CHARACTERS = Regex("[/\\\\:*?\"<>|\\p{Cntrl}]")
private val WHITESPACE = Regex("\\s+")
private val REPEATED_PERIODS = Regex("\\.{2,}")
private val MEDIA_EXTENSION = Regex("\\.(m3u8|mpd|mp4|m4v|mkv|webm|mov|flv|avi|3gp|wmv)$", RegexOption.IGNORE_CASE)

internal fun buildMediaFileName(
    title: String,
    quality: String?,
    platform: String?,
    extension: String?,
): String {
    val safeTitle = sanitizeFilenamePart(title)
        .replace(MEDIA_EXTENSION, "")
        .take(120)
        .ifBlank { "video" }
    val safeQuality = sanitizeFilenamePart(
        quality?.substringBefore(" + audio")?.substringBefore(" - ") ?: "Best",
    ).take(32).ifBlank { "Best" }
    val safePlatform = sanitizeFilenamePart(platform.orEmpty())
        .take(40)
        .ifBlank { "Web" }
    val safeExtension = extension
        ?.takeIf { it.length in 1..10 && it.all(Char::isLetterOrDigit) }
        ?: "mp4"

    return "$safeTitle - $safeQuality - $safePlatform.$safeExtension"
}

private fun sanitizeFilenamePart(value: String): String =
    value.replace(UNSAFE_FILENAME_CHARACTERS, " ")
        .replace(WHITESPACE, " ")
        .replace(REPEATED_PERIODS, ".")
        .trim()
        .trim('.')
        .trim()
