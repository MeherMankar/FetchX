package dev.fetchx.downloads.ui

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertNotNull

/**
 * Unit tests for UI utility functions that form a security boundary between
 * internal Rust/JNA error strings and user-visible UI text.
 */
class UtilsTest {

    // ── sanitizeFailureReason ──────────────────────────────────────────────

    @Test
    fun sanitize_blank_returns_unknown_error() {
        assertEquals("Unknown error", sanitizeFailureReason(""))
        assertEquals("Unknown error", sanitizeFailureReason("   "))
    }

    @Test
    fun sanitize_java_exception_class_returns_generic() {
        assertEquals(
            "Download failed. Please try again.",
            sanitizeFailureReason("java.net.SocketTimeoutException: connect timed out")
        )
    }

    @Test
    fun sanitize_jni_fragment_returns_generic() {
        assertEquals(
            "Download failed. Please try again.",
            sanitizeFailureReason("jni: failed to attach thread")
        )
    }

    @Test
    fun sanitize_jna_fragment_returns_generic() {
        assertEquals(
            "Download failed. Please try again.",
            sanitizeFailureReason("JNA dispatch error in native call")
        )
    }

    @Test
    fun sanitize_panic_string_returns_generic() {
        assertEquals(
            "Download failed. Please try again.",
            sanitizeFailureReason("panicked at 'invalid state', crates/core/src/lib.rs:42")
        )
    }

    @Test
    fun sanitize_uniffi_fragment_returns_generic() {
        assertEquals(
            "Download failed. Please try again.",
            sanitizeFailureReason("uniffi: callback interface error")
        )
    }

    @Test
    fun sanitize_network_error_returns_friendly_network_message() {
        assertEquals(
            "Network error. Check your connection and retry.",
            sanitizeFailureReason("network connection refused")
        )
        assertEquals(
            "Network error. Check your connection and retry.",
            sanitizeFailureReason("timeout waiting for server response")
        )
        assertEquals(
            "Network error. Check your connection and retry.",
            sanitizeFailureReason("connection reset by peer")
        )
    }

    @Test
    fun sanitize_storage_error_returns_friendly_storage_message() {
        assertEquals(
            "Storage error. Check available space and retry.",
            sanitizeFailureReason("storage full")
        )
        assertEquals(
            "Storage error. Check available space and retry.",
            sanitizeFailureReason("disk write error")
        )
    }

    @Test
    fun sanitize_404_returns_not_found_message() {
        assertEquals(
            "File not found on the server.",
            sanitizeFailureReason("404 not found")
        )
        assertEquals(
            "File not found on the server.",
            sanitizeFailureReason("resource not found on server")
        )
    }

    @Test
    fun sanitize_403_returns_access_denied_message() {
        assertEquals(
            "Access denied by the server.",
            sanitizeFailureReason("403 forbidden")
        )
        assertEquals(
            "Access denied by the server.",
            sanitizeFailureReason("unauthorized access")
        )
    }

    @Test
    fun sanitize_url_error_returns_url_message() {
        assertEquals(
            "Invalid download URL.",
            sanitizeFailureReason("invalid url provided")
        )
    }

    @Test
    fun sanitize_cancelled_returns_cancelled_message() {
        assertEquals(
            "Download was cancelled.",
            sanitizeFailureReason("download was cancelled by user")
        )
    }

    @Test
    fun sanitize_snake_case_internal_token_returns_generic() {
        // Strings like "invalid_state_transition" contain snake_case identifiers
        // characteristic of Rust internals and must NOT be shown directly.
        assertEquals(
            "Download failed. Please try again.",
            sanitizeFailureReason("invalid_state_transition")
        )
        assertEquals(
            "Download failed. Please try again.",
            sanitizeFailureReason("state_machine: error")
        )
    }

    @Test
    fun sanitize_colon_separator_returns_generic() {
        // Strings with ":" are characteristic of structured internal messages.
        assertEquals(
            "Download failed. Please try again.",
            sanitizeFailureReason("engine: aria2 failed")
        )
    }

    @Test
    fun sanitize_short_clean_string_passes_through() {
        // A short, plain string with no internal markers should pass through verbatim.
        val reason = "Server unavailable"
        assertEquals(reason, sanitizeFailureReason(reason))
    }

    @Test
    fun sanitize_long_clean_string_returns_generic() {
        // Even a clean-looking string that is too long (> 80 chars) falls back to generic.
        val longReason = "A".repeat(81)
        assertEquals("Download failed. Please try again.", sanitizeFailureReason(longReason))
    }

    // ── validateUrl ────────────────────────────────────────────────────────

    @Test
    fun validateUrl_blank_returns_error() {
        assertNotNull(validateUrl(""))
        assertNotNull(validateUrl("   "))
    }

    @Test
    fun validateUrl_http_passes() {
        assertNull(validateUrl("http://example.com/file.zip"))
    }

    @Test
    fun validateUrl_https_passes() {
        assertNull(validateUrl("https://example.com/file.zip"))
    }

    @Test
    fun validateUrl_magnet_passes_to_rust_core_validation() {
        assertNull(
            validateUrl(
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
            ),
        )
    }

    @Test
    fun validateUrl_local_torrent_path_passes_to_rust_core_validation() {
        assertNull(validateUrl("C:\\FetchX\\legal-fixture.torrent"))
        assertNotNull(validateUrl("ftp://example.com/file.torrent"))
    }

    @Test
    fun validateUrl_missing_scheme_returns_error() {
        assertNotNull(validateUrl("example.com/file.zip"))
        assertNotNull(validateUrl("ftp://example.com/file.zip"))
        assertNotNull(validateUrl("//example.com/file.zip"))
    }

    @Test
    fun validateUrl_error_message_is_user_readable() {
        val error = validateUrl("example.com/file.zip")
        assertNotNull(error)
        // Must not contain internal class names, stack trace fragments, or identifiers.
        assert(!error.contains("Exception")) { "Error must not mention exceptions: $error" }
        assert(!error.contains("null")) { "Error must not contain 'null': $error" }
    }
}
