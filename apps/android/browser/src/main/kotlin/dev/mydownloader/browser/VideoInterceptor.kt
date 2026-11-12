package dev.fetchx.browser

import java.util.Locale

/**
 * Stateless helper that decides whether a network response looks like
 * a downloadable video and suggests a filename for it.
 *
 * No network calls, no Android context required.
 */
object VideoInterceptor {

    val VIDEO_EXTENSIONS = setOf(
        "mp4", "m4v", "mkv", "webm", "avi", "mov", "flv",
        "ts", "m3u8", "mpd", "3gp", "wmv",
    )

    val VIDEO_MIME_PREFIXES = listOf("video/")

    /** Path segments or query keys whose presence suggests a media stream. */
    val VIDEO_PATH_HINTS = listOf(
        "/videoplayback", "/manifest", ".m3u8", ".mpd",
        "googlevideo.com", "fbcdn.net", "vimeocdn.com",
    )

    /**
     * Returns true when the URL looks like an individual HLS/DASH segment rather
     * than a playlist or a full video file. Noisy segments produce high-frequency
     * callbacks and should be suppressed at the [detect] entry point.
     */
    fun isNoisySegment(url: String): Boolean {
        val path = url.lowercase(Locale.ROOT).substringBefore("?")
        // .ts or .m4s file extension
        if (path.endsWith(".ts") || path.endsWith(".m4s")) return true
        // Common segment path patterns (case-insensitive)
        val segmentPattern = Regex("""/seg-\d+|/chunk-\d+|/fragment\d+""", RegexOption.IGNORE_CASE)
        if (segmentPattern.containsMatchIn(url)) return true
        // /hls/ path with a purely numeric (or numeric.ts) last path segment
        if (url.contains("/hls/", ignoreCase = true)) {
            val lastSegment = path.substringAfterLast("/")
            val numericSegment = Regex("""^\d+(\.ts)?$""")
            if (numericSegment.matches(lastSegment)) return true
        }
        return false
    }

    /**
     * Returns true for HLS playlists (.m3u8) and DASH manifests (.mpd).
     */
    fun isHlsPlaylist(url: String): Boolean {
        val path = url.lowercase(Locale.ROOT).substringBefore("?")
        return path.endsWith(".m3u8") || path.endsWith(".mpd")
    }

    fun isVideoUrl(url: String): Boolean {
        val lower = url.lowercase(Locale.ROOT)
        val path = lower.substringBefore("?").substringBefore("#").trimEnd('/')
        val ext = path.substringAfterLast(".", "")
        if (ext in VIDEO_EXTENSIONS) return true
        return VIDEO_PATH_HINTS.any { lower.contains(it) }
    }

    fun isVideoMimeType(mimeType: String?): Boolean {
        if (mimeType == null) return false
        val lower = mimeType.lowercase(Locale.ROOT).substringBefore(";").trim()
        return VIDEO_MIME_PREFIXES.any { lower.startsWith(it) }
    }

    fun suggestFilename(url: String, mimeType: String?): String {
        val path = url.substringBefore("?").substringBefore("#").trimEnd('/')
        val raw = path.substringAfterLast("/").trim()
        val base = raw.takeIf { it.isNotBlank() } ?: "video"
        val hasVideoExt = base.substringAfterLast(".").lowercase() in VIDEO_EXTENSIONS
        val extension = base.substringAfterLast(".").lowercase()
        val quality = Regex("""\d{3,4}p""", RegexOption.IGNORE_CASE)
            .find(path)
            ?.value
            ?.lowercase(Locale.ROOT)
        return when {
            hasVideoExt && quality != null -> "$quality.$extension"
            hasVideoExt -> base
            else -> "$base.mp4"
        }
    }

    /**
     * Returns a [DetectedVideo] if the response looks like a video, null otherwise.
     * Decision: MIME type wins when present (most reliable); URL heuristics are
     * the fallback for streams where Content-Type is absent or generic.
     */
    fun detect(
        tabId: String,
        url: String,
        mimeType: String?,
        contentLength: Long?,
    ): DetectedVideo? {
        if (isNoisySegment(url)) return null
        val isMime = isVideoMimeType(mimeType)
        val isUrl = isVideoUrl(url)
        if (!isMime && !isUrl) return null
        return DetectedVideo(
            tabId = tabId,
            url = url,
            mimeType = mimeType,
            suggestedFilename = suggestFilename(url, mimeType),
            contentLength = contentLength,
            detectedAt = System.currentTimeMillis(),
        )
    }
}

data class DetectedVideo(
    val tabId: String,
    val url: String,
    val mimeType: String?,
    val suggestedFilename: String,
    val contentLength: Long?,
    val detectedAt: Long,
)
