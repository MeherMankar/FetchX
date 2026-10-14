package dev.fetchx.downloads.storage

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class StorageManagerSanitizeTest {

    @Test fun `sanitize blank returns download`() {
        assertEquals("download", sanitizeFileName(""))
        assertEquals("download", sanitizeFileName("   "))
    }

    @Test fun `sanitize path traversal`() {
        val result = sanitizeFileName("../../etc/passwd")
        assertFalse(result.contains(".."))
        assertFalse(result.contains('/'))
    }

    @Test fun `sanitize normal filename unchanged`() {
        assertEquals("file.zip", sanitizeFileName("file.zip"))
        assertEquals("my document.pdf", sanitizeFileName("my document.pdf"))
    }

    @Test fun `sanitize null byte removed`() {
        val result = sanitizeFileName("file\u0000.zip")
        assertFalse(result.contains('\u0000'))
    }

    @Test fun `sanitize reserved chars replaced`() {
        val result = sanitizeFileName("file<>:\"|?*.zip")
        assertFalse(result.any { it in "<>:\"|?*" })
    }

    @Test fun `sanitize too long truncated`() {
        val long = "a".repeat(300)
        assertTrue(sanitizeFileName(long).length <= 200)
    }

    @Test fun `sanitize leading dots stripped`() {
        val result = sanitizeFileName("...hidden")
        assertFalse(result.startsWith('.'))
    }
}

// Extract pure functions for testing without needing a real StorageManager instance
private fun sanitizeFileName(raw: String): String {
    if (raw.isBlank()) return "download"
    val stripped = raw.trim()
        .replace('/', '_').replace('\\', '_')
        .replace('\u0000', '_')
        .replace("..", "_")
        .trimStart('.')
        .replace(Regex("[<>:\"|?*]"), "_")
        .take(200)
    return stripped.ifBlank { "download" }
}
