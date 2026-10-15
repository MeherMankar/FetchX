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

    fun isVideoUrl(url: String): Boolean {
        val lower = url.lowercase(Locale.ROOT)
        val ext = lower.substringBefore("?").substringAfterLast(".", "")
        if (ext in VIDEO_EXTENSIONS) return true
        return VIDEO_PATH_HINTS.any { lower.contains(it) }
    }

    fun isVideoMimeType(mimeType: String?): Boolean {
        if (mimeType == null) return false
        val lower = mimeType.lowercase(Locale.ROOT).substringBefore(";").trim()
        return VIDEO_MIME_PREFIXES.any { lower.startsWith(it) }
    }

    fun suggestFilename(url: String, mimeType: String?): String {
        val raw = url.substringBefore("?").substringAfterLast("/").trim()
        val base = raw.takeIf { it.isNotBlank() } ?: "video"
        val hasVideoExt = base.substringAfterLast(".").lowercase() in VIDEO_EXTENSIONS
        return if (hasVideoExt) base else "$base.mp4"
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
