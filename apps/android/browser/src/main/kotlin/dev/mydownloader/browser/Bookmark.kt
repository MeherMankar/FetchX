package dev.fetchx.browser

/**
 * A user-saved browser bookmark.
 *
 * Design constraints:
 * - Zero Android imports (JVM-testable).
 * - [id] is stable for the lifetime of this bookmark.
 * - [url] is always a non-blank http(s) URL.
 * - [title] has a fallback to the URL hostname if not provided.
 * - Does NOT store: cookies, credentials, session tokens, private browsing metadata.
 * - [createdAtMs] and [updatedAtMs] are epoch-milliseconds.
 */
data class Bookmark(
    val id: String,          // UUID stable identifier
    val url: String,
    val title: String,
    val createdAtMs: Long,
    val updatedAtMs: Long,
)
